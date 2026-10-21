//! [`MapDef`]: TNT terrain + typed OTA header + CRT summary, with features
//! resolved against a [`Ruleset`] and a first derived passability grid.

use crate::fields::{Coverage, Ctx, Issue};
use crate::ota::{OtaDef, ota_from_tdf};
use crate::ruleset::{DanglingRef, RefKind, Ruleset, read_tdf};
use serde::Serialize;
use tak_formats::tnt::{self, Cell, Tnt};
use tak_vfs::Vfs;

/// Errors that stop a map from building at all (the TNT is required).
#[derive(Debug, thiserror::Error)]
pub enum MapError {
    #[error("{path}: {source}")]
    Read {
        path: String,
        source: tak_vfs::VfsError,
    },
    #[error("{path}: {source}")]
    Tnt { path: String, source: tnt::TntError },
}

/// A feature instance from the TNT feature grid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlacedFeature {
    /// Name from the TNT feature table, as written.
    pub name: String,
    /// Lowercase key into [`Ruleset::features`], if the def exists.
    pub def: Option<String>,
    /// Cell holding the feature index (the anchor; see [`FeatureAnchor`]).
    pub x: u32,
    pub z: u32,
}

/// A player's start position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StartPosition {
    pub player: u8,
    /// Attribute-cell coordinates (Believed).
    pub x: i32,
    pub z: i32,
    pub source: StartSource,
}

/// Where a [`StartPosition`] came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum StartSource {
    /// An OTA `StartPosN` special.
    Marker,
    /// No marker for this slot: the first OTA `[UNITn]` owned by the player
    /// (3 missions, where the slot is an AI with a pre-placed base).
    FirstUnit,
}

/// The CRT header, when the map has one. The body is not decoded yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CrtSummary {
    pub kind: u32,
    pub count: u32,
    pub first_name: Option<String>,
    pub body_len: usize,
}

/// Where a feature's footprint sits relative to its anchor cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub enum FeatureAnchor {
    /// The anchor is the footprint's top-left cell. Default: it puts fewer
    /// start markers on features than `Center` (6 vs 13 of 1086); which
    /// one the engine uses is Unknown (`docs/content.md`).
    #[default]
    TopLeft,
    /// The anchor is the footprint's centre cell (rounded toward the
    /// top-left for even sizes).
    Center,
}

/// Tunables for [`PassGrid::derive`]. Heights are in TNT height units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PassRule {
    /// A cell is too steep when the spread (max - min) of its four corner
    /// heights exceeds this.
    pub max_slope: u8,
    /// A water cell is too deep when `sea_level - height` exceeds this.
    pub max_water_depth: u8,
    /// Blocking features occupy their footprint.
    pub features_block: bool,
    pub anchor: FeatureAnchor,
}

impl Default for PassRule {
    /// Matches `gamedata/moveinfo.tdf` `GROUND2` (MaxSlope 30, MaxWaterDepth 20).
    fn default() -> Self {
        PassRule {
            max_slope: 30,
            max_water_depth: 20,
            features_block: true,
            anchor: FeatureAnchor::TopLeft,
        }
    }
}

/// Per-cell facts and the derived ground passability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PassGrid {
    pub width: u32,
    pub height: u32,
    pub rule: PassRule,
    /// `width * height` bit sets of the `PassGrid::*` flags, row-major.
    pub cells: Vec<u8>,
}

impl PassGrid {
    /// Below sea level.
    pub const WATER: u8 = 1;
    /// Deeper than `rule.max_water_depth`.
    pub const DEEP: u8 = 2;
    /// Corner spread above `rule.max_slope`.
    pub const STEEP: u8 = 4;
    /// Covered by a blocking feature's footprint.
    pub const FEATURE: u8 = 8;
    /// TNT `0xFFFB` (paved, Believed).
    pub const PAVED: u8 = 16;
    /// TNT `0xFFFC` (Unknown).
    pub const MARKER: u8 = 32;
    /// Flags that stop a ground unit.
    pub const BLOCKS: u8 = Self::DEEP | Self::STEEP | Self::FEATURE;

    /// Derives the grid from terrain, sea level and placed features.
    pub fn derive(t: &Tnt, features: &[PlacedFeature], rs: &Ruleset, rule: PassRule) -> PassGrid {
        let (w, h) = (t.width(), t.height());
        let sea = t.header.sea_level;
        let mut cells = vec![0u8; w as usize * h as usize];
        let hgt = |x: u32, z: u32| t.height_at(x.min(w - 1), z.min(h - 1)).unwrap_or(0);
        for z in 0..h {
            for x in 0..w {
                let mut flags = 0u8;
                let corners = [hgt(x, z), hgt(x + 1, z), hgt(x, z + 1), hgt(x + 1, z + 1)];
                let lo = *corners.iter().min().unwrap_or(&0);
                let hi = *corners.iter().max().unwrap_or(&0);
                let here = u32::from(hgt(x, z));
                if here < sea {
                    flags |= Self::WATER;
                    if sea - here > u32::from(rule.max_water_depth) {
                        flags |= Self::DEEP;
                    }
                }
                if hi - lo > rule.max_slope {
                    flags |= Self::STEEP;
                }
                match t.cell(x, z) {
                    Some(Cell::Paved) => flags |= Self::PAVED,
                    Some(Cell::Marker) => flags |= Self::MARKER,
                    _ => {}
                }
                cells[z as usize * w as usize + x as usize] = flags;
            }
        }
        if rule.features_block {
            for pf in features {
                let Some(def) = pf.def.as_deref().and_then(|k| rs.features.get(k)) else {
                    continue;
                };
                if !def.blocking {
                    continue;
                }
                let (fw, fh) = (u32::from(def.footprint_x), u32::from(def.footprint_z));
                let (x0, z0) = match rule.anchor {
                    FeatureAnchor::TopLeft => (pf.x, pf.z),
                    FeatureAnchor::Center => (
                        pf.x.saturating_sub(fw.saturating_sub(1) / 2),
                        pf.z.saturating_sub(fh.saturating_sub(1) / 2),
                    ),
                };
                for z in z0..(z0 + fh).min(h) {
                    for x in x0..(x0 + fw).min(w) {
                        cells[z as usize * w as usize + x as usize] |= Self::FEATURE;
                    }
                }
            }
        }
        PassGrid {
            width: w,
            height: h,
            rule,
            cells,
        }
    }

    pub fn flags(&self, x: u32, z: u32) -> u8 {
        if x >= self.width || z >= self.height {
            return Self::BLOCKS;
        }
        self.cells[z as usize * self.width as usize + x as usize]
    }

    /// Ground units can stand on this cell.
    pub fn passable(&self, x: u32, z: u32) -> bool {
        self.flags(x, z) & Self::BLOCKS == 0
    }

    /// Cells with any of `mask` set.
    pub fn count(&self, mask: u8) -> usize {
        self.cells.iter().filter(|&&c| c & mask != 0).count()
    }
}

/// Everything the sim needs to start a game on one map.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MapDef {
    /// VFS path without extension (`kmap/Adamantine Gate`).
    pub name: String,
    pub ota: OtaDef,
    /// Attribute cells (16 px each).
    pub width: u32,
    pub height: u32,
    /// TNT sea level in height units.
    pub sea_level: u32,
    /// `width * height` heights, row-major.
    pub heights: Vec<u8>,
    /// `ota.kingdom`, lowercased.
    pub kingdom: Option<String>,
    /// `palettes/<kingdom>.pcx`.
    pub palette: Option<String>,
    pub features: Vec<PlacedFeature>,
    pub paved_cells: u32,
    pub marker_cells: u32,
    /// Sorted by player.
    pub start_positions: Vec<StartPosition>,
    /// Player slots the map offers: the largest `numplayers` value, else
    /// the number of `StartPos` markers.
    pub player_slots: u8,
    pub crt: Option<CrtSummary>,
    pub passability: PassGrid,
    /// OTA/TDF problems (empty on the retail corpus).
    pub issues: Vec<Issue>,
    /// Feature names with no def; OTA units with no unit def.
    pub dangling: Vec<DanglingRef>,
    /// OTA key coverage for this map.
    pub coverage: Coverage,
}

impl MapDef {
    /// Builds the map at `name` (VFS path without extension; `.tnt`
    /// required, `.ota` and `.crt` optional) with the default [`PassRule`].
    pub fn load(vfs: &Vfs, rs: &Ruleset, name: &str) -> Result<MapDef, MapError> {
        Self::load_with(vfs, rs, name, PassRule::default())
    }

    pub fn load_with(
        vfs: &Vfs,
        rs: &Ruleset,
        name: &str,
        rule: PassRule,
    ) -> Result<MapDef, MapError> {
        let name = name
            .strip_suffix(".tnt")
            .or_else(|| name.strip_suffix(".TNT"))
            .unwrap_or(name);
        let tnt_path = format!("{name}.tnt");
        let bytes = vfs.read(&tnt_path).map_err(|source| MapError::Read {
            path: tnt_path.clone(),
            source,
        })?;
        let t = tnt::parse(&bytes).map_err(|source| MapError::Tnt {
            path: tnt_path.clone(),
            source,
        })?;
        let mut ctx = Ctx::default();
        let ota_path = format!("{name}.ota");
        let ota = if vfs.stat(&ota_path).is_some() {
            read_tdf(vfs, &ota_path, &mut ctx)
                .map(|d| ota_from_tdf(&d, &ota_path, &mut ctx))
                .unwrap_or_default()
        } else {
            OtaDef::default()
        };
        let crt = vfs
            .read(&format!("{name}.crt"))
            .ok()
            .and_then(|b| tnt::crt::parse(&b).ok())
            .map(|c| CrtSummary {
                first_name: c.first_name(),
                kind: c.kind,
                count: c.count,
                body_len: c.body.len(),
            });
        let mut m = Self::build(name, &t, ota, crt, rs, rule, ctx.issues);
        m.coverage = ctx.coverage;
        Ok(m)
    }

    /// Assembles a map from parsed parts (no VFS access).
    pub fn build(
        name: &str,
        t: &Tnt,
        ota: OtaDef,
        crt: Option<CrtSummary>,
        rs: &Ruleset,
        rule: PassRule,
        issues: Vec<Issue>,
    ) -> MapDef {
        let from = format!("map {name}");
        let mut dangling = Vec::new();
        let defs: Vec<Option<String>> = t
            .features
            .iter()
            .map(|f| {
                let key = f.name.to_ascii_lowercase();
                if rs.features.contains_key(&key) {
                    Some(key)
                } else {
                    dangling.push(DanglingRef {
                        from: from.clone(),
                        field: "tnt features".into(),
                        kind: RefKind::Feature,
                        target: f.name.clone(),
                    });
                    None
                }
            })
            .collect();
        let mut features = Vec::new();
        let (mut paved, mut marker) = (0u32, 0u32);
        for z in 0..t.height() {
            for x in 0..t.width() {
                match t.cell(x, z) {
                    Some(Cell::Feature(i)) => {
                        let i = usize::from(i);
                        features.push(PlacedFeature {
                            name: t.features[i].name.clone(),
                            def: defs[i].clone(),
                            x,
                            z,
                        });
                    }
                    Some(Cell::Paved) => paved += 1,
                    Some(Cell::Marker) => marker += 1,
                    _ => {}
                }
            }
        }
        for u in &ota.units {
            if rs.unit(&u.unitname).is_none() {
                dangling.push(DanglingRef {
                    from: from.clone(),
                    field: "ota units".into(),
                    kind: RefKind::Unit,
                    target: u.unitname.clone(),
                });
            }
        }
        dangling.sort();
        dangling.dedup();
        let mut start_positions: Vec<StartPosition> = ota
            .start_positions()
            .into_iter()
            .map(|(player, x, z)| StartPosition {
                player,
                x,
                z,
                source: StartSource::Marker,
            })
            .collect();
        let player_slots = ota
            .num_players
            .iter()
            .copied()
            .max()
            .unwrap_or_else(|| u8::try_from(start_positions.len()).unwrap_or(u8::MAX));
        for p in 1..=player_slots {
            if start_positions.iter().any(|s| s.player == p) {
                continue;
            }
            if let Some(u) = ota.units.iter().find(|u| u.player == Some(i32::from(p))) {
                start_positions.push(StartPosition {
                    player: p,
                    x: u.x,
                    z: u.z,
                    source: StartSource::FirstUnit,
                });
            }
        }
        start_positions.sort_by_key(|s| s.player);
        let passability = PassGrid::derive(t, &features, rs, rule);
        MapDef {
            name: name.to_string(),
            kingdom: ota.kingdom.clone(),
            palette: ota.palette_path(),
            ota,
            width: t.width(),
            height: t.height(),
            sea_level: t.header.sea_level,
            heights: t.heights.clone(),
            features,
            paved_cells: paved,
            marker_cells: marker,
            start_positions,
            player_slots,
            crt,
            passability,
            issues,
            dangling,
            coverage: Coverage::default(),
        }
    }

    /// Height at a cell, `None` outside the map.
    pub fn height_at(&self, x: u32, z: u32) -> Option<u8> {
        if x >= self.width || z >= self.height {
            return None;
        }
        self.heights
            .get(z as usize * self.width as usize + x as usize)
            .copied()
    }

    /// Player slots `1..=player_slots` with no `StartPos` marker.
    pub fn missing_start_slots(&self) -> Vec<u8> {
        (1..=self.player_slots)
            .filter(|p| !self.start_positions.iter().any(|s| s.player == *p))
            .collect()
    }
}

/// Every playable map: VFS paths (without extension) of `.tnt` files that
/// have a sibling `.ota`, under `kmap/`, `maps/` and `missions/`.
pub fn playable_maps(vfs: &Vfs) -> Vec<String> {
    let mut out = Vec::new();
    for dir in ["kmap", "maps", "missions"] {
        for fi in vfs.list(dir) {
            if !fi.path.ends_with(".tnt") {
                continue;
            }
            let Some(stem) = fi.display_path.get(..fi.display_path.len() - 4) else {
                continue;
            };
            if vfs.stat(&format!("{stem}.ota")).is_some() {
                out.push(stem.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feature::FeatureDef;
    use std::collections::BTreeMap;
    use tak_formats::tnt::{Feature, Header, TileGrid};

    fn tnt(w: u32, h: u32, heights: Vec<u8>, grid: Vec<u16>, names: &[&str]) -> Tnt {
        Tnt {
            header: Header {
                width: w,
                height: h,
                sea_level: 10,
                ptr_heights: 52,
                ptr_feature_grid: 0,
                ptr_features: 0,
                feature_count: names.len() as u32,
                ptr_tile_ids: 0,
                ptr_tile_cols: 0,
                ptr_tile_rows: 0,
                ptr_minimap_small: 0,
                ptr_minimap: 0,
            },
            heights,
            feature_grid: grid,
            features: names
                .iter()
                .enumerate()
                .map(|(i, n)| Feature {
                    index: i as u32,
                    name: n.to_string(),
                })
                .collect(),
            tiles: TileGrid {
                width: w / 2,
                height: h / 2,
                ids: vec![],
                cols: vec![],
                rows: vec![],
            },
            minimap_small: None,
            minimap: None,
            unclaimed: vec![],
        }
    }

    fn rock() -> FeatureDef {
        FeatureDef {
            name: "TstRock".into(),
            source: "features/t.tdf".into(),
            world: None,
            description: None,
            category: vec![],
            footprint_x: 3,
            footprint_z: 3,
            height: None,
            blocking: true,
            hit_density: None,
            damage: None,
            indestructible: false,
            reclaimable: false,
            auto_reclaimable: false,
            resurrectable: false,
            flamable: false,
            permanent: false,
            is_building: false,
            is_stone: false,
            is_frozen: false,
            no_display_info: false,
            object: None,
            filename: None,
            seqname: None,
            seqname_shadow: None,
            energy: None,
            sacred_site: None,
            feature_dead: None,
            feature_burnt: None,
            feature_reclamate: None,
            burn_weapon: None,
            burn_min: None,
            burn_max: None,
            spread_chance: None,
            decompose_time: None,
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn builds_synthetic_map() {
        // 8x8: water in column 0, a cliff between columns 5 and 6, a 3x3
        // rock anchored at (3,3), one paved cell, one unknown feature.
        let (w, h) = (8u32, 8u32);
        let mut heights = vec![20u8; 64];
        let mut grid = vec![tnt::CELL_EMPTY; 64];
        for z in 0..h {
            heights[(z * w) as usize] = 5; // depth 5: water, shallow
            for x in 6..w {
                heights[(z * w + x) as usize] = 80;
            }
        }
        heights[(7 * w) as usize] = 0; // depth 10 at (0,7)
        grid[(3 * w + 3) as usize] = 0;
        grid[(w + 1) as usize] = 1;
        grid[(2 * w + 2) as usize] = tnt::CELL_PAVED;
        let t = tnt(w, h, heights, grid, &["TstRock", "TstMissing"]);
        let mut rs = Ruleset::default();
        rs.features.insert("tstrock".into(), rock());
        let ota = OtaDef {
            num_players: vec![2],
            specials: vec![
                crate::ota::Special {
                    what: "StartPos2".into(),
                    x: 6,
                    z: 6,
                },
                crate::ota::Special {
                    what: "StartPos1".into(),
                    x: 1,
                    z: 2,
                },
            ],
            ..OtaDef::default()
        };
        let rule = PassRule {
            max_water_depth: 7,
            anchor: FeatureAnchor::Center,
            ..PassRule::default()
        };
        let m = MapDef::build("kmap/tst", &t, ota, None, &rs, rule, vec![]);
        assert_eq!(m.features.len(), 2);
        assert_eq!(m.features[1].def.as_deref(), Some("tstrock"));
        assert_eq!(m.dangling.len(), 1);
        assert_eq!(m.dangling[0].target, "TstMissing");
        assert_eq!(m.paved_cells, 1);
        assert_eq!(m.player_slots, 2);
        assert_eq!(m.start_positions[0].player, 1);
        assert!(m.missing_start_slots().is_empty());
        let p = &m.passability;
        assert_eq!(p.flags(0, 0), PassGrid::WATER);
        assert!(p.passable(0, 0));
        assert!(!p.passable(0, 7), "too deep");
        assert!(!p.passable(5, 4), "cliff edge");
        assert!(p.passable(6, 4), "plateau top");
        assert!(p.passable(7, 4), "plateau top");
        for (x, z) in [(2, 2), (4, 4), (3, 2)] {
            assert!(!p.passable(x, z), "rock covers ({x},{z})");
        }
        assert!(p.flags(2, 2) & PassGrid::PAVED != 0);
        assert!(p.passable(1, 1), "unknown feature does not block");
        assert!(p.passable(4, 5));
        assert!(!p.passable(100, 0));

        // No numplayers: slots = markers (0). With numplayers 2 and no
        // markers, slot 2 falls back to its first pre-placed unit.
        let mut ota2 = OtaDef {
            num_players: vec![2],
            ..OtaDef::default()
        };
        ota2.units.push(crate::ota::PlacedUnit {
            unitname: "TSTKNIGHT".into(),
            player: Some(2),
            x: 5,
            y: 0,
            z: 6,
            angle: None,
            health_percentage: None,
            mana_percentage: None,
            kills: None,
            ident: None,
            initial_mission: None,
            ai_ignore: false,
            ai_priority_target: false,
            mission_critical: false,
            extra: BTreeMap::new(),
        });
        let m2 = MapDef::build("kmap/tst", &t, ota2, None, &rs, rule, vec![]);
        assert_eq!(m2.missing_start_slots(), vec![1]);
        assert_eq!(m2.start_positions.len(), 1);
        assert_eq!(m2.start_positions[0].source, StartSource::FirstUnit);
        assert_eq!((m2.start_positions[0].x, m2.start_positions[0].z), (5, 6));
        assert!(m2.dangling.iter().any(|d| d.target == "TSTKNIGHT"));

        let tl = MapDef::build(
            "kmap/tst",
            &t,
            OtaDef::default(),
            None,
            &rs,
            PassRule {
                anchor: FeatureAnchor::TopLeft,
                ..rule
            },
            vec![],
        );
        assert!(tl.passability.passable(2, 2));
        assert!(!tl.passability.passable(4, 5));
        assert_eq!(tl.player_slots, 0);
    }
}
