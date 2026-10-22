//! HUD logic that needs no Bevy (WP-V, design system section 6): which
//! race skin to use, where the order buttons sit on the skin's button
//! panel, the multi-selection strip's type groups, and the bottom bar's
//! verb line. Drawing is in `app.rs` / `hud_view.rs`.

use tak_commands::{UnitDefId, UnitId};

/// The in-game skin GAF (`anims/<name>.gaf`, palette `anims/<name>.pcx`)
/// for a side, from the monarch's unit name prefix (**Known**: the five
/// files exist; Veruna's entries carry a `Veruna`/`V` prefix, so entries
/// are taken by index: 0 ButtonPanel, 1 InfoPanel, 2 EndCap, 3 UnitInfo,
/// 4 UnitPic).
pub fn race_skin(monarch_unitname: &str) -> &'static str {
    let up = monarch_unitname.to_ascii_uppercase();
    match up.get(..3).unwrap_or("") {
        "VER" => "verunaig",
        "TAR" => "tarosig",
        "ZON" => "zhonig",
        "CRE" => "creonig",
        _ => "aramonig",
    }
}

/// Skin GAF entry indices.
pub const SKIN_BUTTON_PANEL: usize = 0;
/// See [`SKIN_BUTTON_PANEL`].
pub const SKIN_INFO_PANEL: usize = 1;
/// See [`SKIN_BUTTON_PANEL`].
pub const SKIN_END_CAP: usize = 2;

/// Button slots on Aramon's button panel (frame 1), `[x, y, w, h]` of the
/// dark wells, when detection fails (**Known** for `aramonig.gaf`).
pub const ARAMON_SLOTS: [[u32; 4]; 13] = [
    [18, 26, 27, 27],
    [88, 26, 27, 27],
    [18, 61, 27, 27],
    [88, 61, 27, 27],
    [18, 96, 27, 27],
    [88, 96, 27, 27],
    [53, 131, 27, 27],
    [18, 181, 27, 27],
    [53, 181, 27, 27],
    [88, 181, 27, 27],
    [18, 222, 27, 27],
    [53, 222, 27, 27],
    [88, 222, 27, 27],
];

/// The dark button wells on a skin's button panel (RGBA8, `w`×`h`): every
/// connected region of near-black opaque pixels whose bounding box is
/// 22..=34 px on both sides and at least a fifth dark, sorted top to bottom, then
/// left to right. Each race paints them a few pixels apart, so the HUD
/// finds them rather than assuming Aramon's.
pub fn button_slots(rgba: &[u8], w: u32, h: u32) -> Vec<[u32; 4]> {
    let (w, h) = (w as usize, h as usize);
    if rgba.len() < w * h * 4 {
        return Vec::new();
    }
    let dark = |i: usize| {
        let p = &rgba[i * 4..i * 4 + 4];
        p[3] > 0 && u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]) < 60
    };
    let mut seen = vec![false; w * h];
    let mut out = Vec::new();
    let mut stack = Vec::new();
    for start in 0..w * h {
        if seen[start] || !dark(start) {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let (mut x0, mut y0, mut x1, mut y1, mut n) = (w, h, 0, 0, 0usize);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            n += 1;
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
            let mut push = |j: usize| {
                if !seen[j] && dark(j) {
                    seen[j] = true;
                    stack.push(j);
                }
            };
            if x > 0 {
                push(i - 1);
            }
            if x + 1 < w {
                push(i + 1);
            }
            if y > 0 {
                push(i - w);
            }
            if y + 1 < h {
                push(i + w);
            }
        }
        let (bw, bh) = (x1 + 1 - x0, y1 + 1 - y0);
        // The wells have a lit bevel inside: about a third of the box is
        // near-black.
        if (22..=34).contains(&bw) && (22..=34).contains(&bh) && n * 10 >= bw * bh * 2 {
            out.push([x0 as u32, y0 as u32, bw as u32, bh as u32]);
        }
    }
    // Rows: wells within 8 px vertically are one row.
    out.sort_by_key(|s| (s[1] / 8, s[0]));
    out
}

/// What each of the panel's 13 wells holds, in [`button_slots`] order:
/// three rows of two orders and a centre one, the three stances, the three
/// weapon (spell) buttons. Entries are `igcommonbuttons.gaf` sequences.
/// **Chosen** arrangement (the GUI file creates these buttons in code).
pub const PANEL_BUTTONS: [&str; 13] = [
    "AttackButton",
    "MoveButton",
    "GuardButton",
    "PatrolButton",
    "StopButton",
    "ClearButton",
    "RepairButton",
    "AgOffensiveButton",
    "AgDefensiveButton",
    "AgPassiveButton",
    "WPrimaryButton",
    "WSecondaryButton",
    "WSpecialButton",
];

/// The selection grouped by unit type for the multi-selection strip: each
/// type with its units, in order of the first unit (by id).
pub fn group_by_type(units: &[(UnitId, UnitDefId)]) -> Vec<(UnitDefId, Vec<UnitId>)> {
    let mut sorted = units.to_vec();
    sorted.sort_by_key(|(id, _)| *id);
    let mut out: Vec<(UnitDefId, Vec<UnitId>)> = Vec::new();
    for (id, def) in sorted {
        match out.iter_mut().find(|(d, _)| *d == def) {
            Some((_, v)) => v.push(id),
            None => out.push((def, vec![id])),
        }
    }
    out
}

/// Damage bars (TA ~): which units show a health bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DamageBars {
    /// Selected, damaged and unfinished units (the default).
    All,
    /// Selected units only.
    Selected,
    /// None.
    Off,
}

impl DamageBars {
    /// From the cycle counter (0, 1, 2).
    pub fn from_cycle(n: u8) -> DamageBars {
        match n % 3 {
            0 => DamageBars::All,
            1 => DamageBars::Selected,
            _ => DamageBars::Off,
        }
    }

    /// Show a bar for a unit?
    pub fn show(self, selected: bool, damaged: bool, building: bool) -> bool {
        match self {
            DamageBars::All => selected || damaged || building,
            DamageBars::Selected => selected,
            DamageBars::Off => false,
        }
    }

    /// Bottom-bar text when toggled.
    pub fn label(self) -> &'static str {
        match self {
            DamageBars::All => "Damage bars: all",
            DamageBars::Selected => "Damage bars: selected",
            DamageBars::Off => "Damage bars: off",
        }
    }
}

/// The bottom bar's verb line: what the confirm input does here, e.g.
/// "X: Attack Zhon Warrior" or "RMB: Move". `prompt` is the input's name,
/// `verb` the order, `target` what is under the cursor.
pub fn verb_line(prompt: &str, verb: &str, target: Option<&str>) -> String {
    match target {
        Some(t) if !t.is_empty() => format!("{prompt}: {verb} {t}"),
        _ => format!("{prompt}: {verb}"),
    }
}

/// Health bar colour (green, then yellow, then red), RGB 0..1.
pub fn health_rgb(frac: f32) -> [f32; 3] {
    let f = frac.clamp(0.0, 1.0);
    if f > 0.6 {
        [0.2, 0.85, 0.2]
    } else if f > 0.3 {
        [0.95, 0.8, 0.15]
    } else {
        [0.9, 0.2, 0.15]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skins_by_side() {
        assert_eq!(race_skin("ARAKING"), "aramonig");
        assert_eq!(race_skin("verking"), "verunaig");
        assert_eq!(race_skin("TARKING"), "tarosig");
        assert_eq!(race_skin("ZONKING"), "zhonig");
        assert_eq!(race_skin("CREKING"), "creonig");
        assert_eq!(race_skin(""), "aramonig");
    }

    #[test]
    fn wells_are_found_on_a_synthetic_panel() {
        // A 128x120 tan panel with three dark 27x27 wells (one row of two,
        // one centred below) and a big dark box that is not a well.
        let (w, h) = (128u32, 120u32);
        let mut px = vec![0u8; (w * h * 4) as usize];
        for i in 0..(w * h) as usize {
            px[i * 4..i * 4 + 4].copy_from_slice(&[150, 120, 80, 255]);
        }
        let mut fill = |x0: u32, y0: u32, bw: u32, bh: u32| {
            for y in y0..y0 + bh {
                for x in x0..x0 + bw {
                    let i = ((y * w + x) * 4) as usize;
                    px[i..i + 4].copy_from_slice(&[10, 10, 12, 255]);
                }
            }
        };
        fill(88, 20, 27, 27);
        fill(18, 21, 27, 27);
        fill(53, 60, 27, 27);
        fill(10, 95, 100, 20);
        let s = button_slots(&px, w, h);
        assert_eq!(
            s,
            vec![[18, 21, 27, 27], [88, 20, 27, 27], [53, 60, 27, 27]]
        );
        assert!(button_slots(&px[..10], w, h).is_empty());
    }

    #[test]
    fn strip_groups_types_in_id_order() {
        let u = |i| UnitId::new(i, 0);
        let g = group_by_type(&[
            (u(5), UnitDefId(2)),
            (u(1), UnitDefId(7)),
            (u(3), UnitDefId(2)),
        ]);
        assert_eq!(
            g,
            vec![(UnitDefId(7), vec![u(1)]), (UnitDefId(2), vec![u(3), u(5)])]
        );
    }

    #[test]
    fn damage_bars_cycle() {
        assert_eq!(DamageBars::from_cycle(0), DamageBars::All);
        assert!(DamageBars::All.show(false, true, false));
        assert!(!DamageBars::Selected.show(false, true, false));
        assert!(DamageBars::Selected.show(true, false, false));
        assert!(!DamageBars::from_cycle(2).show(true, true, true));
        assert_eq!(verb_line("X", "Attack", Some("Zhon")), "X: Attack Zhon");
        assert_eq!(verb_line("RMB", "Move", None), "RMB: Move");
    }
}
