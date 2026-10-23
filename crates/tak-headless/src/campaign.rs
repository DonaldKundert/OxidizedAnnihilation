//! Explicit campaign setup for headless tools, not campaign progression.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use tak_sim::campaign::{CampaignMatch, CampaignPlayer, enable_objectives};
use tak_sim::content::ContentError;
use tak_sim::victory::{Condition, UnitType};

use crate::Error;

/// Parse one-based OTA owner slots and sides in simulation player order.
/// No side or ownership is inferred from undocumented mission headers.
pub fn parse_players(text: &str) -> Result<Vec<CampaignPlayer>, Error> {
    let mut players = Vec::new();
    let mut slots = BTreeSet::new();
    for entry in text.split(',') {
        let (slot, side) = entry.trim().split_once(':').ok_or_else(|| {
            Error::Setup("--campaign-players expects OTA_SLOT:SIDE pairs, e.g. 1:ARA,2:ZON".into())
        })?;
        let slot = slot
            .trim()
            .parse::<u8>()
            .ok()
            .filter(|s| *s != 0)
            .ok_or_else(|| Error::Setup("campaign OTA owner slots must be in 1..=255".into()))?;
        let side = side.trim();
        if side.is_empty() || side.contains(':') || side.chars().any(char::is_control) {
            return Err(Error::Setup(
                "each campaign owner needs one nonempty side".into(),
            ));
        }
        if !slots.insert(slot) {
            return Err(Error::Setup(format!(
                "duplicate campaign OTA owner slot {slot}"
            )));
        }
        players.push(CampaignPlayer {
            ota_slot: slot,
            side: side.into(),
        });
        if players.len() > tak_sim::scenario::MAX_PLAYERS {
            return Err(Error::Setup("too many campaign players".into()));
        }
    }
    Ok(players)
}

/// Load authored placements, with objectives off unless explicitly requested.
/// The CLI's objective mode is strict: recognized unsupported/malformed
/// conditions or unresolved unit types error instead of being dropped.
pub fn load_campaign_map(
    data: &Path,
    map: &str,
    players: &[CampaignPlayer],
    seed: u64,
    objectives: bool,
) -> Result<CampaignMatch, Error> {
    if objectives && (players.len() != 2 || players[0].ota_slot != 1 || players[1].ota_slot != 2) {
        return Err(Error::Setup(
            "--campaign-objectives currently requires exactly OTA slots 1,2 in that order".into(),
        ));
    }
    let vfs = tak_vfs::open_kingdoms(data).map_err(ContentError::from)?;
    let mut loaded = tak_sim::campaign::load_campaign(&vfs, map, players, seed)?;
    if objectives {
        enable_objectives(&mut loaded.game.scenario, &loaded.game.map, players)?;
        let rules = loaded
            .game
            .scenario
            .victory
            .mission
            .as_ref()
            .expect("enabled above");
        if !rules.unsupported.is_empty() {
            let keys: Vec<_> = rules
                .unsupported
                .iter()
                .map(|s| s.split('=').next().unwrap_or(s))
                .collect();
            return Err(Error::Setup(format!(
                "campaign objectives contain unsupported or malformed conditions: {}",
                keys.join(", ")
            )));
        }
        for condition in rules.victory.iter().chain(&rules.defeat) {
            if let Some(ty) = condition_unit_type(condition).filter(|ty| ty.def.is_none()) {
                return Err(Error::Setup(format!(
                    "campaign objective unit type {:?} is absent from the loaded definitions",
                    ty.name
                )));
            }
        }
    }
    Ok(loaded)
}

fn condition_unit_type(condition: &Condition) -> Option<&UnitType> {
    match condition {
        Condition::KillAllOfType(ty)
        | Condition::KillUnitType(ty, _)
        | Condition::BuildUnitType(ty)
        | Condition::AllUnitsKilledOfType(ty)
        | Condition::UnitTypeKilled(ty, _) => Some(ty),
        Condition::MoveUnitToRadius { ty, .. }
        | Condition::UnitTypePassesX(ty, _)
        | Condition::UnitTypePassesZ(ty, _) => ty.as_ref(),
        _ => None,
    }
}

/// Concise, deterministic diagnostics, kept separate from hash/log stdout.
pub fn report(loaded: &CampaignMatch, players: &[CampaignPlayer], objectives: bool) -> String {
    let count = loaded
        .game
        .scenario
        .initial_units
        .as_ref()
        .map_or(0, Vec::len);
    let mut report = format!(
        "campaign: {count} authored units, {} mapped owners; placement loading only, not full campaign support\n",
        players.len()
    );
    for (i, player) in players.iter().enumerate() {
        report.push_str(&format!(
            "campaign player {i}: OTA owner {}, side {}\n",
            player.ota_slot, player.side
        ));
    }
    let mut fields = BTreeMap::<&str, usize>::new();
    for warning in &loaded.unsupported {
        match warning.split_once(": ") {
            Some((unit, field))
                if unit
                    .strip_prefix("unit ")
                    .is_some_and(|index| index.parse::<usize>().is_ok()) =>
            {
                *fields.entry(field).or_default() += 1;
            }
            _ => report.push_str(&format!("warning: {warning}\n")),
        }
    }
    for (field, count) in fields {
        report.push_str(&format!(
            "warning: campaign ignores {field} on {count} placement(s)\n"
        ));
    }
    report.push_str("warning: different owners are treated as enemies; mission AI, alliances and neutrality are not implemented\n");
    if objectives {
        report.push_str("campaign objectives: existing two-player condition evaluator enabled; no scenario trigger interpreter\n");
    } else {
        report.push_str(
            "warning: authored objectives are disabled; existing skirmish defeat rules apply\n",
        );
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_mapping_is_explicit_sparse_and_ordered() {
        let players = parse_players(" 10 : ZHON, 1: Aramon ").unwrap();
        assert_eq!(
            (players[0].ota_slot, players[0].side.as_str()),
            (10, "ZHON")
        );
        assert_eq!(
            (players[1].ota_slot, players[1].side.as_str()),
            (1, "Aramon")
        );
    }

    #[test]
    fn malformed_or_duplicate_owner_mapping_is_rejected() {
        for text in [
            "",
            "ARA,ZON",
            "0:ARA",
            "256:ARA",
            "-1:ARA",
            "1:",
            "1:ARA,",
            ",1:ARA",
            "1:ARA,1:ZON",
            "1:ARA:ZON",
            "1:AR\nA",
        ] {
            assert!(parse_players(text).is_err(), "{text:?}");
        }
        let too_many = (1..=17)
            .map(|i| format!("{i}:ARA"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(parse_players(&too_many).is_err());
    }
}
