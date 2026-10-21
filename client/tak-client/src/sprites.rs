//! GAF/TAF sprite sequences as RGBA frames, and the billboard geometry that
//! shows them under the oblique projection. No Bevy types.
//!
//! - Files are parsed on first use and cached ([`SpriteStore`]).
//! - Palettes: a GAF's sibling `.pcx` when present (**Known** for 174
//!   `anims/` files, `gaf.md`); feature sprites otherwise use
//!   `palettes/<kingdom>_features.pcx` of the map's OTA `kingdom`
//!   (**Believed**: tree, rock and lodestone sprites show natural colours
//!   with it and noise with `<kingdom>.pcx`); else `palettes/palette.pal`.
//!   TAF frames are 16-bit and ignore the palette.
//! - A frame's `(x_offset, y_offset)` is the pixel that lands on the
//!   sprite's anchor (TA rule, `gaf.md`).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use tak_formats::gaf::palette::Palette;
use tak_formats::gaf::{Gaf, PixelFormat};
use tak_vfs::Vfs;

use crate::models::load_palette;
use crate::view::{SHEAR, ViewGeom};

/// One decoded frame.
#[derive(Clone, Debug)]
pub struct SpriteFrame {
    /// Width in pixels.
    pub w: u32,
    /// Height in pixels.
    pub h: u32,
    /// Anchor pixel inside the frame.
    pub x_off: i32,
    /// See `x_off`.
    pub y_off: i32,
    /// RGBA8 pixels.
    pub rgba: Vec<u8>,
    /// Second word of the frame reference (**Believed**, TA: ticks per
    /// frame).
    pub duration: u32,
    /// The frame's raw subframe word (`0xFF00` on some TAFs, meaning
    /// Unknown).
    pub subframe_word: u16,
    /// Stored as 16-bit ARGB (a TAF).
    pub direct: bool,
}

impl SpriteFrame {
    /// Pixels of the frame below its anchor (0 if none).
    pub fn below(&self) -> i32 {
        (self.h as i32 - self.y_off).max(0)
    }
}

/// A named sequence of frames.
#[derive(Clone, Debug, Default)]
pub struct SpriteSeq {
    /// Frames in order.
    pub frames: Vec<SpriteFrame>,
}

impl SpriteSeq {
    /// Largest number of pixels any frame reaches below its anchor.
    pub fn max_below(&self) -> i32 {
        self.frames
            .iter()
            .map(SpriteFrame::below)
            .max()
            .unwrap_or(0)
    }

    /// Frame to show at sim tick `tick` when animating (durations in ticks,
    /// at least 1 each).
    pub fn frame_at(&self, tick: u32) -> usize {
        let total: u32 = self.frames.iter().map(|f| f.duration.max(1)).sum();
        if total == 0 {
            return 0;
        }
        let mut t = tick % total;
        for (i, f) in self.frames.iter().enumerate() {
            let d = f.duration.max(1);
            if t < d {
                return i;
            }
            t -= d;
        }
        0
    }
}

/// Lazily parsed sprite files and palettes.
#[derive(Default)]
pub struct SpriteStore {
    gafs: RefCell<BTreeMap<String, Option<Rc<Gaf>>>>,
    palettes: RefCell<BTreeMap<String, Option<Palette>>>,
}

impl SpriteStore {
    /// The parsed file at `path`, cached (failures too).
    pub fn gaf(&self, vfs: &Vfs, path: &str) -> Option<Rc<Gaf>> {
        let key = path.to_ascii_lowercase();
        self.gafs
            .borrow_mut()
            .entry(key)
            .or_insert_with(|| {
                let bytes = vfs.read(path).ok()?;
                Gaf::parse(&bytes).ok().map(Rc::new)
            })
            .clone()
    }

    /// A palette by VFS path, cached.
    pub fn palette(&self, vfs: &Vfs, path: &str) -> Option<Palette> {
        let key = path.to_ascii_lowercase();
        self.palettes
            .borrow_mut()
            .entry(key)
            .or_insert_with(|| load_palette(vfs, path))
            .clone()
    }

    /// The palette for a feature sprite file on a map of `kingdom` (see the
    /// module docs).
    pub fn feature_palette(&self, vfs: &Vfs, path: &str, kingdom: Option<&str>) -> Palette {
        let sibling = match path.rsplit_once('.') {
            Some((stem, _)) => format!("{stem}.pcx"),
            None => format!("{path}.pcx"),
        };
        self.palette(vfs, &sibling)
            .or_else(|| {
                let k = kingdom?;
                self.palette(vfs, &format!("palettes/{k}_features.pcx"))
            })
            .or_else(|| self.palette(vfs, "palettes/palette.pal"))
            .unwrap_or_else(Palette::grayscale)
    }

    /// Decode sequence `name` of the file at `path` (case-insensitive).
    pub fn seq(&self, vfs: &Vfs, path: &str, name: &str, pal: &Palette) -> Option<SpriteSeq> {
        let gaf = self.gaf(vfs, path)?;
        let entry = gaf.entry(name)?;
        let frames = entry
            .frames
            .iter()
            .filter(|fr| fr.frame.width > 0 && fr.frame.height > 0)
            .map(|fr| {
                let f = &fr.frame;
                let img = f.to_rgba(pal);
                SpriteFrame {
                    w: img.width,
                    h: img.height,
                    x_off: i32::from(f.x_offset),
                    y_off: i32::from(f.y_offset),
                    rgba: img.data,
                    duration: fr.duration,
                    subframe_word: f.subframe_word,
                    direct: matches!(f.format, PixelFormat::Argb1555 | PixelFormat::Argb4444),
                }
            })
            .collect::<Vec<_>>();
        (!frames.is_empty()).then_some(SpriteSeq { frames })
    }
}

/// Height a sprite quad is lifted toward the viewer so that its lowest
/// pixel (`below` pixels under the anchor) stays above flat ground at the
/// anchor: screen-up moves `SHEAR / (1 + SHEAR²)` down per pixel, plus a
/// margin for slopes.
pub fn lift_for(below: i32, margin: f32) -> f32 {
    below.max(0) as f32 * SHEAR / (1.0 + SHEAR * SHEAR) + margin
}

/// A billboard quad for `frame`, in the anchor's local space: it lies in
/// the screen plane (one world unit per sprite pixel at zoom 1, as TA
/// draws sprites 1:1) with the anchor pixel at the origin, moved `lift`
/// height units toward the viewer along the view ray (which does not move
/// it on screen). Returns positions and UVs of the four corners
/// (top-left, top-right, bottom-right, bottom-left) and the two triangles.
pub fn billboard(frame: &SpriteFrame, lift: f32) -> ([[f32; 3]; 4], [[f32; 2]; 4], [u32; 6]) {
    let up = ViewGeom::screen_up();
    let l = [0.0, lift, lift * SHEAR];
    let at = |px: f32, py: f32| -> [f32; 3] {
        let dx = px - frame.x_off as f32;
        let dy = frame.y_off as f32 - py;
        [dx + l[0], up[1] * dy + l[1], up[2] * dy + l[2]]
    };
    let (w, h) = (frame.w as f32, frame.h as f32);
    (
        [at(0.0, 0.0), at(w, 0.0), at(w, h), at(0.0, h)],
        [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
        [0, 3, 2, 0, 2, 1],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: u32, h: u32, x: i32, y: i32, d: u32) -> SpriteFrame {
        SpriteFrame {
            w,
            h,
            x_off: x,
            y_off: y,
            rgba: vec![0; (w * h * 4) as usize],
            duration: d,
            subframe_word: 0,
            direct: false,
        }
    }

    #[test]
    fn frames_follow_durations() {
        let s = SpriteSeq {
            frames: vec![
                frame(1, 1, 0, 0, 2),
                frame(1, 1, 0, 0, 1),
                frame(1, 1, 0, 0, 0),
            ],
        };
        let seq: Vec<usize> = (0..6).map(|t| s.frame_at(t)).collect();
        assert_eq!(seq, vec![0, 0, 1, 2, 0, 0]);
        assert_eq!(SpriteSeq::default().frame_at(5), 0);
    }

    #[test]
    fn billboard_lands_pixels_on_screen() {
        let g = ViewGeom {
            target: [0.0, 0.0],
            zoom: 1.0,
            size: [200.0, 200.0],
        };
        let f = frame(40, 30, 10, 20, 1);
        let lift = lift_for(f.below(), 2.0);
        let (pos, _, _) = billboard(&f, lift);
        let anchor = g.to_screen([0.0, 0.0, 0.0]);
        // Top-left corner is 10 px left of and 20 px above the anchor.
        let tl = g.to_screen(pos[0]);
        assert!((tl[0] - (anchor[0] - 10.0)).abs() < 1e-3, "{tl:?}");
        assert!((tl[1] - (anchor[1] - 20.0)).abs() < 1e-3, "{tl:?}");
        let br = g.to_screen(pos[2]);
        assert!((br[0] - (anchor[0] + 30.0)).abs() < 1e-3);
        assert!((br[1] - (anchor[1] + 10.0)).abs() < 1e-3);
        // The lowest edge stays above the ground plane.
        assert!(pos[2][1] > 0.0 && pos[3][1] > 0.0, "{pos:?}");
    }
}
