//! Retail corpus checks, gated on `TAK_DATA_DIR`. They assert counts and
//! zero errors and print distributions; they never snapshot file contents.

use std::collections::BTreeMap;
use std::sync::Arc;

use tak_formats::model3do;
use tak_script::{CallOutcome, HeadlessEngine, Instance, Program, VmError};

/// Reads one `key = value;` from FBI text (case-insensitive key).
fn fbi_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (k, v) = line.trim().split_once('=')?;
        k.trim()
            .eq_ignore_ascii_case(key)
            .then(|| v.trim().trim_end_matches(';').trim().to_string())
    })
}

fn stem(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.split('.').next().unwrap_or(name).to_ascii_lowercase()
}

#[test]
fn every_unit_script_runs_create_for_300_ticks() {
    let Some(dir) = tak_vfs::testing::data_dir_or_skip("every_unit_script_runs_create") else {
        return;
    };
    let vfs = tak_vfs::open_kingdoms(&dir).expect("mount");
    let scripts = vfs.list("scripts/");
    let mut ran = 0;
    let mut errors = Vec::new();
    let mut reads: BTreeMap<i32, u64> = BTreeMap::new();
    let mut writes: BTreeMap<i32, u64> = BTreeMap::new();
    let mut threads_alive = 0;
    let mut animated = 0;
    for f in &scripts {
        if !f.path.ends_with(".cob") {
            continue;
        }
        let bytes = vfs.read(&f.path).expect("read");
        let program = Arc::new(Program::from_bytes(&bytes).expect("parse"));
        let mut inst = Instance::new(program);
        let mut eng = HeadlessEngine::new(1);
        inst.start("Create", &[]).expect("Create");
        let h0 = inst.anim_hash();
        for _ in 0..300 {
            inst.tick(&mut eng);
        }
        if inst.anim_hash() != h0 {
            animated += 1;
        }
        threads_alive += inst.thread_count();
        for e in inst.errors() {
            let func = &inst.program().script().functions[e.function as usize].name;
            errors.push(format!(
                "{}: {} @{} tick {}: {}",
                f.path, func, e.pc, e.tick, e.error
            ));
        }
        for (p, n) in &eng.unknown_reads {
            *reads.entry(*p).or_default() += n;
        }
        for (p, n) in &eng.unknown_writes {
            *writes.entry(*p).or_default() += n;
        }
        ran += 1;
    }
    eprintln!(
        "ran Create x300 ticks on {ran} scripts: {} errors, {threads_alive} threads alive at end, {animated} changed pose",
        errors.len()
    );
    eprintln!("unknown port reads: {reads:?}");
    eprintln!("unknown port writes: {writes:?}");
    for e in errors.iter().take(40) {
        eprintln!("  {e}");
    }
    assert_eq!(ran, 204);
    assert!(errors.is_empty(), "{} VM errors", errors.len());
}

/// The COB piece table is a list of names, not the model's pre-order: the
/// VM must resolve pieces by name. Checks every name resolves.
#[test]
fn cob_pieces_resolve_by_name_in_models() {
    let Some(dir) = tak_vfs::testing::data_dir_or_skip("cob_pieces_resolve_by_name") else {
        return;
    };
    let vfs = tak_vfs::open_kingdoms(&dir).expect("mount");
    let mut units = 0;
    let mut preorder = 0;
    let mut prefix_of_preorder = 0;
    let mut missing = Vec::new();
    let mut names_checked = 0;
    for f in vfs.list("units/") {
        if !f.path.ends_with(".fbi") {
            continue;
        }
        let text = String::from_utf8_lossy(&vfs.read(&f.path).expect("read")).into_owned();
        let unit = fbi_value(&text, "unitname").unwrap_or_else(|| stem(&f.path));
        let Some(object) = fbi_value(&text, "objectname") else {
            continue;
        };
        let cob_path = format!("scripts/{}.cob", unit.to_ascii_lowercase());
        let model_path = format!("objects3d/{}.3do", object.to_ascii_lowercase());
        let (Ok(cob), Ok(m)) = (vfs.read(&cob_path), vfs.read(&model_path)) else {
            continue;
        };
        let script = tak_formats::cob::parse(&cob).expect("cob");
        let model = model3do::parse(&m).expect("3do");
        units += 1;
        let same = script.pieces.len() == model.pieces.len()
            && script
                .pieces
                .iter()
                .zip(&model.pieces)
                .all(|(a, b)| a.eq_ignore_ascii_case(&b.name));
        if same {
            preorder += 1;
        }
        if script
            .pieces
            .iter()
            .zip(&model.pieces)
            .all(|(a, b)| a.eq_ignore_ascii_case(&b.name))
        {
            prefix_of_preorder += 1;
        }
        for p in &script.pieces {
            names_checked += 1;
            if model.find(p).is_none() {
                missing.push(format!("{cob_path}: {p} not in {model_path}"));
            }
        }
    }
    eprintln!(
        "{units} unit/model pairs; COB order == 3DO pre-order in {preorder}, a prefix of it in {prefix_of_preorder}; {names_checked} names, {} unresolved",
        missing.len()
    );
    for m in missing.iter().take(40) {
        eprintln!("  {m}");
    }
    assert!(units >= 190, "only {units} pairs found");
}

/// Engine callbacks with the argument counts the prologues suggest. Arguments
/// are zero; the point is that nothing errors.
const CALLBACKS: &[(&str, usize)] = &[
    ("SetMaxReloadTime", 1),
    ("QueryWeapon", 1),
    ("AimWeapon", 2),
    ("FireWeapon", 0),
    ("TargetCleared", 1),
    ("SweetSpot", 1),
    ("Sweetspot", 1),
    ("QueryBlood", 1),
    ("TurnDirection", 1),
    ("MoveRate", 1),
    ("setSFXoccupy", 1),
    ("QueryBuildInfo", 1),
    ("HitByWeapon", 2),
    ("Activate", 0),
    ("StartBuilding", 0),
    ("StopBuilding", 0),
    ("Deactivate", 0),
];

/// Moving units, weapon callbacks, then death: a wider sweep than `Create`.
#[test]
fn every_unit_script_survives_walk_callbacks_and_death() {
    let Some(dir) = tak_vfs::testing::data_dir_or_skip("every_unit_script_survives_callbacks")
    else {
        return;
    };
    let vfs = tak_vfs::open_kingdoms(&dir).expect("mount");
    let mut ran = 0;
    let mut calls = 0;
    let mut blocked = 0;
    let mut errors = Vec::new();
    let mut runaway = Vec::new();
    let mut reads: BTreeMap<i32, u64> = BTreeMap::new();
    let mut writes: BTreeMap<i32, u64> = BTreeMap::new();
    for f in vfs.list("scripts/") {
        if !f.path.ends_with(".cob") {
            continue;
        }
        let bytes = vfs.read(&f.path).expect("read");
        let program = Arc::new(Program::from_bytes(&bytes).expect("parse"));
        let mut inst = Instance::new(program);
        let mut eng = HeadlessEngine::new(2);
        inst.start("Create", &[]).expect("Create");
        for _ in 0..30 {
            inst.tick(&mut eng);
        }
        // Port 29 > 5 is what MoveWatcher polls for "moving" (cob-vm.md).
        eng.pin(29, 10);
        for _ in 0..150 {
            inst.tick(&mut eng);
        }
        for &(name, argc) in CALLBACKS {
            if inst.program().function(name).is_none() {
                continue;
            }
            calls += 1;
            match inst.call(&mut eng, name, &vec![0; argc]).expect("call") {
                CallOutcome::Blocked(_) => blocked += 1,
                CallOutcome::Failed(e) => errors.push(format!("{}: {name}: {e}", f.path)),
                CallOutcome::Returned { .. } => {}
            }
            for _ in 0..5 {
                inst.tick(&mut eng);
            }
        }
        eng.pin(29, 0);
        if inst.program().function("Killed").is_some() {
            let _ = inst.call(&mut eng, "Killed", &[50, 0]).expect("Killed");
        }
        for _ in 0..150 {
            inst.tick(&mut eng);
        }
        for e in inst.errors() {
            let func = &inst.program().script().functions[e.function as usize].name;
            if matches!(e.error, VmError::InstructionLimit(_)) {
                // Faked inputs can reach states the real engine may never
                // produce (cob-vm.md, "Runaway loops"); the cap must catch them.
                runaway.push(format!("{}: {func}", f.path));
            } else {
                errors.push(format!("{}: {func} @{}: {}", f.path, e.pc, e.error));
            }
        }
        for (p, n) in &eng.unknown_reads {
            *reads.entry(*p).or_default() += n;
        }
        for (p, n) in &eng.unknown_writes {
            *writes.entry(*p).or_default() += n;
        }
        ran += 1;
    }
    eprintln!("sweep unknown port reads: {reads:?}");
    eprintln!("sweep unknown port writes: {writes:?}");
    eprintln!(
        "{ran} scripts, {calls} callback calls ({blocked} blocked into threads), {} errors, {} runaway threads stopped by the instruction cap: {runaway:?}",
        errors.len(),
        runaway.len()
    );
    for e in errors.iter().take(40) {
        eprintln!("  {e}");
    }
    assert_eq!(ran, 204);
    assert!(errors.is_empty(), "{} VM errors", errors.len());
}
