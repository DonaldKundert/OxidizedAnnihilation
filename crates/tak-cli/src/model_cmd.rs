//! `dump-model`: export a 3DO piece tree as Wavefront OBJ + MTL and a JSON
//! piece tree, for checking models in an external viewer.
//!
//! Textures are not resolved yet (WP-C); each texture name gets a stable flat
//! color derived from its name, and untextured primitives get a gray keyed by
//! their palette index. Floats appear only here, never in the parsed model.

use std::fmt::Write as _;
use std::path::Path;

use serde_json::json;
use tak_formats::model3do::{self, FIXED_ONE, Model};

type Res = Result<(), Box<dyn std::error::Error>>;

pub fn dump_model(vfs: &tak_vfs::Vfs, path: &str, out: &Path) -> Res {
    let resolved = if vfs.stat(path).is_some() {
        path.to_string()
    } else {
        format!("objects3d/{}.3do", path.trim_end_matches(".3do"))
    };
    let bytes = vfs.read(&resolved)?;
    let model = model3do::parse(&bytes).map_err(|e| format!("{resolved}: {e}"))?;
    let stem = Path::new(&resolved)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("model")
        .to_lowercase();
    std::fs::create_dir_all(out)?;
    let (obj, mtl) = to_obj(&model, &stem);
    std::fs::write(out.join(format!("{stem}.obj")), obj)?;
    std::fs::write(out.join(format!("{stem}.mtl")), mtl)?;
    let tree = serde_json::to_string_pretty(&to_json(&model, &resolved))?;
    std::fs::write(out.join(format!("{stem}.json")), tree)?;
    eprintln!(
        "{resolved}: {} pieces, {} vertices, {} primitives -> {}",
        model.pieces.len(),
        model.vertex_count(),
        model.primitive_count(),
        out.join(format!("{stem}.obj")).display()
    );
    Ok(())
}

fn fx(v: i32) -> f64 {
    f64::from(v) / f64::from(FIXED_ONE)
}

fn material_name(tex: Option<&str>, color: u32) -> String {
    match tex {
        Some(t) => format!("tex_{}", t.to_lowercase()),
        None => format!("pal_{}", color & 0xff),
    }
}

/// FNV-1a, for stable placeholder colors.
fn hash(s: &str) -> u32 {
    s.bytes().fold(0x811c_9dc5u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    })
}

fn material_rgb(name: &str) -> [f64; 3] {
    if let Some(idx) = name.strip_prefix("pal_") {
        let g = 0.35 + f64::from(idx.parse::<u8>().unwrap_or(0)) / 255.0 * 0.5;
        return [g, g, g];
    }
    let h = hash(name);
    let c = |shift: u32| 0.3 + f64::from((h >> shift) & 0xff) / 255.0 * 0.6;
    [c(0), c(8), c(16)]
}

/// Builds OBJ and MTL text. Each piece becomes an OBJ object placed at its
/// accumulated origin (bind pose), in authored coordinates (Y up).
pub fn to_obj(model: &Model, stem: &str) -> (String, String) {
    let mut obj = format!(
        "# exported by tak-cli dump-model; units: 1.0 = 65536 file units\nmtllib {stem}.mtl\n"
    );
    let mut materials = std::collections::BTreeSet::new();
    let mut base = 1usize; // OBJ indices are 1-based and global.
    for (i, piece) in model.pieces.iter().enumerate() {
        let o = model.origin(i);
        let _ = writeln!(obj, "o {}_{}", i, piece.name);
        for v in &piece.vertices {
            let _ = writeln!(
                obj,
                "v {:.5} {:.5} {:.5}",
                fx(v[0].wrapping_add(o[0])),
                fx(v[1].wrapping_add(o[1])),
                fx(v[2].wrapping_add(o[2]))
            );
        }
        for prim in &piece.primitives {
            let mat = material_name(prim.texture.as_deref(), prim.color_index);
            let _ = writeln!(obj, "usemtl {mat}");
            materials.insert(mat);
            let kind = if prim.indices.len() < 3 { "l" } else { "f" };
            obj.push_str(kind);
            for &ix in &prim.indices {
                let _ = write!(obj, " {}", base + usize::from(ix));
            }
            obj.push('\n');
        }
        base += piece.vertices.len();
    }
    let mut mtl = String::from("# placeholder flat colors until textures are decoded\n");
    for m in &materials {
        let [r, g, b] = material_rgb(m);
        let _ = writeln!(mtl, "newmtl {m}\nKd {r:.3} {g:.3} {b:.3}\nKa 0 0 0\nd 1\n");
    }
    (obj, mtl)
}

fn to_json(model: &Model, path: &str) -> serde_json::Value {
    let pieces: Vec<_> = model
        .pieces
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let mut textures: Vec<&str> = p
                .primitives
                .iter()
                .filter_map(|q| q.texture.as_deref())
                .collect();
            textures.sort_unstable();
            textures.dedup();
            json!({
                "index": i,
                "name": p.name,
                "parent": p.parent,
                "children": p.children,
                "offset_fixed": p.offset,
                "offset": p.offset.map(fx),
                "origin": model.origin(i).map(fx),
                "vertices": p.vertices.len(),
                "primitives": p.primitives.len(),
                "selection_primitive": p.selection_primitive,
                "textures": textures,
            })
        })
        .collect();
    json!({
        "path": path,
        "piece_count": model.pieces.len(),
        "vertex_count": model.vertex_count(),
        "primitive_count": model.primitive_count(),
        "depth": model.depth(),
        "pieces": pieces,
    })
}
