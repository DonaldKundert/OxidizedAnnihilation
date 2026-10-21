//! VM tests on hand-built synthetic scripts (no game data).

use std::sync::Arc;

use tak_formats::cob::{self, Function, Op, Script};
use tak_script::{
    CallOutcome, Effect, HeadlessEngine, Instance, Limits, Program, StartError, VmError,
    sleep_ticks,
};

/// A tiny assembler over `cob::encode`.
#[derive(Default)]
struct Asm {
    code: Vec<u32>,
    funcs: Vec<Function>,
    pieces: Vec<String>,
    sounds: Vec<String>,
    statics: u32,
}

impl Asm {
    fn new(pieces: &[&str], statics: u32) -> Asm {
        Asm {
            pieces: pieces.iter().map(|s| s.to_string()).collect(),
            statics,
            ..Asm::default()
        }
    }
    fn func(&mut self, name: &str) -> &mut Self {
        self.funcs.push(Function {
            name: name.into(),
            entry: self.code.len() as u32,
        });
        self
    }
    fn op(&mut self, op: Op, args: &[u32]) -> &mut Self {
        cob::encode(&mut self.code, op, args);
        self
    }
    fn push(&mut self, v: i32) -> &mut Self {
        self.op(Op::PushConst, &[v as u32])
    }
    /// `alloc-local; return`, the retail epilogue.
    fn end(&mut self) -> &mut Self {
        self.op(Op::AllocLocal, &[]).op(Op::Return, &[])
    }
    fn here(&self) -> u32 {
        self.code.len() as u32
    }
    /// Emits a jump with a placeholder target; returns the operand slot.
    fn jump_fwd(&mut self, op: Op) -> usize {
        self.op(op, &[0]);
        self.code.len() - 1
    }
    fn patch(&mut self, slot: usize) {
        self.code[slot] = self.here();
    }
    fn program(&self) -> Arc<Program> {
        let script = Script {
            // Recomputed by `to_bytes`.
            header: empty_script().header,
            functions: self.funcs.clone(),
            pieces: self.pieces.clone(),
            sounds: self.sounds.clone(),
            static_count: self.statics,
            code: self.code.clone(),
        };
        // Round-trip through the file format so tests also cover parsing.
        Arc::new(Program::from_bytes(&script.to_bytes()).expect("synthetic script parses"))
    }
    fn instance(&self) -> Instance {
        Instance::new(self.program())
    }
}

fn empty_script() -> Script {
    let mut code = Vec::new();
    cob::encode(&mut code, Op::AllocLocal, &[]);
    cob::encode(&mut code, Op::Return, &[]);
    Script {
        header: cob::Header {
            version: 6,
            script_count: 0,
            piece_count: 0,
            code_words: 0,
            static_count: 0,
            trailing_record_count: 0,
            script_entries_at: 0,
            script_names_at: 0,
            piece_names_at: 0,
            code_at: 0,
            trailing_records_at: 0,
            sound_names_at: 0,
            sound_count: 0,
        },
        functions: vec![Function {
            name: "f".into(),
            entry: 0,
        }],
        pieces: vec![],
        sounds: vec![],
        static_count: 0,
        code,
    }
}

fn eng() -> HeadlessEngine {
    HeadlessEngine::new(42)
}

/// Runs `Main` once via `call` and returns statics.
fn run_main(a: &Asm) -> (Instance, HeadlessEngine) {
    let mut inst = a.instance();
    let mut e = eng();
    let out = inst.call(&mut e, "Main", &[]).unwrap();
    assert!(
        matches!(out, CallOutcome::Returned { .. }),
        "{out:?} {:?}",
        inst.errors()
    );
    (inst, e)
}

#[test]
fn arithmetic_comparison_and_logic() {
    let cases: &[(Op, i32, i32, i32)] = &[
        (Op::Add, 7, -3, 4),
        (Op::Add, i32::MAX, 1, i32::MIN),
        (Op::Sub, 7, 10, -3),
        (Op::Mul, -6, 7, -42),
        (Op::Div, -7, 2, -3),
        (Op::Div, i32::MIN, -1, i32::MIN),
        (Op::Mod, 10, 4, 2),
        (Op::BitAnd, 0b1100, 0b1010, 0b1000),
        (Op::BitOr, 0b1100, 0b1010, 0b1110),
        (Op::BitXor, 0b1100, 0b1010, 0b0110),
        (Op::ShiftLeft, 3, 4, 48),
        (Op::ShiftRight, 0xF0, 4, 0xF),
        (Op::ShiftRight, -16, 2, -4),
        (Op::ShiftLeft, 1, 40, 0),
        (Op::Lt, 1, 2, 1),
        (Op::Lt, 2, 2, 0),
        (Op::Le, 2, 2, 1),
        (Op::Gt, 3, 2, 1),
        (Op::Gt, -3, 2, 0),
        (Op::Ge, 2, 2, 1),
        (Op::Eq, 5, 5, 1),
        (Op::Ne, 5, 5, 0),
        (Op::And, 5, 0, 0),
        (Op::And, 5, -1, 1),
        (Op::Or, 0, 0, 0),
        (Op::Or, 0, 9, 1),
        (Op::Xor, 3, 0, 1),
        (Op::Xor, 3, 4, 0),
    ];
    let mut a = Asm::new(&[], cases.len() as u32 + 2);
    a.func("Main");
    for (i, &(op, x, y, _)) in cases.iter().enumerate() {
        a.push(x).push(y).op(op, &[]).op(Op::PopStatic, &[i as u32]);
    }
    let n = cases.len() as u32;
    a.push(0).op(Op::Not, &[]).op(Op::PopStatic, &[n]);
    a.push(0b1010)
        .op(Op::BitNot, &[])
        .op(Op::PopStatic, &[n + 1]);
    a.end();
    let (inst, _) = run_main(&a);
    for (i, &(op, x, y, want)) in cases.iter().enumerate() {
        assert_eq!(inst.statics()[i], want, "{op:?} {x} {y}");
    }
    assert_eq!(inst.statics()[cases.len()], 1);
    assert_eq!(inst.statics()[cases.len() + 1], !0b1010);
}

#[test]
fn division_by_zero_errors_the_thread() {
    for op in [Op::Div, Op::Mod] {
        let mut a = Asm::new(&[], 0);
        a.func("Main").push(1).push(0).op(op, &[]).end();
        let mut inst = a.instance();
        let out = inst.call(&mut eng(), "Main", &[]).unwrap();
        assert_eq!(out, CallOutcome::Failed(VmError::DivideByZero));
        assert_eq!(inst.error_count(), 1);
    }
}

#[test]
fn locals_statics_discard_and_pop_n() {
    let mut a = Asm::new(&[], 3);
    a.func("Main")
        .op(Op::AllocLocal, &[])
        .op(Op::AllocLocal, &[])
        .push(11)
        .op(Op::PopLocal, &[1])
        .op(Op::PushLocal, &[1])
        .op(Op::PushLocal, &[0])
        .op(Op::Add, &[])
        .op(Op::PopStatic, &[0])
        .op(Op::PushStatic, &[0])
        .push(1)
        .op(Op::Add, &[])
        .op(Op::PopStatic, &[1])
        .push(99)
        .op(Op::Discard, &[])
        .push(1)
        .push(2)
        .push(3)
        .op(Op::PopN, &[0, 2])
        .op(Op::PopStatic, &[2])
        .end();
    let (inst, _) = run_main(&a);
    assert_eq!(inst.statics(), &[11, 12, 1]);
}

#[test]
fn bad_local_static_piece_and_address_are_errors_not_panics() {
    type Case = (Box<dyn Fn(&mut Asm)>, VmError);
    let cases: Vec<Case> = vec![
        (
            Box::new(|a: &mut Asm| {
                a.op(Op::PushLocal, &[0]);
            }),
            VmError::BadLocal(0),
        ),
        (
            Box::new(|a: &mut Asm| {
                a.op(Op::PushStatic, &[5]);
            }),
            VmError::BadStatic(5),
        ),
        (
            Box::new(|a: &mut Asm| {
                a.op(Op::Hide, &[3]);
            }),
            VmError::BadPiece(3),
        ),
        (
            Box::new(|a: &mut Asm| {
                a.push(0).op(Op::TurnNow, &[0, 3]);
            }),
            VmError::BadAxis(3),
        ),
        (
            Box::new(|a: &mut Asm| {
                // Into the middle of the jump itself.
                let at = a.here();
                a.op(Op::Jump, &[at + 1]);
            }),
            VmError::BadAddress(1),
        ),
        (
            Box::new(|a: &mut Asm| {
                a.op(Op::CallScript, &[9, 0]);
            }),
            VmError::BadFunction(9),
        ),
    ];
    for (build, want) in cases {
        let mut a = Asm::new(&["p"], 1);
        a.func("Main");
        build(&mut a);
        a.end();
        let mut inst = a.instance();
        assert_eq!(
            inst.call(&mut eng(), "Main", &[]).unwrap(),
            CallOutcome::Failed(want)
        );
    }
}

#[test]
fn stack_underflow_errors_only_that_thread() {
    let mut a = Asm::new(&[], 1);
    a.func("Bad").op(Op::Add, &[]).end();
    // A healthy thread that counts ticks.
    a.func("Good");
    let top = a.here();
    a.op(Op::PushStatic, &[0])
        .push(1)
        .op(Op::Add, &[])
        .op(Op::PopStatic, &[0])
        .push(0)
        .op(Op::Sleep, &[])
        .op(Op::Jump, &[top]);
    let mut inst = a.instance();
    let mut e = eng();
    inst.start("Good", &[]).unwrap();
    inst.start("Bad", &[]).unwrap();
    for _ in 0..5 {
        inst.tick(&mut e);
    }
    assert_eq!(inst.error_count(), 1);
    assert_eq!(inst.errors()[0].error, VmError::StackUnderflow);
    assert_eq!(inst.errors()[0].tick, 1);
    assert_eq!(inst.thread_count(), 1);
    assert_eq!(inst.statics()[0], 5);
}

#[test]
fn empty_epilogue_returns_zero_and_explicit_return_value() {
    let mut a = Asm::new(&[], 0);
    a.func("Implicit").end();
    a.func("Explicit").push(7).op(Op::Return, &[]);
    let mut inst = a.instance();
    let mut e = eng();
    let r = inst.call(&mut e, "Implicit", &[]).unwrap();
    assert_eq!(
        r,
        CallOutcome::Returned {
            value: 0,
            locals: vec![0]
        }
    );
    let r = inst.call(&mut e, "Explicit", &[]).unwrap();
    assert!(matches!(r, CallOutcome::Returned { value: 7, .. }));
    assert_eq!(inst.error_count(), 0);
}

#[test]
fn callee_cannot_pop_callers_operands() {
    let mut a = Asm::new(&[], 0);
    a.func("Main").push(1).op(Op::CallScript, &[1, 0]).end();
    a.func("Callee").op(Op::Discard, &[]).end();
    let mut inst = a.instance();
    assert_eq!(
        inst.call(&mut eng(), "Main", &[]).unwrap(),
        CallOutcome::Failed(VmError::StackUnderflow)
    );
}

#[test]
fn call_script_passes_args_and_discards_return_value() {
    let mut a = Asm::new(&[], 2);
    // Main: push 3, 4; call Sum(3, 4); stack depth must be back to 0.
    a.func("Main")
        .push(3)
        .push(4)
        .op(Op::CallScript, &[1, 2])
        .end();
    // Sum(x, y): static0 = x - y (order check); return x + y.
    a.func("Sum")
        .op(Op::AllocLocal, &[])
        .op(Op::AllocLocal, &[])
        .op(Op::PushLocal, &[0])
        .op(Op::PushLocal, &[1])
        .op(Op::Sub, &[])
        .op(Op::PopStatic, &[0])
        .op(Op::PushLocal, &[0])
        .op(Op::PushLocal, &[1])
        .op(Op::Add, &[])
        .op(Op::Return, &[]);
    let (inst, _) = run_main(&a);
    assert_eq!(inst.statics()[0], -1);
    // Query style: the engine reads locals back.
    let mut inst = a.instance();
    let r = inst.call(&mut eng(), "Sum", &[10, 4]).unwrap();
    assert_eq!(
        r,
        CallOutcome::Returned {
            value: 14,
            locals: vec![10, 4, 0, 0]
        }
    );
}

#[test]
fn query_callback_writes_piece_into_local() {
    // QueryWeapon(piece) { piece = 2; }
    let mut a = Asm::new(&["a", "b", "c"], 0);
    a.func("QueryWeapon")
        .op(Op::AllocLocal, &[])
        .push(2)
        .op(Op::PopLocal, &[0])
        .end();
    let mut inst = a.instance();
    let r = inst.call(&mut eng(), "QueryWeapon", &[0]).unwrap();
    assert_eq!(r.local(0), Some(2));
    assert_eq!(
        inst.call(&mut eng(), "Nope", &[]),
        Err(StartError::NoSuchFunction("Nope".into()))
    );
}

#[test]
fn sleep_tick_mapping() {
    assert_eq!(sleep_ticks(-5), 1);
    assert_eq!(sleep_ticks(0), 1);
    assert_eq!(sleep_ticks(1), 1);
    assert_eq!(sleep_ticks(33), 1);
    assert_eq!(sleep_ticks(34), 2);
    assert_eq!(sleep_ticks(100), 3);
    assert_eq!(sleep_ticks(115), 4);
    assert_eq!(sleep_ticks(1000), 30);
    assert_eq!(sleep_ticks(i32::MAX), 64_424_510);
}

/// `static0 += 1; sleep ms` forever; returns the count after each tick.
fn sleep_counts(ms: i32, ticks: usize) -> Vec<i32> {
    let mut a = Asm::new(&[], 1);
    a.func("Loop");
    let top = a.here();
    a.op(Op::PushStatic, &[0])
        .push(1)
        .op(Op::Add, &[])
        .op(Op::PopStatic, &[0])
        .push(ms)
        .op(Op::Sleep, &[])
        .op(Op::Jump, &[top]);
    let mut inst = a.instance();
    let mut e = eng();
    inst.start("Loop", &[]).unwrap();
    (0..ticks)
        .map(|_| {
            inst.tick(&mut e);
            inst.statics()[0]
        })
        .collect()
}

#[test]
fn sleep_wakes_on_first_tick_at_or_after_deadline() {
    // Started threads first run on tick 1.
    assert_eq!(sleep_counts(0, 4), vec![1, 2, 3, 4]);
    assert_eq!(sleep_counts(33, 3), vec![1, 2, 3]);
    assert_eq!(sleep_counts(100, 7), vec![1, 1, 1, 2, 2, 2, 3]);
    assert_eq!(sleep_counts(34, 5), vec![1, 1, 2, 2, 3]);
    let second = sleep_counts(1000, 61);
    assert_eq!(second[29], 1);
    assert_eq!(second[30], 2);
    assert_eq!(second[60], 3);
}

#[test]
fn move_and_turn_take_exact_ticks_and_now_variants_snap() {
    let mut a = Asm::new(&["p"], 0);
    a.func("Main")
        // move p x to 300 raw at 900 raw/s: 30 per tick, 10 ticks.
        .push(900)
        .push(300)
        .op(Op::Move, &[0, 0])
        // turn p y to 1000 at 3000/s: 100 per tick, 10 ticks.
        .push(3000)
        .push(1000)
        .op(Op::Turn, &[0, 1])
        .push(-5)
        .op(Op::MoveNow, &[0, 2])
        .push(-16384)
        .op(Op::TurnNow, &[0, 2])
        .end();
    let mut inst = a.instance();
    let mut e = eng();
    inst.call(&mut e, "Main", &[]).unwrap();
    let t = inst.transforms()[0];
    assert_eq!(t.offset[2].raw(), -5);
    assert_eq!(t.rotation[2].raw(), 49152);
    assert_eq!(t.offset[0].raw(), 0);
    for i in 1..=10 {
        inst.tick(&mut e);
        let t = inst.transforms()[0];
        assert_eq!(t.offset[0].raw(), 30 * i);
        assert_eq!(i32::from(t.rotation[1].raw()), 100 * i);
    }
    assert!(!inst.pieces()[0].moves[0].busy());
    assert!(!inst.pieces()[0].turns[1].busy());
}

#[test]
fn spin_and_stop_spin() {
    let mut a = Asm::new(&["wheel"], 0);
    a.func("Go")
        .push(0)
        .push(30 * 182)
        .op(Op::Spin, &[0, 0])
        .end();
    a.func("Halt").push(0).op(Op::StopSpin, &[0, 0]).end();
    let mut inst = a.instance();
    let mut e = eng();
    inst.call(&mut e, "Go", &[]).unwrap();
    for _ in 0..30 {
        inst.tick(&mut e);
    }
    // 30 deg/s for one second: 5460 units.
    assert_eq!(inst.transforms()[0].rotation[0].raw(), 5460);
    assert!(inst.pieces()[0].turns[0].spinning());
    inst.call(&mut e, "Halt", &[]).unwrap();
    inst.tick(&mut e);
    assert_eq!(inst.transforms()[0].rotation[0].raw(), 5460);
    assert!(!inst.pieces()[0].turns[0].spinning());
}

#[test]
fn render_flags() {
    let mut a = Asm::new(&["p", "q"], 0);
    a.func("Main")
        .op(Op::Hide, &[0])
        .op(Op::DontCache, &[0])
        .op(Op::DontShade, &[0])
        .op(Op::DontShadow, &[0])
        .op(Op::Hide, &[1])
        .op(Op::Show, &[1])
        .op(Op::DontCache, &[1])
        .op(Op::Cache, &[1])
        .op(Op::DontShade, &[1])
        .op(Op::Shade, &[1])
        .end();
    let (inst, _) = run_main(&a);
    let p = inst.pieces()[0].flags;
    assert!(!p.visible && !p.cached && !p.shaded && !p.shadow);
    let q = inst.pieces()[1].flags;
    assert!(q.visible && q.cached && q.shaded && q.shadow);
    assert!(!inst.transforms()[0].visible);
}

#[test]
fn wait_for_turn_and_move_block_until_done() {
    let mut a = Asm::new(&["arm"], 2);
    // Turn 900 units at 2700/s: 90 per tick, done on tick 10.
    a.func("Swing")
        .push(2700)
        .push(900)
        .op(Op::Turn, &[0, 0])
        .op(Op::WaitForTurn, &[0, 0])
        .push(1)
        .op(Op::PopStatic, &[0])
        // Waiting on an idle axis does not block.
        .op(Op::WaitForTurn, &[0, 1])
        .push(30)
        .push(60)
        .op(Op::Move, &[0, 1])
        .op(Op::WaitForMove, &[0, 1])
        .push(1)
        .op(Op::PopStatic, &[1])
        .end();
    let mut inst = a.instance();
    let mut e = eng();
    let id = match inst.call(&mut e, "Swing", &[]).unwrap() {
        CallOutcome::Blocked(id) => id,
        other => panic!("{other:?}"),
    };
    for _ in 0..9 {
        inst.tick(&mut e);
        assert_eq!(inst.statics()[0], 0);
    }
    inst.tick(&mut e);
    assert_eq!(inst.statics()[0], 1);
    assert_eq!(inst.now(), 10);
    // 60 raw at 30/s = 1 per tick: 60 ticks.
    for _ in 0..59 {
        inst.tick(&mut e);
    }
    assert_eq!(inst.statics()[1], 0);
    assert!(inst.is_alive(id));
    inst.tick(&mut e);
    assert_eq!(inst.statics()[1], 1);
    assert!(!inst.is_alive(id));
}

/// `Spin(n) { set-signal-mask n; loop { static[n] += 1; sleep 0 } }`
fn signal_program() -> Asm {
    let mut a = Asm::new(&[], 5);
    a.func("Counter")
        .op(Op::AllocLocal, &[])
        .op(Op::PushLocal, &[0])
        .op(Op::SetSignalMask, &[]);
    let top = a.here();
    // static[local0] has no opcode; use two fixed counters by mask.
    a.op(Op::PushLocal, &[0]).push(1).op(Op::Eq, &[]);
    let not_one = a.jump_fwd(Op::JumpIfFalse);
    a.op(Op::PushStatic, &[1])
        .push(1)
        .op(Op::Add, &[])
        .op(Op::PopStatic, &[1]);
    let join = a.jump_fwd(Op::Jump);
    a.patch(not_one);
    a.op(Op::PushStatic, &[2])
        .push(1)
        .op(Op::Add, &[])
        .op(Op::PopStatic, &[2]);
    a.patch(join);
    a.push(0).op(Op::Sleep, &[]).op(Op::Jump, &[top]);
    // Spawner: starts Counter(1) and Counter(2) with one and three args.
    a.func("Spawn")
        .push(1)
        .op(Op::StartScript, &[0, 1])
        .push(2)
        .push(77)
        .push(78)
        .op(Op::StartScript, &[0, 3])
        .end();
    // Kill(mask)
    a.func("Kill")
        .op(Op::AllocLocal, &[])
        .op(Op::PushLocal, &[0])
        .op(Op::Signal, &[])
        .end();
    // Parent sets mask 4 and starts a sleeper that inherits it.
    a.func("Parent")
        .push(4)
        .op(Op::SetSignalMask, &[])
        .op(Op::StartScript, &[4, 0])
        .end();
    a.func("Sleeper")
        .push(1000)
        .op(Op::Sleep, &[])
        .push(1)
        .op(Op::PopStatic, &[4])
        .end();
    a
}

#[test]
fn start_script_signal_and_mask_inheritance() {
    let a = signal_program();
    let mut inst = a.instance();
    let mut e = eng();
    inst.call(&mut e, "Spawn", &[]).unwrap();
    assert_eq!(inst.thread_count(), 2);
    for _ in 0..3 {
        inst.tick(&mut e);
    }
    assert_eq!(&inst.statics()[1..3], &[3, 3]);
    // signal 2 kills only Counter(2).
    inst.call(&mut e, "Kill", &[2]).unwrap();
    assert_eq!(inst.thread_count(), 1);
    for _ in 0..3 {
        inst.tick(&mut e);
    }
    assert_eq!(&inst.statics()[1..3], &[6, 3]);
    // The signalling thread survives its own signal.
    let mut b = Asm::new(&[], 1);
    b.func("SelfSignal")
        .push(1)
        .op(Op::SetSignalMask, &[])
        .push(1)
        .op(Op::Signal, &[])
        .push(5)
        .op(Op::PopStatic, &[0])
        .end();
    let mut solo = b.instance();
    let r = solo.call(&mut e, "SelfSignal", &[]).unwrap();
    assert!(matches!(r, CallOutcome::Returned { .. }));
    assert_eq!(solo.statics()[0], 5);
    // Started threads inherit the parent's mask: signal 4 kills the sleeper.
    inst.call(&mut e, "Parent", &[]).unwrap();
    assert_eq!(inst.thread_count(), 2);
    inst.signal(4);
    for _ in 0..40 {
        inst.tick(&mut e);
    }
    assert_eq!(inst.statics()[4], 0);
    assert_eq!(inst.thread_count(), 1);
    inst.kill_all();
    inst.tick(&mut e);
    assert_eq!(inst.thread_count(), 0);
}

#[test]
fn threads_run_in_creation_order_and_new_threads_run_same_tick() {
    // Each thread appends its id digit: static0 = static0 * 10 + id.
    let mut a = Asm::new(&[], 1);
    a.func("Mark")
        .op(Op::AllocLocal, &[])
        .op(Op::PushStatic, &[0])
        .push(10)
        .op(Op::Mul, &[])
        .op(Op::PushLocal, &[0])
        .op(Op::Add, &[])
        .op(Op::PopStatic, &[0])
        .end();
    a.func("Chain").push(3).op(Op::StartScript, &[0, 1]).end();
    let mut inst = a.instance();
    let mut e = eng();
    inst.start("Mark", &[1]).unwrap();
    inst.start("Chain", &[]).unwrap();
    inst.start("Mark", &[2]).unwrap();
    inst.tick(&mut e);
    assert_eq!(inst.statics()[0], 123);
    assert_eq!(inst.thread_count(), 0);
}

#[test]
fn engine_ports_effects_and_random() {
    let mut a = Asm::new(&["p"], 6);
    a.sounds = vec!["SWISH".into(), "create X".into()];
    a.func("Main")
        .push(21)
        .push(5)
        .op(Op::SetUnitValue, &[])
        .push(21)
        .op(Op::GetUnitValue, &[])
        .op(Op::PopStatic, &[0])
        .push(4)
        .op(Op::GetUnitValue, &[])
        .op(Op::PopStatic, &[1])
        .push(21)
        .push(1)
        .push(2)
        .push(3)
        .push(4)
        .op(Op::Get, &[])
        .op(Op::PopStatic, &[2])
        .push(10)
        .push(12)
        .op(Op::Rand, &[])
        .op(Op::PopStatic, &[3])
        .push(7)
        .op(Op::EmitSfx, &[0])
        .push(3)
        .op(Op::Explode, &[0])
        .push(2)
        .op(Op::PlaySound, &[0])
        .op(Op::PopStatic, &[4])
        .push(8)
        .push(9)
        .op(Op::MissionCommand, &[1, 2])
        .op(Op::PopStatic, &[5])
        .end();
    let (inst, e) = run_main(&a);
    assert_eq!(inst.statics()[0], 5);
    assert_eq!(inst.statics()[1], 100);
    assert_eq!(inst.statics()[2], 5);
    assert!((10..=12).contains(&inst.statics()[3]));
    assert_eq!(
        e.effects,
        vec![
            Effect::Sfx { piece: 0, sfx: 7 },
            Effect::Explode { piece: 0, flags: 3 },
            Effect::Sound {
                sound: 0,
                priority: 2
            },
            Effect::MissionCommand {
                command: 1,
                args: vec![8, 9]
            },
        ]
    );
    assert_eq!(e.unknown_writes.get(&21), Some(&1));
    assert_eq!(e.unknown_reads.get(&21), Some(&2));
}

#[test]
fn runaway_loop_hits_instruction_limit() {
    let mut a = Asm::new(&[], 0);
    a.func("Spin");
    let top = a.here();
    a.op(Op::Jump, &[top]);
    let mut inst = Instance::with_limits(
        a.program(),
        Limits {
            max_instructions_per_tick: 1000,
            ..Limits::default()
        },
    );
    inst.start("Spin", &[]).unwrap();
    inst.tick(&mut eng());
    assert_eq!(inst.errors()[0].error, VmError::InstructionLimit(1000));
    assert_eq!(inst.thread_count(), 0);
}

#[test]
fn recursion_hits_call_depth() {
    let mut a = Asm::new(&[], 0);
    a.func("Rec").op(Op::CallScript, &[0, 0]).end();
    let mut inst = a.instance();
    assert_eq!(
        inst.call(&mut eng(), "Rec", &[]).unwrap(),
        CallOutcome::Failed(VmError::CallDepth(64))
    );
}

#[test]
fn push_loop_hits_stack_limit() {
    let mut a = Asm::new(&[], 0);
    a.func("Fill");
    let top = a.here();
    a.push(1).op(Op::Jump, &[top]);
    let mut inst = a.instance();
    assert_eq!(
        inst.call(&mut eng(), "Fill", &[]).unwrap(),
        CallOutcome::Failed(VmError::StackOverflow(1024))
    );
}

#[test]
fn fork_bomb_hits_thread_limit() {
    let mut a = Asm::new(&[], 0);
    a.func("Fork");
    let top = a.here();
    a.op(Op::StartScript, &[0, 0])
        .push(0)
        .op(Op::Sleep, &[])
        .op(Op::Jump, &[top]);
    let mut inst = a.instance();
    let mut e = eng();
    inst.start("Fork", &[]).unwrap();
    for _ in 0..12 {
        inst.tick(&mut e);
    }
    assert!(inst.thread_count() <= Limits::default().max_threads);
    assert!(
        inst.errors()
            .iter()
            .any(|e| e.error == VmError::TooManyThreads(256))
    );
}

#[test]
fn deterministic_per_tick_hashes() {
    // Random turns on two pieces, two threads, sleeps of random length.
    let mut a = Asm::new(&["a", "b"], 1);
    a.func("Wiggle").op(Op::AllocLocal, &[]);
    let top = a.here();
    a.push(1000)
        .push(9000)
        .op(Op::Rand, &[])
        .push(-8000)
        .push(8000)
        .op(Op::Rand, &[])
        .op(Op::Turn, &[0, 0])
        .push(-3)
        .push(3)
        .op(Op::Rand, &[])
        .op(Op::MoveNow, &[1, 1])
        .push(30)
        .push(200)
        .op(Op::Rand, &[])
        .op(Op::Sleep, &[])
        .op(Op::Jump, &[top]);
    a.func("Create")
        .op(Op::StartScript, &[0, 0])
        .op(Op::StartScript, &[0, 0])
        .end();
    let run = |seed: u64| {
        let mut inst = a.instance();
        let mut e = HeadlessEngine::new(seed);
        inst.start("Create", &[]).unwrap();
        (0..120)
            .map(|_| {
                inst.tick(&mut e);
                (inst.anim_hash(), inst.state_hash())
            })
            .collect::<Vec<_>>()
    };
    let first = run(9);
    assert_eq!(first, run(9));
    assert_ne!(first, run(10));
    // Poses actually change over time.
    let distinct: std::collections::BTreeSet<_> = first.iter().map(|h| h.0).collect();
    assert!(distinct.len() > 10);
}

#[test]
fn blocked_call_keeps_running_as_a_thread() {
    let mut a = Asm::new(&[], 1);
    a.func("Later")
        .push(100)
        .op(Op::Sleep, &[])
        .push(9)
        .op(Op::PopStatic, &[0])
        .end();
    let mut inst = a.instance();
    let mut e = eng();
    let r = inst.call(&mut e, "Later", &[]).unwrap();
    assert!(matches!(r, CallOutcome::Blocked(_)));
    assert_eq!(r.local(0), None);
    for _ in 0..3 {
        inst.tick(&mut e);
    }
    assert_eq!(inst.statics()[0], 9);
}
