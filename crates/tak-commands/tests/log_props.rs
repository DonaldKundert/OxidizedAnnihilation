//! Command and log properties: JSON round trips, canonical ordering, and
//! rejection of malformed logs.

use proptest::collection::vec;
use proptest::prelude::*;
use tak_commands::*;
use tak_math::{Fx, Vec2};

fn pos() -> impl Strategy<Value = WorldPos> {
    (any::<i32>(), any::<i32>()).prop_map(|(x, y)| Vec2::new(Fx::from_raw(x), Fx::from_raw(y)))
}

fn unit_id() -> impl Strategy<Value = UnitId> {
    (0u32..64, 0u32..4).prop_map(|(i, g)| UnitId::new(i, g))
}

fn units() -> impl Strategy<Value = UnitSet> {
    vec(unit_id(), 0..8).prop_map(UnitSet::from)
}

fn queue() -> impl Strategy<Value = QueueMode> {
    prop_oneof![Just(QueueMode::Replace), Just(QueueMode::Append)]
}

fn target() -> impl Strategy<Value = Target> {
    prop_oneof![
        pos().prop_map(Target::Ground),
        unit_id().prop_map(Target::Unit)
    ]
}

fn fire_state() -> impl Strategy<Value = FireState> {
    prop_oneof![
        Just(FireState::HoldFire),
        Just(FireState::ReturnFire),
        Just(FireState::FireAtWill)
    ]
}

fn sim_command() -> impl Strategy<Value = SimCommand> {
    prop_oneof![
        (units(), pos(), queue()).prop_map(|(units, to, queue)| SimCommand::Move {
            units,
            to,
            queue
        }),
        (units(), pos(), queue()).prop_map(|(units, to, queue)| SimCommand::AttackMove {
            units,
            to,
            queue
        }),
        (units(), target(), queue()).prop_map(|(units, target, queue)| SimCommand::Attack {
            units,
            target,
            queue
        }),
        (units(), any::<u32>(), proptest::option::of(pos()), queue()).prop_map(
            |(units, d, at, queue)| SimCommand::Build {
                units,
                def: UnitDefId(d),
                at,
                queue
            }
        ),
        (units(), unit_id(), queue()).prop_map(|(units, target, queue)| SimCommand::Guard {
            units,
            target,
            queue
        }),
        (units(), pos(), queue()).prop_map(|(units, to, queue)| SimCommand::Patrol {
            units,
            to,
            queue
        }),
        (
            units(),
            any::<u32>(),
            proptest::option::of(target()),
            queue()
        )
            .prop_map(|(units, s, target, queue)| SimCommand::CastSpell {
                units,
                spell: SpellId(s),
                target,
                queue
            }),
        units().prop_map(|units| SimCommand::Stop { units }),
        (units(), fire_state())
            .prop_map(|(units, state)| SimCommand::SetFireState { units, state }),
    ]
}

fn client_command() -> impl Strategy<Value = ClientCommand> {
    let group = (0u8..10).prop_map(|g| ControlGroup::new(g).unwrap());
    prop_oneof![
        (pos(), any::<bool>())
            .prop_map(|(pos, additive)| ClientCommand::SelectAt { pos, additive }),
        (pos(), any::<i32>(), any::<bool>()).prop_map(|(center, r, additive)| {
            ClientCommand::SelectBrush {
                center,
                radius: Fx::from_raw(r),
                additive,
            }
        }),
        prop_oneof![Just(Scope::OnScreen), Just(Scope::Map)]
            .prop_map(|scope| ClientCommand::SelectSameType { scope }),
        group
            .clone()
            .prop_map(|group| ClientCommand::AssignGroup { group }),
        group.prop_map(|group| ClientCommand::RecallGroup { group }),
        Just(ClientCommand::JumpToAlert),
        Just(ClientCommand::CycleIdleBuilder),
        Just(ClientCommand::CenterOnMonarch),
        Just(ClientCommand::ClearSelection),
    ]
}

/// (tick, player, command) triples, recorded in arbitrary order.
fn issued() -> impl Strategy<Value = Vec<(u32, u8, SimCommand)>> {
    vec((0u32..20, 0u8..4, sim_command()), 0..40)
}

fn build(issued: &[(u32, u8, SimCommand)]) -> CommandLog {
    let mut log = CommandLog::new();
    for (t, p, c) in issued {
        log.record(Tick(*t), PlayerId(*p), c.clone());
    }
    log
}

proptest! {
    #[test]
    fn sim_command_json_round_trip(c in sim_command()) {
        let json = serde_json::to_string(&c).unwrap();
        let back: SimCommand = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(back, c);
    }

    #[test]
    fn client_command_json_round_trip(c in client_command()) {
        let json = serde_json::to_string(&c).unwrap();
        let back: ClientCommand = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(back, c);
    }

    /// Logs survive compact and pretty JSON unchanged.
    #[test]
    fn log_json_round_trip(cmds in issued()) {
        let log = build(&cmds);
        prop_assert_eq!(&CommandLog::from_json(&log.to_json()).unwrap(), &log);
        prop_assert_eq!(&CommandLog::from_json(&log.to_json_pretty()).unwrap(), &log);
        let via_serde: CommandLog = serde_json::from_str(&log.to_json()).unwrap();
        prop_assert_eq!(via_serde, log);
    }

    /// The log is always sorted by (tick, player, seq) with unique keys, and
    /// each player's commands within a tick keep their issue order.
    #[test]
    fn log_is_canonical(cmds in issued()) {
        let log = build(&cmds);
        prop_assert_eq!(log.len(), cmds.len());
        for w in log.entries().windows(2) {
            prop_assert!(w[0].key() < w[1].key());
        }
        for t in 0..20u32 {
            for p in 0..4u8 {
                let issued_order: Vec<&SimCommand> = cmds
                    .iter()
                    .filter(|(ct, cp, _)| *ct == t && *cp == p)
                    .map(|(_, _, c)| c)
                    .collect();
                let logged: Vec<&LoggedCommand> = log
                    .for_tick(Tick(t))
                    .iter()
                    .filter(|e| e.player == PlayerId(p))
                    .collect();
                prop_assert_eq!(logged.len(), issued_order.len());
                for (i, (e, c)) in logged.iter().zip(issued_order).enumerate() {
                    prop_assert_eq!(e.seq, i as u32);
                    prop_assert_eq!(&e.command, c);
                }
            }
        }
    }

    /// Shuffled entries load into the same log (order is canonical), and the
    /// same commands recorded in a different interleaving across players
    /// serialize identically.
    #[test]
    fn loading_sorts_entries(cmds in issued(), seed in any::<u64>()) {
        let log = build(&cmds);
        let mut entries: Vec<LoggedCommand> = log.entries().to_vec();
        // Deterministic shuffle.
        let mut rng = tak_math::Rng::new(seed);
        for i in (1..entries.len()).rev() {
            let j = rng.below(i as u32 + 1) as usize;
            entries.swap(i, j);
        }
        let json = serde_json::json!({ "version": LOG_FORMAT_VERSION, "entries": entries });
        let loaded = CommandLog::from_json(&json.to_string()).unwrap();
        prop_assert_eq!(&loaded, &log);

        let mut rebuilt = CommandLog::new();
        for e in &entries {
            rebuilt.insert(e.clone()).unwrap();
        }
        prop_assert_eq!(rebuilt.to_json(), log.to_json());
    }

    /// UnitSet is sorted and deduplicated however it is built or loaded.
    #[test]
    fn unit_set_normalized(ids in vec(unit_id(), 0..20)) {
        let set: UnitSet = ids.iter().copied().collect();
        prop_assert!(set.as_slice().windows(2).all(|w| w[0] < w[1]));
        for id in &ids {
            prop_assert!(set.contains(*id));
        }
        let json = serde_json::to_string(&ids).unwrap();
        let loaded: UnitSet = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(loaded, set);
    }
}

#[test]
fn duplicate_keys_rejected() {
    let cmd = SimCommand::Stop {
        units: UnitSet::single(UnitId::new(1, 0)),
    };
    let entry = LoggedCommand {
        tick: Tick(5),
        player: PlayerId(1),
        seq: 0,
        command: cmd.clone(),
    };
    let mut log = CommandLog::new();
    log.insert(entry.clone()).unwrap();
    assert!(matches!(
        log.insert(entry.clone()),
        Err(LogError::Duplicate { .. })
    ));

    let json = serde_json::json!({ "version": 1, "entries": [entry.clone(), entry] });
    assert!(matches!(
        CommandLog::from_json(&json.to_string()),
        Err(LogError::Duplicate { .. })
    ));
    // Through plain serde, the same check surfaces as a JSON error.
    assert!(serde_json::from_str::<CommandLog>(&json.to_string()).is_err());
}

#[test]
fn bad_version_and_group_rejected() {
    assert!(matches!(
        CommandLog::from_json(r#"{"version": 99, "entries": []}"#),
        Err(LogError::UnsupportedVersion(99))
    ));
    assert!(
        serde_json::from_str::<ClientCommand>(r#"{"type":"recall_group","group":10}"#).is_err()
    );
    assert!(ControlGroup::new(10).is_none());
}

/// Pins the wire format, so accidental serde changes show up in review.
#[test]
fn json_shape() {
    let mut log = CommandLog::new();
    log.record(
        Tick(3),
        PlayerId(0),
        SimCommand::Move {
            units: UnitSet::from(vec![UnitId::new(2, 1), UnitId::new(1, 0)]),
            to: Vec2::from_ints(1, 2),
            queue: QueueMode::Append,
        },
    );
    assert_eq!(
        log.to_json(),
        r#"{"version":1,"entries":[{"tick":3,"player":0,"seq":0,"command":{"type":"move","units":[{"index":1,"generation":0},{"index":2,"generation":1}],"to":{"x":65536,"y":131072},"queue":"append"}}]}"#
    );
    assert_eq!(log.for_tick(Tick(3)).len(), 1);
    assert!(log.for_tick(Tick(2)).is_empty());
    assert_eq!(log.last_tick(), Some(Tick(3)));
}
