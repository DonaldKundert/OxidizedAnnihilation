//! 3DO model piece trees (.3do). Work package WP-B (M4a).
//!
//! A model is a tree of pieces linked by absolute file offsets. Each piece has
//! a name, a translation from its parent, its own vertices, and primitives
//! (faces) that index into those vertices. Unit scripts animate pieces by name
//! and by index, so the piece order produced here is part of the contract with
//! the script VM. See `docs/formats/3do.md` for what is Known and what is not.
//!
//! Positions stay as the raw signed 16.16 fixed-point integers from the file.
//! Convert to floats only when exporting.

use crate::reader::{ReadError, Reader};

/// Size of one piece (object) record.
pub const PIECE_RECORD_LEN: usize = 52;
/// Size of one primitive record.
pub const PRIMITIVE_RECORD_LEN: usize = 32;
/// Size of one vertex record.
pub const VERTEX_RECORD_LEN: usize = 12;
/// Deepest child nesting accepted. The retail corpus reaches 13.
pub const MAX_DEPTH: usize = 64;

/// One unit of the fixed-point coordinate scale (1.0 world unit).
pub const FIXED_ONE: i32 = 1 << 16;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Model3doError {
    #[error(transparent)]
    Read(#[from] ReadError),
    #[error("piece record at offset {offset} is reached twice (cycle or shared node)")]
    Cycle { offset: u32 },
    #[error("more pieces than the file has room for ({max})")]
    TooManyPieces { max: usize },
    #[error("piece nesting deeper than {MAX_DEPTH} at offset {offset}")]
    TooDeep { offset: u32 },
    #[error("negative {what} ({value}) in piece record at offset {offset}")]
    NegativeCount {
        what: &'static str,
        value: i32,
        offset: u32,
    },
    #[error("negative {what} offset ({value}) at file offset {at}")]
    NegativeOffset {
        what: &'static str,
        value: i32,
        at: usize,
    },
    #[error("piece {piece:?} primitive {primitive}: vertex index {index} >= vertex count {count}")]
    VertexIndex {
        piece: String,
        primitive: usize,
        index: u16,
        count: usize,
    },
}

/// A parsed model: every piece in depth-first pre-order (a piece, then its
/// children's subtrees in sibling order, then its next sibling).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Model {
    /// `pieces[0]` is the root. Indices into this vector are piece indices.
    pub pieces: Vec<Piece>,
}

/// A position or offset in signed 16.16 fixed point, `[x, y, z]`, Y up.
pub type FixedVec3 = [i32; 3];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    pub name: String,
    /// Index of the parent piece, `None` for the root (and for any pieces on
    /// the root's sibling chain; none exist in retail data).
    pub parent: Option<usize>,
    /// Child piece indices, in sibling-list order.
    pub children: Vec<usize>,
    /// Translation from the parent's origin.
    pub offset: FixedVec3,
    pub vertices: Vec<FixedVec3>,
    pub primitives: Vec<Primitive>,
    /// Raw selection-primitive field: a primitive index, `-1` for none.
    pub selection_primitive: i32,
    /// Raw version word (always 1 in retail data; not validated).
    pub version: u32,
    /// Raw auxiliary offset word at +0x20 (always 0 in retail data).
    pub aux_offset: u32,
    /// Absolute file offset of this piece's record.
    pub record_offset: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Primitive {
    /// Indices into the owning piece's `vertices`. 2 = line, 3 = triangle,
    /// 4 = quad, more = polygon.
    pub indices: Vec<u16>,
    /// Texture name: a GAF entry name, no extension. `None` = flat colored.
    pub texture: Option<String>,
    /// Raw color word. A palette index for untextured primitives; editor
    /// residue on many textured ones.
    pub color_index: u32,
    /// Raw flag word; bit 0 set on every untextured primitive in retail data.
    pub is_colored: u32,
    /// Raw words at +0x08 (aux offset), +0x14 and +0x18 (editor residue).
    pub unknown: [u32; 3],
}

impl Model {
    pub fn root(&self) -> &Piece {
        &self.pieces[0]
    }

    /// Finds a piece index by name, case-insensitively (first match).
    pub fn find(&self, name: &str) -> Option<usize> {
        self.pieces
            .iter()
            .position(|p| p.name.eq_ignore_ascii_case(name))
    }

    /// Sum of offsets from the root down to `piece`: its origin in model space.
    pub fn origin(&self, piece: usize) -> FixedVec3 {
        let mut acc = [0i32; 3];
        let mut cur = Some(piece);
        // A parent always has a smaller index, so this terminates.
        while let Some(i) = cur {
            let Some(p) = self.pieces.get(i) else { break };
            for (a, o) in acc.iter_mut().zip(p.offset) {
                *a = a.wrapping_add(o);
            }
            cur = p.parent.filter(|&q| q < i);
        }
        acc
    }

    /// Depth of the deepest piece; the root is depth 1.
    pub fn depth(&self) -> usize {
        let mut depth = vec![0usize; self.pieces.len()];
        for (i, p) in self.pieces.iter().enumerate() {
            depth[i] = p.parent.map_or(1, |q| depth[q] + 1);
        }
        depth.into_iter().max().unwrap_or(0)
    }

    pub fn vertex_count(&self) -> usize {
        self.pieces.iter().map(|p| p.vertices.len()).sum()
    }

    pub fn primitive_count(&self) -> usize {
        self.pieces.iter().map(|p| p.primitives.len()).sum()
    }
}

fn offset_field(r: &mut Reader<'_>, what: &'static str) -> Result<u32, Model3doError> {
    let at = r.pos();
    let v = r.i32(what)?;
    u32::try_from(v).map_err(|_| Model3doError::NegativeOffset { what, value: v, at })
}

fn count_field(r: &mut Reader<'_>, what: &'static str, rec: u32) -> Result<u32, Model3doError> {
    let v = r.i32(what)?;
    u32::try_from(v).map_err(|_| Model3doError::NegativeCount {
        what,
        value: v,
        offset: rec,
    })
}

/// Parses a whole `.3do` file.
pub fn parse(data: &[u8]) -> Result<Model, Model3doError> {
    // Every piece needs its own 52-byte record, so this caps the piece count.
    let max_pieces = data.len() / PIECE_RECORD_LEN;
    let mut visited = std::collections::BTreeSet::new();
    let mut pieces: Vec<Piece> = Vec::new();
    // (record offset, parent index, depth). Pop order yields pre-order:
    // a piece's child is pushed after its sibling, so it is visited first.
    let mut stack: Vec<(u32, Option<usize>, usize)> = vec![(0, None, 1)];

    while let Some((off, parent, depth)) = stack.pop() {
        if depth > MAX_DEPTH {
            return Err(Model3doError::TooDeep { offset: off });
        }
        if !visited.insert(off) {
            return Err(Model3doError::Cycle { offset: off });
        }
        if pieces.len() >= max_pieces.max(1) {
            return Err(Model3doError::TooManyPieces { max: max_pieces });
        }

        let (piece, sibling, child) = parse_piece(data, off, parent)?;
        let idx = pieces.len();
        pieces.push(piece);
        if let Some(p) = parent {
            pieces[p].children.push(idx);
        }
        if sibling != 0 {
            stack.push((sibling, parent, depth));
        }
        if child != 0 {
            stack.push((child, Some(idx), depth + 1));
        }
    }
    Ok(Model { pieces })
}

fn parse_piece(
    data: &[u8],
    off: u32,
    parent: Option<usize>,
) -> Result<(Piece, u32, u32), Model3doError> {
    let mut r = Reader::at(data, off.into(), "piece record")?;
    // Make sure the whole record is present before reading fields.
    r.check_count(1, PIECE_RECORD_LEN, "piece record")?;
    let version = r.u32("version")?;
    let n_vert = count_field(&mut r, "vertex count", off)?;
    let n_prim = count_field(&mut r, "primitive count", off)?;
    let selection_primitive = r.i32("selection primitive")?;
    let offset = [r.i32("x offset")?, r.i32("y offset")?, r.i32("z offset")?];
    let name_off = offset_field(&mut r, "piece name")?;
    let aux_offset = r.u32("aux offset")?;
    let vert_off = offset_field(&mut r, "vertex array")?;
    let prim_off = offset_field(&mut r, "primitive array")?;
    let sibling = offset_field(&mut r, "sibling piece")?;
    let child = offset_field(&mut r, "child piece")?;

    let name = Reader::at(data, name_off.into(), "piece name")?.cstr("piece name")?;

    let mut vr = Reader::at(data, vert_off.into(), "vertex array")?;
    let n = vr.check_count(n_vert.into(), VERTEX_RECORD_LEN, "vertex array")?;
    let mut vertices = Vec::with_capacity(n);
    for _ in 0..n {
        vertices.push([
            vr.i32("vertex x")?,
            vr.i32("vertex y")?,
            vr.i32("vertex z")?,
        ]);
    }

    let mut pr = Reader::at(data, prim_off.into(), "primitive array")?;
    let n = pr.check_count(n_prim.into(), PRIMITIVE_RECORD_LEN, "primitive array")?;
    let mut primitives = Vec::with_capacity(n);
    for i in 0..n {
        let prim = parse_primitive(data, &mut pr, off)?;
        if let Some(&bad) = prim
            .indices
            .iter()
            .find(|&&v| usize::from(v) >= vertices.len())
        {
            return Err(Model3doError::VertexIndex {
                piece: name,
                primitive: i,
                index: bad,
                count: vertices.len(),
            });
        }
        primitives.push(prim);
    }

    let piece = Piece {
        name,
        parent,
        children: Vec::new(),
        offset,
        vertices,
        primitives,
        selection_primitive,
        version,
        aux_offset,
        record_offset: off,
    };
    Ok((piece, sibling, child))
}

fn parse_primitive(
    data: &[u8],
    r: &mut Reader<'_>,
    piece_off: u32,
) -> Result<Primitive, Model3doError> {
    let color_index = r.u32("color index")?;
    let n_idx = count_field(r, "vertex index count", piece_off)?;
    let aux = r.u32("primitive aux offset")?;
    let idx_off = offset_field(r, "vertex index array")?;
    let tex_off = offset_field(r, "texture name")?;
    let u1 = r.u32("primitive unknown 1")?;
    let u2 = r.u32("primitive unknown 2")?;
    let is_colored = r.u32("is colored")?;

    let mut ir = Reader::at(data, idx_off.into(), "vertex index array")?;
    let n = ir.check_count(n_idx.into(), 2, "vertex index array")?;
    let mut indices = Vec::with_capacity(n);
    for _ in 0..n {
        indices.push(ir.u16("vertex index")?);
    }
    let texture = if tex_off == 0 {
        None
    } else {
        Some(Reader::at(data, tex_off.into(), "texture name")?.cstr("texture name")?)
    };
    Ok(Primitive {
        indices,
        texture,
        color_index,
        is_colored,
        unknown: [aux, u1, u2],
    })
}

/// Hand-builds `.3do` bytes for tests. Not for retail data.
#[cfg(test)]
pub(crate) mod testing {
    /// A synthetic piece for [`build`].
    pub struct SynthPiece {
        pub name: &'static str,
        pub offset: [i32; 3],
        pub vertices: Vec<[i32; 3]>,
        /// (indices, texture name)
        pub primitives: Vec<(Vec<u16>, Option<&'static str>)>,
        pub children: Vec<SynthPiece>,
    }

    /// Serializes a piece tree with the root at offset 0, in the same field
    /// layout as retail files (but not the same string/array placement).
    pub fn build(root: &SynthPiece) -> Vec<u8> {
        let mut out = Vec::new();
        emit(&mut out, root, None);
        out
    }

    fn put(out: &mut [u8], at: usize, v: u32) {
        out[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    /// Writes `p` (and its subtree) and returns its record offset.
    fn emit(out: &mut Vec<u8>, p: &SynthPiece, _parent: Option<usize>) -> usize {
        let rec = out.len();
        out.resize(rec + super::PIECE_RECORD_LEN, 0);
        let name_off = out.len();
        out.extend(p.name.bytes());
        out.push(0);
        let vert_off = out.len();
        for v in &p.vertices {
            for c in v {
                out.extend(c.to_le_bytes());
            }
        }
        let mut prim_meta = Vec::new();
        for (idx, tex) in &p.primitives {
            let io = out.len();
            for i in idx {
                out.extend(i.to_le_bytes());
            }
            let to = match tex {
                Some(t) => {
                    let o = out.len();
                    out.extend(t.bytes());
                    out.push(0);
                    o
                }
                None => 0,
            };
            prim_meta.push((idx.len(), io, to, tex.is_none()));
        }
        let prim_off = out.len();
        for (n, io, to, flat) in prim_meta {
            let words = [
                if flat { 12 } else { 0 },
                n as u32,
                0,
                io as u32,
                to as u32,
                0,
                0,
                flat as u32,
            ];
            for w in words {
                out.extend(w.to_le_bytes());
            }
        }
        let header = [
            1u32,
            p.vertices.len() as u32,
            p.primitives.len() as u32,
            if p.primitives.is_empty() { u32::MAX } else { 0 },
            p.offset[0] as u32,
            p.offset[1] as u32,
            p.offset[2] as u32,
            name_off as u32,
            0,
            vert_off as u32,
            prim_off as u32,
            0,
            0,
        ];
        for (k, w) in header.iter().enumerate() {
            put(out, rec + 4 * k, *w);
        }
        let mut prev: Option<usize> = None;
        for c in &p.children {
            let co = emit(out, c, Some(rec));
            match prev {
                None => put(out, rec + 48, co as u32),
                Some(s) => put(out, s + 44, co as u32),
            }
            prev = Some(co);
        }
        rec
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{SynthPiece, build};
    use super::*;

    fn leaf(name: &'static str, offset: [i32; 3]) -> SynthPiece {
        SynthPiece {
            name,
            offset,
            vertices: vec![[0, 0, 0]],
            primitives: vec![],
            children: vec![],
        }
    }

    fn sample() -> Vec<u8> {
        let f = FIXED_ONE;
        build(&SynthPiece {
            name: "base",
            offset: [0, 0, 0],
            vertices: vec![
                [-f, 0, -f],
                [f, 0, -f],
                [f, 0, f],
                [-f, 0, f],
                [0, 2 * f, 0],
            ],
            primitives: vec![
                (vec![0, 1, 2, 3], None),
                (vec![0, 1, 4], Some("stone1")),
                (vec![2, 4], None),
            ],
            children: vec![
                SynthPiece {
                    name: "turret",
                    offset: [0, 3 * f, f / 2],
                    vertices: vec![[0, 0, 0], [f, 0, 0], [0, f, 0]],
                    primitives: vec![(vec![0, 1, 2], Some("metal"))],
                    children: vec![leaf("flare", [0, 0, -4 * f])],
                },
                leaf("smoke", [f, 0, 0]),
            ],
        })
    }

    #[test]
    fn parses_tree_in_preorder() {
        let m = parse(&sample()).unwrap();
        let names: Vec<_> = m.pieces.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["base", "turret", "flare", "smoke"]);
        assert_eq!(m.pieces[0].children, [1, 3]);
        assert_eq!(m.pieces[1].children, [2]);
        assert_eq!(m.pieces[2].parent, Some(1));
        assert_eq!(m.pieces[3].parent, Some(0));
        assert_eq!(m.depth(), 3);
        assert_eq!(m.vertex_count(), 10);
        assert_eq!(m.primitive_count(), 4);
        let base = m.root();
        assert_eq!(base.primitives[0].indices, [0, 1, 2, 3]);
        assert_eq!(base.primitives[0].texture, None);
        assert_eq!(base.primitives[0].is_colored, 1);
        assert_eq!(base.primitives[1].texture.as_deref(), Some("stone1"));
        assert_eq!(base.primitives[2].indices.len(), 2);
        assert_eq!(base.selection_primitive, 0);
        assert_eq!(m.pieces[2].selection_primitive, -1);
        assert_eq!(m.find("FLARE"), Some(2));
        assert_eq!(
            m.origin(2),
            [0, 3 * FIXED_ONE, FIXED_ONE / 2 - 4 * FIXED_ONE]
        );
    }

    #[test]
    fn rejects_cycles() {
        let mut d = sample();
        let m = parse(&d).unwrap();
        let flare = m.pieces[2].record_offset as usize;
        // A child pointer of 0 means "none", so loop back to the turret.
        let turret = m.pieces[1].record_offset;
        d[flare + 48..flare + 52].copy_from_slice(&turret.to_le_bytes());
        assert!(matches!(parse(&d), Err(Model3doError::Cycle { .. })));
        // Self-sibling.
        let mut d = sample();
        let smoke = m.pieces[3].record_offset;
        let s = smoke as usize;
        d[s + 44..s + 48].copy_from_slice(&smoke.to_le_bytes());
        assert!(matches!(parse(&d), Err(Model3doError::Cycle { .. })));
    }

    #[test]
    fn rejects_bad_vertex_index() {
        let f = FIXED_ONE;
        let d = build(&SynthPiece {
            name: "base",
            offset: [0; 3],
            vertices: vec![[0, 0, 0], [f, 0, 0]],
            primitives: vec![(vec![0, 1, 2], None)],
            children: vec![],
        });
        assert!(matches!(
            parse(&d),
            Err(Model3doError::VertexIndex { index: 2, .. })
        ));
    }

    #[test]
    fn rejects_too_deep() {
        let mut p = leaf("p", [0; 3]);
        for _ in 0..MAX_DEPTH {
            p = SynthPiece {
                name: "p",
                offset: [0; 3],
                vertices: vec![],
                primitives: vec![],
                children: vec![p],
            };
        }
        assert!(matches!(
            parse(&build(&p)),
            Err(Model3doError::TooDeep { .. })
        ));
    }

    #[test]
    fn rejects_huge_counts_without_allocating() {
        let mut d = sample();
        d[4..8].copy_from_slice(&0x7fff_ffffu32.to_le_bytes());
        assert!(matches!(
            parse(&d),
            Err(Model3doError::Read(ReadError::CountTooLarge { .. }))
        ));
        let mut d = sample();
        d[8..12].copy_from_slice(&(-5i32).to_le_bytes());
        assert!(matches!(
            parse(&d),
            Err(Model3doError::NegativeCount { .. })
        ));
    }

    #[test]
    fn truncation_and_byte_flip_never_panic() {
        let d = sample();
        for n in 0..d.len() {
            let _ = parse(&d[..n]);
        }
        for i in 0..d.len() {
            for flip in [0x01u8, 0x80, 0xff] {
                let mut e = d.clone();
                e[i] ^= flip;
                let _ = parse(&e);
            }
        }
    }

    proptest::proptest! {
        #[test]
        fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(proptest::num::u8::ANY, 0..512)) {
            let _ = parse(&bytes);
        }
    }

    #[test]
    fn corpus_parses() {
        let Some(dir) = tak_vfs::testing::data_dir_or_skip("model3do::corpus_parses") else {
            return;
        };
        let vfs = tak_vfs::open_kingdoms(&dir).unwrap();
        let files: Vec<_> = vfs
            .list("objects3d/")
            .into_iter()
            .filter(|f| f.path.ends_with(".3do"))
            .collect();
        let mut errors = Vec::new();
        let mut piece_hist = std::collections::BTreeMap::<usize, usize>::new();
        let mut vert_buckets = std::collections::BTreeMap::<usize, usize>::new();
        let mut idx_hist = std::collections::BTreeMap::<usize, usize>::new();
        let mut dup_names = Vec::new();
        let (mut max_depth, mut textured, mut flat) = (0, 0usize, 0usize);
        for f in &files {
            let bytes = vfs.read(&f.path).unwrap();
            match parse(&bytes) {
                Ok(m) => {
                    *piece_hist.entry(m.pieces.len()).or_default() += 1;
                    let v = m.vertex_count();
                    *vert_buckets
                        .entry(v.checked_next_power_of_two().unwrap_or(v))
                        .or_default() += 1;
                    max_depth = max_depth.max(m.depth());
                    for p in &m.pieces {
                        assert_eq!(p.version, 1, "{}", f.path);
                        assert_eq!(p.aux_offset, 0, "{}", f.path);
                        for pr in &p.primitives {
                            *idx_hist.entry(pr.indices.len()).or_default() += 1;
                            if pr.texture.is_some() {
                                textured += 1;
                            } else {
                                flat += 1;
                                assert_eq!(pr.is_colored & 1, 1, "{}", f.path);
                            }
                        }
                    }
                    assert!(
                        m.pieces.iter().skip(1).all(|p| p.parent.is_some()),
                        "root sibling chain in {}",
                        f.path
                    );
                    let mut names: Vec<String> =
                        m.pieces.iter().map(|p| p.name.to_lowercase()).collect();
                    names.sort();
                    let n = names.len();
                    names.dedup();
                    if names.len() != n {
                        dup_names.push(f.path.clone());
                    }
                }
                Err(e) => errors.push(format!("{}: {e}", f.path)),
            }
        }
        eprintln!("parsed {} models, max depth {max_depth}", files.len());
        eprintln!("pieces per model: {piece_hist:?}");
        eprintln!("vertices per model (<= power of two): {vert_buckets:?}");
        eprintln!("indices per primitive: {idx_hist:?}");
        eprintln!("primitives textured {textured}, flat {flat}");
        eprintln!("models with duplicate piece names: {dup_names:?}");
        assert!(errors.is_empty(), "{errors:#?}");
        assert_eq!(files.len(), 447);
        // Documented exceptions in docs/formats/3do.md.
        assert_eq!(dup_names.len(), 6, "{dup_names:?}");
    }
}
