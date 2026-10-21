//! `anim`: run a unit's COB script headless against its 3DO model and write
//! per-tick piece transforms (JSON) plus a PNG strip of sampled frames.
//!
//! The VM state is all integer; floats appear only in the rasterizer below,
//! which turns the integer transforms into pixels. Rotation convention for
//! the picture (**Believed**, chosen so knees bend backward in walk cycles):
//! each piece is translated by its model offset plus the script's move
//! offset, then rotated `Ry(-y) * Rx(-x) * Rz(-z)` with angles in 65,536ths
//! of a turn; children compose onto parents. See `docs/formats/cob-vm.md`.

use std::path::Path;
use std::sync::Arc;

use serde_json::json;
use tak_formats::model3do::{self, FIXED_ONE, Model};
use tak_script::{HeadlessEngine, Instance, PieceTransform, Program};

type Res = Result<(), Box<dyn std::error::Error>>;

pub struct AnimArgs<'a> {
    pub unit: &'a str,
    pub script: &'a str,
    pub ticks: u32,
    pub set: &'a [String],
    pub warmup: u32,
    pub frames: u32,
    pub seed: u64,
    pub out: &'a Path,
}

/// Reads one `key = value;` from FBI text (case-insensitive key).
fn fbi_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (k, v) = line.trim().split_once('=')?;
        k.trim()
            .eq_ignore_ascii_case(key)
            .then(|| v.trim().trim_end_matches(';').trim().to_ascii_lowercase())
    })
}

fn parse_set(s: &str) -> Result<(i32, i32), String> {
    let (p, v) = s
        .split_once('=')
        .ok_or_else(|| format!("--set wants PORT=VALUE, got {s:?}"))?;
    let p = p.trim().parse().map_err(|e| format!("{s:?}: {e}"))?;
    let v = v.trim().parse().map_err(|e| format!("{s:?}: {e}"))?;
    Ok((p, v))
}

pub fn run(vfs: &tak_vfs::Vfs, args: &AnimArgs) -> Res {
    let unit = args.unit.to_ascii_lowercase();
    let (script_name, object) = match vfs.read(&format!("units/{unit}.fbi")) {
        Ok(b) => {
            let text = String::from_utf8_lossy(&b).into_owned();
            (
                fbi_value(&text, "unitname").unwrap_or_else(|| unit.clone()),
                fbi_value(&text, "objectname").unwrap_or_else(|| unit.clone()),
            )
        }
        Err(_) => (unit.clone(), unit.clone()),
    };
    let cob_path = format!("scripts/{script_name}.cob");
    let model_path = format!("objects3d/{object}.3do");
    let program = Arc::new(
        Program::from_bytes(&vfs.read(&cob_path)?).map_err(|e| format!("{cob_path}: {e}"))?,
    );
    let model =
        model3do::parse(&vfs.read(&model_path)?).map_err(|e| format!("{model_path}: {e}"))?;

    // COB pieces are names; resolve each to a model piece (case-insensitive).
    let cob_pieces = &program.script().pieces;
    let mut model_to_cob = vec![None; model.pieces.len()];
    let mut unresolved = Vec::new();
    for (ci, name) in cob_pieces.iter().enumerate() {
        match model.find(name) {
            Some(mi) => model_to_cob[mi] = model_to_cob[mi].or(Some(ci)),
            None => unresolved.push(name.clone()),
        }
    }

    let mut eng = HeadlessEngine::new(args.seed);
    for s in args.set {
        let (p, v) = parse_set(s)?;
        eng.pin(p, v);
    }
    let mut inst = Instance::new(Arc::clone(&program));
    inst.start(args.script, &[])?;
    for _ in 0..args.warmup {
        inst.tick(&mut eng);
    }

    let mut frames_json = Vec::with_capacity(args.ticks as usize);
    let mut poses: Vec<(u64, Vec<PieceTransform>)> = Vec::new();
    let every = (args.ticks / args.frames.max(1)).max(1);
    for t in 0..args.ticks {
        inst.tick(&mut eng);
        let tf = inst.transforms();
        let pieces: Vec<_> = tf
            .iter()
            .zip(cob_pieces)
            .map(|(p, name)| {
                json!({
                    "name": name,
                    "offset": p.offset.map(|v| v.raw()),
                    "rotation": p.rotation.map(|a| a.raw()),
                    "visible": p.visible,
                })
            })
            .collect();
        frames_json.push(json!({
            "tick": inst.now(),
            "anim_hash": format!("{:016x}", inst.anim_hash()),
            "pieces": pieces,
        }));
        if t % every == 0 && poses.len() < args.frames as usize {
            poses.push((inst.now(), tf));
        }
    }

    let errors: Vec<String> = inst
        .errors()
        .iter()
        .map(|e| {
            let f = &program.script().functions[e.function as usize].name;
            format!("tick {} {f} @{}: {}", e.tick, e.pc, e.error)
        })
        .collect();
    let doc = json!({
        "unit": unit,
        "script": cob_path,
        "model": model_path,
        "entry": args.script,
        "warmup_ticks": args.warmup,
        "pinned_ports": args.set,
        "linear_units": "16.16 fixed point, model units",
        "angle_units": "65536 per turn",
        "unresolved_pieces": unresolved,
        "unknown_port_reads": eng.unknown_reads,
        "unknown_port_writes": eng.unknown_writes,
        "errors": errors,
        "frames": frames_json,
    });
    std::fs::create_dir_all(args.out)?;
    let json_path = args.out.join(format!("{unit}_anim.json"));
    std::fs::write(&json_path, serde_json::to_string(&doc)?)?;

    let png_path = args.out.join(format!("{unit}_anim.png"));
    let img = render_strip(&model, &model_to_cob, &poses);
    img.save(&png_path)?;
    eprintln!(
        "{cob_path} + {model_path}: {} ticks after {} warmup, {} errors, {} threads alive -> {}, {} ({} frames: ticks {:?})",
        args.ticks,
        args.warmup,
        inst.error_count(),
        inst.thread_count(),
        json_path.display(),
        png_path.display(),
        poses.len(),
        poses.iter().map(|p| p.0).collect::<Vec<_>>()
    );
    Ok(())
}

// ---- export-only rasterizer (floats allowed here, never in VM state) ----

type M3 = [[f64; 3]; 3];
type V3 = [f64; 3];

fn mul(a: &M3, b: &M3) -> M3 {
    let mut r = [[0.0; 3]; 3];
    for (i, row) in r.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    r
}

fn apply(m: &M3, v: V3) -> V3 {
    [0, 1, 2].map(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

fn rot(axis: usize, units: u16) -> M3 {
    let a = f64::from(units) / 65536.0 * std::f64::consts::TAU;
    let (s, c) = a.sin_cos();
    match axis {
        0 => [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]],
        1 => [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]],
        _ => [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]],
    }
}

fn fx(v: i32) -> f64 {
    f64::from(v) / f64::from(FIXED_ONE)
}

/// World-space triangles (with a color) for one pose.
fn pose_triangles(
    model: &Model,
    model_to_cob: &[Option<usize>],
    pose: &[PieceTransform],
) -> Vec<([V3; 3], [u8; 3])> {
    let n = model.pieces.len();
    let mut world: Vec<(M3, V3)> = Vec::with_capacity(n);
    let mut visible = vec![true; n];
    let mut tris = Vec::new();
    for (i, piece) in model.pieces.iter().enumerate() {
        let tf = model_to_cob[i].and_then(|c| pose.get(c)).copied();
        let mut local_t = piece.offset.map(fx);
        let mut local_r = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        if let Some(tf) = tf {
            for (axis, t) in local_t.iter_mut().enumerate() {
                *t += fx(tf.offset[axis].raw());
            }
            let neg = |a: tak_math::Angle| a.raw().wrapping_neg();
            local_r = mul(
                &rot(1, neg(tf.rotation[1])),
                &mul(&rot(0, neg(tf.rotation[0])), &rot(2, neg(tf.rotation[2]))),
            );
            visible[i] = tf.visible;
        }
        let (r, t) = match piece.parent.filter(|&p| p < i) {
            Some(p) => {
                let (pr, pt) = world[p];
                let lt = apply(&pr, local_t);
                (
                    mul(&pr, &local_r),
                    [pt[0] + lt[0], pt[1] + lt[1], pt[2] + lt[2]],
                )
            }
            None => (local_r, local_t),
        };
        world.push((r, t));
        if !visible[i] {
            continue;
        }
        let verts: Vec<V3> = piece
            .vertices
            .iter()
            .map(|v| {
                let p = apply(&r, v.map(fx));
                [p[0] + t[0], p[1] + t[1], p[2] + t[2]]
            })
            .collect();
        for prim in &piece.primitives {
            if prim.indices.len() < 3 {
                continue;
            }
            let color = prim_color(prim.texture.as_deref(), prim.color_index);
            let idx: Vec<usize> = prim.indices.iter().map(|&x| usize::from(x)).collect();
            if idx.iter().any(|&x| x >= verts.len()) {
                continue;
            }
            for k in 1..idx.len() - 1 {
                tris.push(([verts[idx[0]], verts[idx[k]], verts[idx[k + 1]]], color));
            }
        }
    }
    tris
}

fn prim_color(tex: Option<&str>, color: u32) -> [u8; 3] {
    match tex {
        Some(t) => {
            let h = t.to_ascii_lowercase().bytes().fold(0x811c_9dc5u32, |h, b| {
                (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
            });
            [0, 8, 16].map(|s| 80 + ((h >> s) & 0x7f) as u8)
        }
        None => {
            let g = 90 + ((color & 0xff) / 3) as u8;
            [g, g, g]
        }
    }
}

const CELL_W: u32 = 180;
const CELL_H: u32 = 220;

/// Two rows of frames: side view (model faces right) and a three-quarter view.
fn render_strip(
    model: &Model,
    model_to_cob: &[Option<usize>],
    poses: &[(u64, Vec<PieceTransform>)],
) -> image::RgbImage {
    let cols = poses.len().max(1) as u32;
    let mut img =
        image::RgbImage::from_pixel(cols * CELL_W, 2 * CELL_H, image::Rgb([235, 235, 228]));
    let views: [M3; 2] = [rot(1, 0xC000), rot(1, 0xC000 - 0x1555)];
    let all: Vec<_> = poses
        .iter()
        .map(|(_, p)| pose_triangles(model, model_to_cob, p))
        .collect();
    for (row, view) in views.iter().enumerate() {
        // One scale per row so frames are comparable.
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for tris in &all {
            for (t, _) in tris {
                for v in t {
                    let p = apply(view, *v);
                    lo = [lo[0].min(p[0]), lo[1].min(p[1])];
                    hi = [hi[0].max(p[0]), hi[1].max(p[1])];
                }
            }
        }
        if lo[0] > hi[0] {
            continue;
        }
        let span = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1e-3);
        let scale = 0.85 * f64::from(CELL_W.min(CELL_H)) / span;
        let cx = (lo[0] + hi[0]) / 2.0;
        let cy = (lo[1] + hi[1]) / 2.0;
        for (col, tris) in all.iter().enumerate() {
            let ox = col as f64 * f64::from(CELL_W) + f64::from(CELL_W) / 2.0;
            let oy = row as f64 * f64::from(CELL_H) + f64::from(CELL_H) / 2.0;
            let mut zbuf = vec![f64::MIN; (CELL_W * CELL_H) as usize];
            for (t, color) in tris {
                let p = t.map(|v| apply(view, v));
                let s = p.map(|v| [ox + (v[0] - cx) * scale, oy - (v[1] - cy) * scale, v[2]]);
                // Flat shading from the face normal against a fixed light.
                let e1 = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
                let e2 = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
                let nrm = [
                    e1[1] * e2[2] - e1[2] * e2[1],
                    e1[2] * e2[0] - e1[0] * e2[2],
                    e1[0] * e2[1] - e1[1] * e2[0],
                ];
                let len = (nrm[0] * nrm[0] + nrm[1] * nrm[1] + nrm[2] * nrm[2]).sqrt();
                let lit = if len > 0.0 {
                    let l = [0.4, 0.7, 0.6];
                    ((nrm[0] * l[0] + nrm[1] * l[1] + nrm[2] * l[2]) / len).abs()
                } else {
                    0.5
                };
                let shade = 0.45 + 0.55 * lit;
                let rgb = color.map(|c| (f64::from(c) * shade).min(255.0) as u8);
                fill(&mut img, &mut zbuf, col as u32, row as u32, s, rgb);
            }
        }
    }
    // Cell separators.
    for c in 1..cols {
        for y in 0..img.height() {
            img.put_pixel(c * CELL_W, y, image::Rgb([160, 160, 160]));
        }
    }
    for x in 0..img.width() {
        img.put_pixel(x, CELL_H, image::Rgb([160, 160, 160]));
    }
    img
}

fn fill(
    img: &mut image::RgbImage,
    zbuf: &mut [f64],
    col: u32,
    row: u32,
    s: [[f64; 3]; 3],
    rgb: [u8; 3],
) {
    let x0 = f64::from(col * CELL_W);
    let y0 = f64::from(row * CELL_H);
    let minx = s
        .iter()
        .map(|v| v[0])
        .fold(f64::MAX, f64::min)
        .max(x0)
        .floor();
    let maxx = s
        .iter()
        .map(|v| v[0])
        .fold(f64::MIN, f64::max)
        .min(x0 + f64::from(CELL_W) - 1.0)
        .ceil();
    let miny = s
        .iter()
        .map(|v| v[1])
        .fold(f64::MAX, f64::min)
        .max(y0)
        .floor();
    let maxy = s
        .iter()
        .map(|v| v[1])
        .fold(f64::MIN, f64::max)
        .min(y0 + f64::from(CELL_H) - 1.0)
        .ceil();
    let area =
        (s[1][0] - s[0][0]) * (s[2][1] - s[0][1]) - (s[2][0] - s[0][0]) * (s[1][1] - s[0][1]);
    if area.abs() < 1e-9 || minx > maxx || miny > maxy {
        return;
    }
    let mut y = miny;
    while y <= maxy {
        let mut x = minx;
        while x <= maxx {
            let (px, py) = (x + 0.5, y + 0.5);
            let w0 = ((s[1][0] - px) * (s[2][1] - py) - (s[2][0] - px) * (s[1][1] - py)) / area;
            let w1 = ((s[2][0] - px) * (s[0][1] - py) - (s[0][0] - px) * (s[2][1] - py)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                let z = w0 * s[0][2] + w1 * s[1][2] + w2 * s[2][2];
                let lx = (x - x0) as usize;
                let ly = (y - y0) as usize;
                let zi = ly * CELL_W as usize + lx;
                // Larger z is nearer the camera.
                if zi < zbuf.len() && z > zbuf[zi] {
                    zbuf[zi] = z;
                    img.put_pixel(x as u32, y as u32, image::Rgb(rgb));
                }
            }
            x += 1.0;
        }
        y += 1.0;
    }
}
