//! `tak-cli content`: load a ruleset (and optionally a map) through
//! `tak-content` and print a summary, one def as JSON, or key coverage.
//!
//! Output can contain retail text (display names); do not commit it. The
//! `--coverage` table holds only key names and counts.

use serde::Serialize;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::Write as _;
use tak_content::{
    Coverage, CrtSummary, DanglingRef, MapDef, OtaDef, PassGrid, PassRule, Ruleset, RulesetKind,
    StartPosition,
};
use tak_vfs::Vfs;

#[derive(clap::Args)]
pub struct ContentArgs {
    /// Balance set: `base` (units/, canbuild/) or `cb` (unitscb/, canbuildcb/).
    #[arg(long, default_value = "base")]
    ruleset: RulesetKind,
    /// Print one unit as JSON (by unitname, case-insensitive).
    #[arg(long)]
    unit: Option<String>,
    /// Print one feature as JSON (by name, case-insensitive).
    #[arg(long)]
    feature: Option<String>,
    /// Print a map summary as JSON (VFS path without extension, e.g. "kmap/Adamantine Gate").
    #[arg(long)]
    map: Option<String>,
    /// Print mapped vs unmapped keys per section path as Markdown.
    #[arg(long)]
    coverage: bool,
}

/// A map without its per-cell planes.
#[derive(Serialize)]
struct MapSummary<'a> {
    name: &'a str,
    width: u32,
    height: u32,
    sea_level: u32,
    kingdom: &'a Option<String>,
    palette: &'a Option<String>,
    player_slots: u8,
    start_positions: &'a [StartPosition],
    features_placed: usize,
    features_by_name: BTreeMap<&'a str, usize>,
    paved_cells: u32,
    marker_cells: u32,
    pass_rule: PassRule,
    cells: usize,
    passable_cells: usize,
    water_cells: usize,
    deep_cells: usize,
    steep_cells: usize,
    feature_cells: usize,
    crt: &'a Option<CrtSummary>,
    dangling: &'a [DanglingRef],
    ota: &'a OtaDef,
}

pub fn run(vfs: &Vfs, args: ContentArgs) -> Result<(), Box<dyn Error>> {
    let rs = Ruleset::load(vfs, args.ruleset);
    if let Some(u) = &args.unit {
        let def = rs.unit(u).ok_or_else(|| format!("no unit {u:?}"))?;
        println!("{}", serde_json::to_string_pretty(def)?);
        return Ok(());
    }
    if let Some(f) = &args.feature {
        let def = rs.feature(f).ok_or_else(|| format!("no feature {f:?}"))?;
        println!("{}", serde_json::to_string_pretty(def)?);
        return Ok(());
    }
    if let Some(name) = &args.map {
        let m = MapDef::load(vfs, &rs, name)?;
        let p = &m.passability;
        let mut by_name = BTreeMap::new();
        for f in &m.features {
            *by_name.entry(f.name.as_str()).or_default() += 1;
        }
        let s = MapSummary {
            name: &m.name,
            width: m.width,
            height: m.height,
            sea_level: m.sea_level,
            kingdom: &m.kingdom,
            palette: &m.palette,
            player_slots: m.player_slots,
            start_positions: &m.start_positions,
            features_placed: m.features.len(),
            features_by_name: by_name,
            paved_cells: m.paved_cells,
            marker_cells: m.marker_cells,
            pass_rule: p.rule,
            cells: p.cells.len(),
            passable_cells: p.cells.len() - p.count(PassGrid::BLOCKS),
            water_cells: p.count(PassGrid::WATER),
            deep_cells: p.count(PassGrid::DEEP),
            steep_cells: p.count(PassGrid::STEEP),
            feature_cells: p.count(PassGrid::FEATURE),
            crt: &m.crt,
            dangling: &m.dangling,
            ota: &m.ota,
        };
        println!("{}", serde_json::to_string_pretty(&s)?);
        return Ok(());
    }
    if args.coverage {
        let mut cov = rs.coverage.clone();
        for name in tak_content::playable_maps(vfs) {
            if let Ok(m) = MapDef::load(vfs, &rs, &name) {
                cov.merge(&m.coverage);
            }
        }
        print!("{}", coverage_markdown(&cov));
        return Ok(());
    }
    println!("ruleset {}", rs.kind.name());
    for (k, n) in &rs.files {
        println!("  files {k}: {n}");
    }
    println!("  units: {}", rs.units.len());
    println!(
        "  weapons: {}",
        rs.units.values().map(|u| u.weapons.len()).sum::<usize>()
    );
    println!("  features: {}", rs.features.len());
    println!("  build menus: {}", rs.build_menus.len());
    println!("  sides: {}", rs.sides.len());
    println!("  movement classes: {}", rs.move_classes.len());
    println!("  overrides: {}", rs.overrides.len());
    println!("  issues: {}", rs.issues.len());
    for i in &rs.issues {
        println!("    {i}");
    }
    println!("  dangling references: {}", rs.dangling.len());
    for d in &rs.dangling {
        println!("    {} {} -> {:?} {}", d.from, d.field, d.kind, d.target);
    }
    Ok(())
}

fn coverage_markdown(cov: &Coverage) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "| Section path | Sections | Keys mapped | Keys unmapped | Unmapped keys |"
    );
    let _ = writeln!(s, "|---|--:|--:|--:|---|");
    for (path, c) in &cov.sections {
        let unmapped: Vec<String> = c.unmapped.keys().map(|k| format!("`{k}`")).collect();
        let _ = writeln!(
            s,
            "| `{path}` | {} | {} | {} | {} |",
            c.sections,
            c.mapped.len(),
            c.unmapped.len(),
            if unmapped.len() > 40 {
                format!("{} keys (data, not schema)", unmapped.len())
            } else {
                unmapped.join(", ")
            }
        );
    }
    s
}
