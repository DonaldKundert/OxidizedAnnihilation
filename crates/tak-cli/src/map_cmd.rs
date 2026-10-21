//! `dump-map`: writes PNGs of a TNT's heights, minimap and terrain mosaic.

use image::{GrayImage, Rgb, RgbImage, imageops};
use std::collections::BTreeMap;
use std::path::Path;
use tak_formats::tnt::{self, Cell, PIECE_PX, TileGrid};
use tak_vfs::Vfs;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

/// Dumps `path` (a VFS path to a `.tnt`) into `out`.
///
/// `palette` is a VFS path to a `.pcx` (palette in the last 768 bytes) or a
/// 1024-byte `.pal`. Without one, the kingdom named in the sibling `.ota`
/// picks `palettes/<kingdom>.pcx`. `piece` is the output size in pixels of
/// one tile cell in the terrain mosaic (0 skips the mosaic).
pub fn dump_map(vfs: &Vfs, path: &str, out: &Path, palette: Option<&str>, piece: u32) -> Res<()> {
    let t = tnt::parse(&vfs.read(path)?)?;
    std::fs::create_dir_all(out)?;
    let (w, h) = (t.width(), t.height());
    println!(
        "{path}: {w} x {h} cells, sea level {}, {} features, {} distinct tiles",
        t.header.sea_level,
        t.features.len(),
        t.tiles
            .ids
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    );

    let heights = GrayImage::from_raw(w, h, t.heights.clone()).ok_or("height size")?;
    heights.save(out.join("height.png"))?;

    // Heights shaded blue below sea level, features green, paved red.
    let mut cells = RgbImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let v = t.height_at(x, y).unwrap_or(0);
            let px = match t.cell(x, y) {
                Some(Cell::Feature(_)) => Rgb([0, 255, 0]),
                Some(Cell::Paved) => Rgb([200, 60, 60]),
                Some(Cell::Marker) => Rgb([255, 255, 0]),
                _ if u32::from(v) < t.header.sea_level => Rgb([0, 0, 96 + v / 2]),
                _ => Rgb([v, v, v]),
            };
            cells.put_pixel(x, y, px);
        }
    }
    cells.save(out.join("cells.png"))?;

    let pal = match palette {
        Some(p) => Some(load_palette(&vfs.read(p)?)?),
        None => kingdom_palette(vfs, path),
    };
    for (name, img) in [("minimap", &t.minimap), ("minimap_small", &t.minimap_small)] {
        if let Some(m) = img {
            let mut im = RgbImage::new(m.width, m.height);
            for (i, &p) in m.pixels.iter().enumerate() {
                let c = match &pal {
                    Some(pal) => pal[usize::from(p)],
                    None => [p, p, p],
                };
                im.put_pixel(i as u32 % m.width, i as u32 / m.width, Rgb(c));
            }
            im.save(out.join(format!("{name}.png")))?;
        }
    }

    if piece > 0 {
        mosaic(vfs, &t.tiles, piece)?.save(out.join("terrain.png"))?;
    }
    eprintln!("wrote PNGs to {}", out.display());
    Ok(())
}

/// Assembles the terrain from `terrain/*.jpg` pieces.
fn mosaic(vfs: &Vfs, tiles: &TileGrid, piece: u32) -> Res<RgbImage> {
    let mut img = RgbImage::new(tiles.width * piece, tiles.height * piece);
    let mut cache: BTreeMap<u32, Option<RgbImage>> = BTreeMap::new();
    let mut missing = 0usize;
    for y in 0..tiles.height {
        for x in 0..tiles.width {
            let Some((id, col, row)) = tiles.get(x, y) else {
                continue;
            };
            let jpg = cache.entry(id).or_insert_with(|| {
                let bytes = vfs.read(&TileGrid::jpeg_path(id)).ok()?;
                let im = image::load_from_memory(&bytes).ok()?.to_rgb8();
                // Each piece is PIECE_PX source pixels; scale so it becomes `piece` px.
                if piece == PIECE_PX {
                    return Some(im);
                }
                let (w, h) = (
                    (im.width() * piece / PIECE_PX).max(1),
                    (im.height() * piece / PIECE_PX).max(1),
                );
                Some(imageops::resize(&im, w, h, imageops::FilterType::Triangle))
            });
            let (cx, cy) = (u32::from(col) * piece, u32::from(row) * piece);
            match jpg {
                Some(j) if cx + piece <= j.width() && cy + piece <= j.height() => {
                    let sub = imageops::crop_imm(j, cx, cy, piece, piece).to_image();
                    imageops::replace(&mut img, &sub, i64::from(x * piece), i64::from(y * piece));
                }
                _ => missing += 1,
            }
        }
    }
    if missing > 0 {
        eprintln!("warning: {missing} tile cells had no usable JPEG piece");
    }
    Ok(img)
}

fn load_palette(b: &[u8]) -> Res<Vec<[u8; 3]>> {
    if b.len() == 1024 {
        return Ok(b.chunks_exact(4).map(|c| [c[0], c[1], c[2]]).collect());
    }
    if b.len() >= 769 && b[b.len() - 769] == 12 {
        return Ok(b[b.len() - 768..]
            .chunks_exact(3)
            .map(|c| [c[0], c[1], c[2]])
            .collect());
    }
    Err("palette is neither a 1024-byte .pal nor a PCX with a trailing palette".into())
}

/// Stopgap until the typed OTA view (on WP-A's TDF parser) lands: find
/// `kingdom=<name>;` in the sibling `.ota` and use `palettes/<name>.pcx`.
fn kingdom_palette(vfs: &Vfs, tnt_path: &str) -> Option<Vec<[u8; 3]>> {
    let stem = tnt_path.get(..tnt_path.len().checked_sub(4)?)?;
    let ota = vfs.read(&format!("{stem}.ota")).ok()?;
    let text: String = ota
        .iter()
        .map(|&c| c as char)
        .collect::<String>()
        .to_ascii_lowercase();
    let start = text.find("kingdom=")? + "kingdom=".len();
    let name: String = text[start..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    load_palette(&vfs.read(&format!("palettes/{name}.pcx")).ok()?).ok()
}
