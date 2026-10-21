//! `dump-sprite`: write every frame of a GAF/TAF/GAO (or a PCX) as PNG.

use std::io::Write as _;
use std::path::Path;
use tak_formats::gaf::palette::Palette;
use tak_formats::gaf::pcx::Pcx;
use tak_formats::gaf::{Gaf, Rgba8Image};
use tak_vfs::Vfs;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

/// Loads a palette from a `.pal` (1,024-byte RGBX) or a PCX trailer.
fn load_palette(vfs: &Vfs, path: &str) -> Res<Palette> {
    let data = vfs.read(path)?;
    if data.len() == Palette::PAL_LEN {
        return Ok(Palette::from_pal(&data)?);
    }
    Pcx::parse(&data)?
        .palette
        .ok_or_else(|| format!("{path} has no palette").into())
}

/// Picks a palette for indexed art: `--palette`, else a same-named `.pcx`
/// beside the sprite, else `palettes/<side>_textures.pcx` for `textures/`,
/// else `palettes/palette.pal`. Kingdoms' real per-file palette rules are not
/// fully known yet (see docs/formats/palettes.md).
fn pick_palette(vfs: &Vfs, sprite: &str, explicit: Option<&str>) -> Res<(Palette, String)> {
    if let Some(p) = explicit {
        return Ok((load_palette(vfs, p)?, p.to_string()));
    }
    let lower = sprite.to_ascii_lowercase();
    let stem = lower.rsplit_once('.').map_or(lower.as_str(), |(s, _)| s);
    let mut candidates = vec![format!("{stem}.pcx")];
    if let Some(name) = stem.strip_prefix("textures/") {
        let side: String = name.chars().take(3).collect();
        candidates.push(format!("palettes/{side}_textures.pcx"));
    }
    candidates.push("palettes/palette.pal".into());
    for c in candidates {
        if vfs.stat(&c).is_some() {
            return Ok((load_palette(vfs, &c)?, c));
        }
    }
    Ok((Palette::grayscale(), "grayscale".into()))
}

fn save(img: &Rgba8Image, path: &Path) -> Res<()> {
    if img.width == 0 || img.height == 0 {
        return Ok(());
    }
    let buf = image::RgbaImage::from_raw(img.width, img.height, img.data.clone())
        .ok_or("image buffer size mismatch")?;
    buf.save(path)?;
    Ok(())
}

fn safe_name(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// All frames on one image, one row per entry, over a checkerboard so that
/// transparency is visible. Rows are capped at 16 frames.
fn contact_sheet(rows: &[Vec<Rgba8Image>]) -> Rgba8Image {
    const GAP: u32 = 2;
    const MAX_W: u32 = 4096;
    const MAX_H: u32 = 8192;
    let row_h = |r: &Vec<Rgba8Image>| r.iter().map(|i| i.height).max().unwrap_or(0);
    let row_w = |r: &Vec<Rgba8Image>| r.iter().take(16).map(|i| i.width + GAP).sum::<u32>();
    let w = rows.iter().map(row_w).max().unwrap_or(0).clamp(1, MAX_W);
    let h = rows
        .iter()
        .map(|r| row_h(r) + GAP)
        .sum::<u32>()
        .clamp(1, MAX_H);
    let mut sheet = Rgba8Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let v = if (x / 8 + y / 8) % 2 == 0 { 200 } else { 140 };
            let i = ((y * w + x) * 4) as usize;
            sheet.data[i..i + 4].copy_from_slice(&[v, v, v, 255]);
        }
    }
    let mut y = 0i32;
    for r in rows {
        let mut x = 0i32;
        for img in r.iter().take(16) {
            sheet.draw(img, x, y);
            x += (img.width + GAP) as i32;
        }
        y += (row_h(r) + GAP) as i32;
    }
    sheet
}

pub fn dump_sprite(vfs: &Vfs, path: &str, out: &Path, palette: Option<&str>) -> Res<()> {
    let data = vfs.read(path)?;
    let base = Path::new(path)
        .file_stem()
        .map(|s| safe_name(&s.to_string_lossy()))
        .unwrap_or_default();
    let dir = out.join(&base);
    std::fs::create_dir_all(&dir)?;
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".pcx") || (lower.ends_with(".pal") && data.len() != Palette::PAL_LEN) {
        let p = Pcx::parse(&data)?;
        let pal = p.palette.clone().unwrap_or_else(Palette::grayscale);
        let mut img = Rgba8Image::new(p.width, p.height);
        for (i, &ix) in p.pixels.iter().enumerate() {
            let [r, g, b] = pal.rgb(ix);
            img.data[i * 4..i * 4 + 4].copy_from_slice(&[r, g, b, 255]);
        }
        save(&img, &dir.join(format!("{base}.png")))?;
        eprintln!("{path}: {}x{} PCX -> {}", p.width, p.height, dir.display());
        return Ok(());
    }

    let gaf = Gaf::parse(&data)?;
    let (pal, pal_name) = pick_palette(vfs, path, palette)?;
    let mut stdout = std::io::stdout().lock();
    let mut n = 0usize;
    let mut rows: Vec<Vec<Rgba8Image>> = Vec::new();
    for (ei, e) in gaf.entries.iter().enumerate() {
        let mut row = Vec::new();
        for (fi, r) in e.frames.iter().enumerate() {
            let f = &r.frame;
            let name = format!("{ei:03}_{}_{fi:03}.png", safe_name(&e.name));
            let img = f.to_rgba(&pal);
            save(&img, &dir.join(name))?;
            row.push(img);
            n += 1;
            if fi == 0 {
                writeln!(
                    stdout,
                    "{ei:3} {:32} {:3} frames  {}x{} at ({},{})  {}  dur {}",
                    e.name,
                    e.frames.len(),
                    f.width,
                    f.height,
                    f.x_offset,
                    f.y_offset,
                    f.format.name(),
                    r.duration
                )?;
            }
        }
        rows.push(row);
    }
    save(&contact_sheet(&rows), &dir.join("_sheet.png"))?;
    eprintln!(
        "{path}: {} entries, {n} frames, palette {pal_name} -> {}",
        gaf.entries.len(),
        dir.display()
    );
    Ok(())
}
