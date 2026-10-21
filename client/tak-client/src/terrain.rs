//! Terrain: TNT heights as a mesh, textured with the terrain JPEG pieces
//! exactly as `tak-cli dump-map` assembles its mosaic (tile cell `(x, y)`
//! shows the 32×32 piece `(col, row)` of `terrain/<id>.jpg`).
//!
//! The mosaic is screen-space art: mosaic pixel `(x, y)` shows the world
//! point `(x, z)` with `y = z - SHEAR * height` (**Believed**, the height
//! map's contours line up with the painted cliffs and craters this way and
//! not without the shift; `docs/client.md`). So a mesh vertex at world
//! `(x, h, z)` samples mosaic pixel `(x, z - SHEAR * h)`: under the oblique
//! projection the art then shows on screen exactly as the flat mosaic, and
//! the heights only give depth for occlusion. Chunk images therefore carry
//! a margin of rows from the chunk to the north.
//!
//! World units = map pixels. One attribute cell is 16 units and carries one
//! height sample (the raw TNT byte, **Believed** one world unit per step, as
//! the sim uses it). Mesh vertices sit on cell corners with the average of
//! the touching cells, so the surface passes through the sim's per-cell
//! heights on average; the sim itself samples the containing cell without
//! interpolation (`docs/sim.md`).

use std::collections::BTreeMap;

use tak_formats::tnt::{self, PIECE_PX, TileGrid};
use tak_vfs::Vfs;

use crate::view::SHEAR;

/// Tile rows of northern margin in each chunk image: enough for the
/// largest shift, `SHEAR * 255` = 128 px.
pub const MARGIN_TILES: u32 = 4;

/// Tile cells per texture chunk side (32 × 32 px each, so 1024 px).
pub const CHUNK_TILES: u32 = 32;
/// World units per attribute cell.
pub const CELL: f32 = 16.0;

/// One chunk's texture.
pub struct ChunkImage {
    /// First tile cell (x, y) covered by the mesh.
    pub tile0: (u32, u32),
    /// Tile cells covered by the mesh (w, h).
    pub tiles: (u32, u32),
    /// Tile rows of margin above `tile0.1` included in the image.
    pub margin: u32,
    /// RGBA8, `tiles.0 * 32` × `(tiles.1 + margin) * 32`.
    pub rgba: Vec<u8>,
}

impl ChunkImage {
    /// Image size in pixels.
    pub fn image_size(&self) -> (u32, u32) {
        (
            self.tiles.0 * PIECE_PX,
            (self.tiles.1 + self.margin) * PIECE_PX,
        )
    }
}

/// Terrain data shared by the mesh builder and the picking ray.
pub struct TerrainData {
    /// Attribute cells (w, h).
    pub cells: (u32, u32),
    /// Height per cell corner, `(w + 1) * (h + 1)`, row-major.
    pub corners: Vec<f32>,
    /// Sea level in height units.
    pub sea_level: f32,
    /// Texture chunks.
    pub chunks: Vec<ChunkImage>,
    /// Tile cells with no usable JPEG piece.
    pub missing: usize,
}

impl TerrainData {
    /// Map size in world units.
    pub fn size(&self) -> (f32, f32) {
        (self.cells.0 as f32 * CELL, self.cells.1 as f32 * CELL)
    }

    /// Corner height at corner `(cx, cz)`, clamped.
    pub fn corner(&self, cx: i64, cz: i64) -> f32 {
        let w = i64::from(self.cells.0);
        let h = i64::from(self.cells.1);
        let x = cx.clamp(0, w) as usize;
        let z = cz.clamp(0, h) as usize;
        self.corners[z * (w as usize + 1) + x]
    }

    /// Height of the rendered surface at a world point (bilinear between
    /// corners; the mesh triangles differ by a fraction of a unit).
    pub fn height_at(&self, x: f32, z: f32) -> f32 {
        let fx = (x / CELL).max(0.0);
        let fz = (z / CELL).max(0.0);
        let (ix, iz) = (fx.floor() as i64, fz.floor() as i64);
        let (tx, tz) = (fx - ix as f32, fz - iz as f32);
        let h00 = self.corner(ix, iz);
        let h10 = self.corner(ix + 1, iz);
        let h01 = self.corner(ix, iz + 1);
        let h11 = self.corner(ix + 1, iz + 1);
        let a = h00 + (h10 - h00) * tx;
        let b = h01 + (h11 - h01) * tx;
        a + (b - a) * tz
    }

    /// First hit of a ray (origin, unit direction) with the surface, by
    /// marching between the height-255 and height-0 planes.
    pub fn raycast(&self, o: [f32; 3], d: [f32; 3]) -> Option<[f32; 3]> {
        if d[1].abs() < 1e-6 {
            return None;
        }
        let t_top = (256.0 - o[1]) / d[1];
        let t_bot = (0.0 - o[1]) / d[1];
        let (t0, t1) = (t_top.min(t_bot), t_top.max(t_bot));
        let at = |t: f32| [o[0] + d[0] * t, o[1] + d[1] * t, o[2] + d[2] * t];
        let above = |p: [f32; 3]| p[1] - self.height_at(p[0], p[2]);
        let steps = 256;
        let mut prev_t = t0;
        let mut prev = above(at(t0));
        for i in 1..=steps {
            let t = t0 + (t1 - t0) * i as f32 / steps as f32;
            let cur = above(at(t));
            if prev >= 0.0 && cur < 0.0 {
                // Refine by bisection.
                let (mut a, mut b) = (prev_t, t);
                for _ in 0..16 {
                    let m = 0.5 * (a + b);
                    if above(at(m)) >= 0.0 {
                        a = m;
                    } else {
                        b = m;
                    }
                }
                return Some(at(0.5 * (a + b)));
            }
            prev_t = t;
            prev = cur;
        }
        Some(at(t1))
    }
}

/// Corner heights: the mean of the (up to four) cells touching each corner.
pub fn corner_heights(w: u32, h: u32, heights: &[u8]) -> Vec<f32> {
    let cell = |x: i64, z: i64| -> Option<f32> {
        if x < 0 || z < 0 || x >= i64::from(w) || z >= i64::from(h) {
            return None;
        }
        heights
            .get(z as usize * w as usize + x as usize)
            .map(|&v| f32::from(v))
    };
    let mut out = Vec::with_capacity((w as usize + 1) * (h as usize + 1));
    for cz in 0..=i64::from(h) {
        for cx in 0..=i64::from(w) {
            let s: Vec<f32> = [(cx - 1, cz - 1), (cx, cz - 1), (cx - 1, cz), (cx, cz)]
                .into_iter()
                .filter_map(|(x, z)| cell(x, z))
                .collect();
            out.push(if s.is_empty() {
                0.0
            } else {
                s.iter().sum::<f32>() / s.len() as f32
            });
        }
    }
    out
}

/// Load heights and textures for `map` (VFS path without extension).
pub fn load(vfs: &Vfs, map: &str) -> Result<TerrainData, String> {
    let path = format!("{map}.tnt");
    let bytes = vfs.read(&path).map_err(|e| format!("{path}: {e}"))?;
    let t = tnt::parse(&bytes).map_err(|e| format!("{path}: {e}"))?;
    let (w, h) = (t.width(), t.height());
    let corners = corner_heights(w, h, &t.heights);
    let (chunks, missing) = chunk_images(vfs, &t.tiles);
    Ok(TerrainData {
        cells: (w, h),
        corners,
        sea_level: t.header.sea_level as f32,
        chunks,
        missing,
    })
}

fn chunk_images(vfs: &Vfs, tiles: &TileGrid) -> (Vec<ChunkImage>, usize) {
    let mut cache: BTreeMap<u32, Option<image::RgbImage>> = BTreeMap::new();
    let mut missing = 0;
    let mut chunks = Vec::new();
    let p = PIECE_PX;
    for ty in (0..tiles.height).step_by(CHUNK_TILES as usize) {
        for tx in (0..tiles.width).step_by(CHUNK_TILES as usize) {
            let cw = CHUNK_TILES.min(tiles.width - tx);
            let ch = CHUNK_TILES.min(tiles.height - ty);
            let margin = MARGIN_TILES.min(ty);
            let (iw, ih) = (cw * p, (ch + margin) * p);
            let mut rgba = vec![0u8; (iw * ih * 4) as usize];
            for y in 0..ch + margin {
                for x in 0..cw {
                    let Some((id, col, row)) = tiles.get(tx + x, ty - margin + y) else {
                        if y >= margin {
                            missing += 1;
                        }
                        continue;
                    };
                    let jpg = cache.entry(id).or_insert_with(|| {
                        let b = vfs.read(&TileGrid::jpeg_path(id)).ok()?;
                        Some(image::load_from_memory(&b).ok()?.to_rgb8())
                    });
                    let (sx, sy) = (u32::from(col) * p, u32::from(row) * p);
                    match jpg {
                        Some(j) if sx + p <= j.width() && sy + p <= j.height() => {
                            for py in 0..p {
                                for px in 0..p {
                                    let c = j.get_pixel(sx + px, sy + py).0;
                                    let (dx, dy) = (x * p + px, y * p + py);
                                    let i = ((dy * iw + dx) * 4) as usize;
                                    rgba[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
                                }
                            }
                        }
                        _ if y >= margin => missing += 1,
                        _ => {}
                    }
                }
            }
            chunks.push(ChunkImage {
                tile0: (tx, ty),
                tiles: (cw, ch),
                margin,
                rgba,
            });
        }
    }
    (chunks, missing)
}

/// Mesh arrays for one chunk: positions, UVs (0..1 over the chunk image,
/// sheared as the module docs say), normals and triangle indices. Vertices
/// are on attribute-cell corners.
pub struct ChunkMesh {
    /// Positions `[x, height, z]`.
    pub positions: Vec<[f32; 3]>,
    /// Texture coordinates.
    pub uvs: Vec<[f32; 2]>,
    /// Smooth normals.
    pub normals: Vec<[f32; 3]>,
    /// Triangles.
    pub indices: Vec<u32>,
}

/// Build the mesh for `chunk`.
pub fn chunk_mesh(t: &TerrainData, chunk: &ChunkImage) -> ChunkMesh {
    let cx0 = i64::from(chunk.tile0.0 * 2);
    let cz0 = i64::from(chunk.tile0.1 * 2);
    let nx = i64::from(chunk.tiles.0 * 2);
    let nz = i64::from(chunk.tiles.1 * 2);
    let mut m = ChunkMesh {
        positions: Vec::new(),
        uvs: Vec::new(),
        normals: Vec::new(),
        indices: Vec::new(),
    };
    let (iw, ih) = chunk.image_size();
    let top = (chunk.tile0.1 - chunk.margin) as f32 * PIECE_PX as f32;
    for z in 0..=nz {
        for x in 0..=nx {
            let (cx, cz) = (cx0 + x, cz0 + z);
            let h = t.corner(cx, cz);
            m.positions.push([cx as f32 * CELL, h, cz as f32 * CELL]);
            let v = (cz as f32 * CELL - SHEAR * h - top) / ih as f32;
            debug_assert_eq!(iw, (nx as f32 * CELL) as u32);
            m.uvs.push([x as f32 / nx as f32, v]);
            let dx = t.corner(cx + 1, cz) - t.corner(cx - 1, cz);
            let dz = t.corner(cx, cz + 1) - t.corner(cx, cz - 1);
            let n = [-dx, 2.0 * CELL, -dz];
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            m.normals.push([n[0] / l, n[1] / l, n[2] / l]);
        }
    }
    let row = (nx + 1) as u32;
    for z in 0..nz as u32 {
        for x in 0..nx as u32 {
            let i = z * row + x;
            // Counter-clockwise seen from above (+Y), so faces point up.
            m.indices
                .extend_from_slice(&[i, i + row, i + 1, i + 1, i + row, i + row + 1]);
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(h: u8) -> TerrainData {
        TerrainData {
            cells: (4, 4),
            corners: corner_heights(4, 4, &[h; 16]),
            sea_level: 0.0,
            chunks: Vec::new(),
            missing: 0,
        }
    }

    #[test]
    fn corners_average_their_cells() {
        let c = corner_heights(2, 1, &[10, 30]);
        assert_eq!(c, vec![10.0, 20.0, 30.0, 10.0, 20.0, 30.0]);
    }

    #[test]
    fn ray_hits_flat_ground() {
        let t = flat(40);
        let d = [0.0, -0.894, -0.447];
        let l: f32 = d[1] * d[1] + d[2] * d[2];
        let d = [0.0, d[1] / l.sqrt(), d[2] / l.sqrt()];
        let hit = t.raycast([20.0, 300.0, 200.0], d).unwrap();
        assert!((hit[1] - 40.0).abs() < 0.05, "{hit:?}");
    }

    #[test]
    fn chunk_mesh_counts() {
        let t = flat(1);
        let c = ChunkImage {
            tile0: (0, 0),
            tiles: (2, 2),
            margin: 0,
            rgba: Vec::new(),
        };
        let m = chunk_mesh(&t, &c);
        assert_eq!(m.positions.len(), 25);
        assert_eq!(m.indices.len(), 4 * 4 * 6);
        assert_eq!(m.positions[24], [64.0, 1.0, 64.0]);
        // Sheared UV: height 1 moves the sample half a pixel north.
        assert!((m.uvs[24][1] - (64.0 - 0.5) / 64.0).abs() < 1e-6);
        assert!((m.uvs[24][0] - 1.0).abs() < 1e-6);
    }
}
