//! `render`: run a real-map match headless up to a tick and draw a top-down
//! PNG: the terrain mosaic (downscaled, as `dump-map` builds it), impassable
//! cells tinted red, each monarch's movement trail, its start position, its
//! final position and a heading tick. Output can contain retail art; keep it
//! outside the repo.

use image::{Rgb, RgbImage, imageops};
use std::collections::BTreeMap;
use std::error::Error;
use std::path::PathBuf;
use tak_formats::tnt::{self, PIECE_PX, TileGrid};
use tak_sim::map::terrain;
use tak_sim::testing::walk_log;
use tak_sim::{PlayerSetup, Sim, load_match};
use tak_vfs::Vfs;

type Res<T> = Result<T, Box<dyn Error>>;

#[derive(clap::Args)]
pub struct RenderArgs {
    /// Map: VFS path without extension, e.g. "kmap/Adamantine Gate".
    #[arg(long)]
    map: String,
    /// Sides, comma-separated (one monarch each).
    #[arg(long, default_value = "ARA,ZON")]
    players: String,
    /// Command log JSON. Without one, each monarch walks to a random point
    /// every 450 ticks (`tak-headless gen-log --kind walk`, seed 1).
    #[arg(long)]
    log: Option<PathBuf>,
    /// Draw the state after this many ticks.
    #[arg(long)]
    tick: u32,
    /// Output folder (writes render.png). Keep it outside the repo.
    #[arg(long)]
    out: PathBuf,
    /// Output pixels per 32-px tile cell (32 = full size).
    #[arg(long, default_value_t = 8)]
    piece: u32,
    /// World RNG seed.
    #[arg(long, default_value_t = 1)]
    world_seed: u64,
    /// Run without COB scripts.
    #[arg(long)]
    no_scripts: bool,
}

const COLORS: [[u8; 3]; 8] = [
    [255, 64, 64],
    [64, 160, 255],
    [255, 220, 40],
    [80, 230, 80],
    [255, 128, 0],
    [200, 80, 255],
    [0, 230, 230],
    [255, 255, 255],
];

pub fn run(vfs: &Vfs, a: RenderArgs) -> Res<()> {
    let piece = a.piece.clamp(1, PIECE_PX);
    let players = PlayerSetup::parse_list(&a.players);
    let m = load_match(vfs, &a.map, &players, a.world_seed)?;
    let log = match &a.log {
        Some(p) => tak_headless::load_log(Some(p))?,
        None => walk_log(&m.scenario, a.tick, 450, 1),
    };
    let sc = m.scenario.clone();
    let mut sim = if a.no_scripts {
        Sim::new(&sc)?
    } else {
        Sim::with_scripts(&sc, Box::new(m.scripts))?
    };
    // Trails in world units, per player.
    let mut trails: Vec<Vec<(i32, i32)>> = vec![Vec::new(); sc.players.len()];
    for _ in 0..a.tick {
        let t = sim.tick();
        sim.step(log.for_tick(t));
        for (i, p) in sim.world().players().iter().enumerate() {
            if let Some(u) = p.monarch.and_then(|id| sim.world().unit(id)) {
                let pt = (u.pos.x.to_int_floor(), u.pos.y.to_int_floor());
                if trails[i].last() != Some(&pt) {
                    trails[i].push(pt);
                }
            }
        }
    }

    let t = tnt::parse(&vfs.read(&format!("{}.tnt", a.map))?)?;
    let mut img = mosaic(vfs, &t.tiles, piece)?;
    // World unit -> pixel: piece px per 32 world units.
    let px = |v: i32| v.saturating_mul(piece as i32) / PIECE_PX as i32;

    // Tint impassable cells.
    if let Some(grid) = &sc.map.terrain {
        for r in 0..grid.rows {
            for c in 0..grid.cols {
                if grid.flags_of_cell(c, r) & terrain::BLOCKS == 0 {
                    continue;
                }
                let cs = grid.cell_size as i32;
                let (x0, y0) = (px(c as i32 * cs), px(r as i32 * cs));
                let (x1, y1) = (px((c as i32 + 1) * cs), px((r as i32 + 1) * cs));
                for y in y0..y1.max(y0 + 1) {
                    for x in x0..x1.max(x0 + 1) {
                        blend(&mut img, x, y, [255, 0, 0], 90);
                    }
                }
            }
        }
    }

    for (i, trail) in trails.iter().enumerate() {
        let col = COLORS[i % COLORS.len()];
        for w in trail.windows(2) {
            line(
                &mut img,
                px(w[0].0),
                px(w[0].1),
                px(w[1].0),
                px(w[1].1),
                col,
                1,
            );
        }
        let start = sc.map.start_positions[sc.players[i].start as usize];
        ring(&mut img, px(start.x), px(start.y), 6, [255, 255, 255]);
    }
    let mut summary = String::new();
    for (i, p) in sim.world().players().iter().enumerate() {
        let Some(u) = p.monarch.and_then(|id| sim.world().unit(id)) else {
            continue;
        };
        let col = COLORS[i % COLORS.len()];
        let (x, y) = (u.pos.x.to_int_floor(), u.pos.y.to_int_floor());
        disc(&mut img, px(x), px(y), 5, [0, 0, 0]);
        disc(&mut img, px(x), px(y), 4, col);
        let dir = tak_math::Vec2::from_angle(u.heading).scale(tak_math::Fx::from_int(14));
        let (hx, hy) = (dir.x.to_int_round(), dir.y.to_int_round());
        line(
            &mut img,
            px(x),
            px(y),
            px(x) + hx,
            px(y) + hy,
            [255, 255, 255],
            1,
        );
        let name = &sc.unit_defs[u.def.0 as usize].name;
        summary.push_str(&format!(
            "player {i} {name}: ({x}, {y}) heading {} speed {} ground {}, {} trail points\n",
            u.heading.raw(),
            u.speed,
            u.ground,
            trails[i].len()
        ));
    }
    std::fs::create_dir_all(&a.out)?;
    let path = a.out.join("render.png");
    img.save(&path)?;
    print!("{summary}");
    eprintln!(
        "tick {}: wrote {} ({}x{})",
        a.tick,
        path.display(),
        img.width(),
        img.height()
    );
    Ok(())
}

/// Terrain from `terrain/*.jpg` pieces, `piece` px per tile cell.
fn mosaic(vfs: &Vfs, tiles: &TileGrid, piece: u32) -> Res<RgbImage> {
    let mut img = RgbImage::new(tiles.width * piece, tiles.height * piece);
    let mut cache: BTreeMap<u32, Option<RgbImage>> = BTreeMap::new();
    for y in 0..tiles.height {
        for x in 0..tiles.width {
            let Some((id, col, row)) = tiles.get(x, y) else {
                continue;
            };
            let jpg = cache.entry(id).or_insert_with(|| {
                let bytes = vfs.read(&TileGrid::jpeg_path(id)).ok()?;
                let im = image::load_from_memory(&bytes).ok()?.to_rgb8();
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
            if let Some(j) = jpg
                && cx + piece <= j.width()
                && cy + piece <= j.height()
            {
                let sub = imageops::crop_imm(j, cx, cy, piece, piece).to_image();
                imageops::replace(&mut img, &sub, i64::from(x * piece), i64::from(y * piece));
            }
        }
    }
    Ok(img)
}

fn put(img: &mut RgbImage, x: i32, y: i32, c: [u8; 3]) {
    if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
        img.put_pixel(x as u32, y as u32, Rgb(c));
    }
}

/// Mix `c` over the pixel with `alpha` / 255.
fn blend(img: &mut RgbImage, x: i32, y: i32, c: [u8; 3], alpha: u16) {
    if x < 0 || y < 0 || x as u32 >= img.width() || y as u32 >= img.height() {
        return;
    }
    let p = img.get_pixel_mut(x as u32, y as u32);
    for (dst, &src) in p.0.iter_mut().zip(&c) {
        let v = (u16::from(*dst) * (255 - alpha) + u16::from(src) * alpha) / 255;
        *dst = v as u8;
    }
}

fn line(img: &mut RgbImage, x0: i32, y0: i32, x1: i32, y1: i32, c: [u8; 3], r: i32) {
    let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
    let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
    let (mut x, mut y, mut err) = (x0, y0, dx + dy);
    loop {
        for oy in -r + 1..r {
            for ox in -r + 1..r {
                put(img, x + ox, y + oy, c);
            }
        }
        put(img, x, y, c);
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

fn disc(img: &mut RgbImage, cx: i32, cy: i32, r: i32, c: [u8; 3]) {
    for y in -r..=r {
        for x in -r..=r {
            if x * x + y * y <= r * r {
                put(img, cx + x, cy + y, c);
            }
        }
    }
}

fn ring(img: &mut RgbImage, cx: i32, cy: i32, r: i32, c: [u8; 3]) {
    for y in -r..=r {
        for x in -r..=r {
            let d = x * x + y * y;
            if d <= r * r && d >= (r - 1) * (r - 1) {
                put(img, cx + x, cy + y, c);
            }
        }
    }
}
