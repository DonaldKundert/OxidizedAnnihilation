//! The COB interpreter: one [`Instance`] per unit, running green threads over
//! a shared, pre-decoded [`Program`].
//!
//! Scheduling, time and calling conventions are specified in
//! `docs/formats/cob-vm.md`. In short, one [`Instance::tick`] is:
//!
//! 1. advance every piece animation one tick;
//! 2. walk the threads in creation order; wake any whose sleep deadline has
//!    come or whose awaited turn/move has finished; run each runnable thread
//!    until it blocks, ends, or errors. Threads started during the walk are
//!    appended and run in the same tick.
//!
//! Everything is integer and iteration order is fixed, so the same program,
//! engine responses and entry calls give the same state every run.

use std::sync::Arc;

use tak_formats::cob::{self, CobError, Instr, Op, Script};

use crate::anim::{PieceAnim, PieceTransform, TICKS_PER_SECOND};
use crate::engine::Engine;
use crate::hash::Fnv64;
use std::hash::{Hash, Hasher};

/// A parsed script plus its instruction stream decoded once, indexed by code
/// word. Share one per unit type between all its instances.
#[derive(Debug, Clone)]
pub struct Program {
    script: Script,
    /// `instrs[w]` is the instruction starting at word `w`, if one does.
    instrs: Vec<Option<Instr>>,
}

impl Program {
    /// Decodes linearly from word 0, the way the retail files are laid out.
    /// Unknown words are skipped one at a time; executing one is an error.
    pub fn new(script: Script) -> Program {
        let mut instrs = vec![None; script.code.len()];
        let mut addr = 0usize;
        while addr < script.code.len() {
            match cob::decode(&script.code, addr) {
                Ok(i) => {
                    instrs[addr] = Some(i);
                    addr = i.next() as usize;
                }
                Err(_) => addr += 1,
            }
        }
        Program { script, instrs }
    }

    pub fn from_bytes(data: &[u8]) -> Result<Program, CobError> {
        Ok(Program::new(cob::parse(data)?))
    }

    pub fn script(&self) -> &Script {
        &self.script
    }

    /// First function with this exact name.
    pub fn function(&self, name: &str) -> Option<usize> {
        self.script.function(name)
    }

    fn instr(&self, pc: u32) -> Result<Instr, VmError> {
        self.instrs
            .get(pc as usize)
            .copied()
            .flatten()
            .ok_or(VmError::BadAddress(pc))
    }
}

/// Caps that keep a broken or hostile script from taking down the process.
/// Exceeding one errors the offending thread only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Live threads per instance; `start` past this fails.
    pub max_threads: usize,
    /// Data-stack depth per thread.
    pub max_stack: usize,
    /// Nested `call-script` frames per thread.
    pub max_call_depth: usize,
    /// Locals per frame (`alloc-local` count plus arguments).
    pub max_locals: usize,
    /// Instructions one thread may execute in one tick (or one `call`).
    pub max_instructions_per_tick: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_threads: 256,
            max_stack: 1024,
            max_call_depth: 64,
            max_locals: 256,
            max_instructions_per_tick: 100_000,
        }
    }
}

/// Why a thread stopped with an error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VmError {
    #[error("no instruction starts at word {0}")]
    BadAddress(u32),
    #[error("data stack underflow")]
    StackUnderflow,
    #[error("data stack overflow (limit {0})")]
    StackOverflow(usize),
    #[error("call depth limit {0} exceeded")]
    CallDepth(usize),
    #[error("too many locals (limit {0})")]
    TooManyLocals(usize),
    #[error("local {0} not allocated")]
    BadLocal(u32),
    #[error("static {0} out of range")]
    BadStatic(u32),
    #[error("piece {0} out of range")]
    BadPiece(u32),
    #[error("axis {0} out of range")]
    BadAxis(u32),
    #[error("function {0} out of range")]
    BadFunction(u32),
    #[error("division by zero")]
    DivideByZero,
    #[error("instruction limit {0} reached in one tick")]
    InstructionLimit(u32),
    #[error("thread limit {0} reached")]
    TooManyThreads(usize),
}

/// Errors from the engine-facing entry points.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StartError {
    #[error("no function named {0:?}")]
    NoSuchFunction(String),
    #[error(transparent)]
    Vm(#[from] VmError),
}

/// Stable thread handle, unique within an instance (never reused).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ThreadId(pub u32);

/// A thread that died with an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadError {
    pub thread: ThreadId,
    /// Function index of the innermost frame.
    pub function: u32,
    /// Word index of the failing instruction.
    pub pc: u32,
    pub tick: u64,
    pub error: VmError,
}

/// Most errors kept in [`Instance::errors`]; [`Instance::error_count`]
/// counts all of them.
pub const MAX_LOGGED_ERRORS: usize = 1024;

/// Result of [`Instance::call`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallOutcome {
    /// The function ran to `return` without blocking. `locals` are the
    /// callee's final locals: arguments first, so query callbacks
    /// (`QueryWeapon`, `SweetSpot`, ...) report through `locals[0]`.
    Returned { value: i32, locals: Vec<i32> },
    /// The function slept or waited; it keeps running as a normal thread.
    Blocked(ThreadId),
    /// The function errored (also logged in [`Instance::errors`]).
    Failed(VmError),
}

impl CallOutcome {
    /// `locals[i]` of a completed call, else `None`.
    pub fn local(&self, i: usize) -> Option<i32> {
        match self {
            CallOutcome::Returned { locals, .. } => locals.get(i).copied(),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Frame {
    func: u32,
    pc: u32,
    locals: Vec<i32>,
    /// Data-stack height when the frame was entered.
    base: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Wait {
    Ready,
    /// Runnable again at the start of this tick's thread walk.
    Sleep(u64),
    Turn(u32, u8),
    Move(u32, u8),
    Dead,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Thread {
    id: ThreadId,
    frames: Vec<Frame>,
    stack: Vec<i32>,
    signal_mask: i32,
    wait: Wait,
}

impl Thread {
    fn placeholder() -> Thread {
        Thread {
            id: ThreadId(u32::MAX),
            frames: Vec::new(),
            stack: Vec::new(),
            signal_mask: 0,
            wait: Wait::Dead,
        }
    }
}

enum Stop {
    Blocked,
    Finished { value: i32, locals: Vec<i32> },
}

/// Converts a `sleep` argument to whole ticks: the first tick whose start
/// time is at or after the deadline, and never less than one tick (so
/// `sleep 0` yields instead of spinning). 30 ticks per second, so this is
/// `ceil(ms * 3 / 100)`, min 1.
pub fn sleep_ticks(ms: i32) -> u64 {
    let ms = i64::from(ms.max(0));
    let ticks = (ms * TICKS_PER_SECOND + 999) / 1000;
    ticks.max(1) as u64
}

/// One unit's script state: statics, piece animations and threads.
#[derive(Debug, Clone)]
pub struct Instance {
    program: Arc<Program>,
    limits: Limits,
    statics: Vec<i32>,
    pieces: Vec<PieceAnim>,
    threads: Vec<Thread>,
    next_thread: u32,
    tick: u64,
    errors: Vec<ThreadError>,
    error_count: u64,
}

impl Instance {
    pub fn new(program: Arc<Program>) -> Instance {
        Instance::with_limits(program, Limits::default())
    }

    pub fn with_limits(program: Arc<Program>, limits: Limits) -> Instance {
        // static_count and piece count come from a parsed file; both are
        // bounded by its length, so these allocations are too.
        let statics = vec![0; program.script.static_count as usize];
        let pieces = vec![PieceAnim::default(); program.script.pieces.len()];
        Instance {
            program,
            limits,
            statics,
            pieces,
            threads: Vec::new(),
            next_thread: 0,
            tick: 0,
            errors: Vec::new(),
            error_count: 0,
        }
    }

    pub fn program(&self) -> &Arc<Program> {
        &self.program
    }
    /// Ticks run so far.
    pub fn now(&self) -> u64 {
        self.tick
    }
    pub fn statics(&self) -> &[i32] {
        &self.statics
    }
    /// Animation state, indexed like the script's piece table.
    pub fn pieces(&self) -> &[PieceAnim] {
        &self.pieces
    }
    pub fn transforms(&self) -> Vec<PieceTransform> {
        self.pieces.iter().map(PieceAnim::transform).collect()
    }
    pub fn errors(&self) -> &[ThreadError] {
        &self.errors
    }
    pub fn error_count(&self) -> u64 {
        self.error_count
    }
    /// Threads that are alive (running, sleeping or waiting).
    pub fn thread_count(&self) -> usize {
        self.threads.iter().filter(|t| t.wait != Wait::Dead).count()
    }
    pub fn is_alive(&self, id: ThreadId) -> bool {
        self.threads
            .iter()
            .any(|t| t.id == id && t.wait != Wait::Dead)
    }

    /// Hash of every piece's pose and render flags.
    pub fn anim_hash(&self) -> u64 {
        let mut h = Fnv64::new();
        for t in self.transforms() {
            t.hash(&mut h);
        }
        h.finish()
    }

    /// Hash of all VM state: statics, full animation state, threads.
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv64::new();
        self.tick.hash(&mut h);
        self.statics.hash(&mut h);
        self.pieces.hash(&mut h);
        self.threads.hash(&mut h);
        self.next_thread.hash(&mut h);
        h.finish()
    }

    /// Kills every thread whose signal mask shares a bit with `mask`, as the
    /// `signal` opcode does. For the engine (e.g. before `Killed`).
    pub fn signal(&mut self, mask: i32) {
        for t in &mut self.threads {
            if t.signal_mask & mask != 0 {
                t.wait = Wait::Dead;
            }
        }
    }

    /// Kills every thread.
    pub fn kill_all(&mut self) {
        for t in &mut self.threads {
            t.wait = Wait::Dead;
        }
    }

    /// `start-script` from the engine: spawns `name(args)` as a new thread
    /// with signal mask 0. It first runs during the next [`Instance::tick`].
    pub fn start(&mut self, name: &str, args: &[i32]) -> Result<ThreadId, StartError> {
        let func = self.lookup(name)?;
        Ok(self.spawn(func, args, 0)?)
    }

    /// `call-script` from the engine: runs `name(args)` now, until it
    /// returns or blocks. See [`CallOutcome`].
    pub fn call(
        &mut self,
        engine: &mut dyn Engine,
        name: &str,
        args: &[i32],
    ) -> Result<CallOutcome, StartError> {
        let func = self.lookup(name)?;
        let id = self.spawn(func, args, 0)?;
        let idx = self.threads.len() - 1;
        let outcome = match self.run_thread(idx, engine) {
            Some(Ok(Stop::Finished { value, locals })) => CallOutcome::Returned { value, locals },
            Some(Ok(Stop::Blocked)) => CallOutcome::Blocked(id),
            Some(Err(e)) => CallOutcome::Failed(e),
            None => CallOutcome::Blocked(id),
        };
        self.reap();
        Ok(outcome)
    }

    /// Advances one 30 Hz tick.
    pub fn tick(&mut self, engine: &mut dyn Engine) {
        self.tick += 1;
        for p in &mut self.pieces {
            p.advance();
        }
        let mut i = 0;
        while i < self.threads.len() {
            let ready = match self.threads[i].wait {
                Wait::Ready => true,
                Wait::Sleep(until) => until <= self.tick,
                Wait::Turn(p, a) => !self.pieces[p as usize].turns[a as usize].busy(),
                Wait::Move(p, a) => !self.pieces[p as usize].moves[a as usize].busy(),
                Wait::Dead => false,
            };
            if ready {
                self.threads[i].wait = Wait::Ready;
                let _ = self.run_thread(i, engine);
            }
            i += 1;
        }
        self.reap();
    }

    fn lookup(&self, name: &str) -> Result<u32, StartError> {
        self.program
            .function(name)
            .map(|f| f as u32)
            .ok_or_else(|| StartError::NoSuchFunction(name.to_string()))
    }

    fn reap(&mut self) {
        self.threads.retain(|t| t.wait != Wait::Dead);
    }

    fn spawn(&mut self, func: u32, args: &[i32], signal_mask: i32) -> Result<ThreadId, VmError> {
        let f = self
            .program
            .script
            .functions
            .get(func as usize)
            .ok_or(VmError::BadFunction(func))?;
        if self.thread_count() >= self.limits.max_threads {
            return Err(VmError::TooManyThreads(self.limits.max_threads));
        }
        if args.len() > self.limits.max_locals {
            return Err(VmError::TooManyLocals(self.limits.max_locals));
        }
        let id = ThreadId(self.next_thread);
        self.next_thread = self.next_thread.wrapping_add(1);
        self.threads.push(Thread {
            id,
            frames: vec![Frame {
                func,
                pc: f.entry,
                locals: args.to_vec(),
                base: 0,
            }],
            stack: Vec::new(),
            signal_mask,
            wait: Wait::Ready,
        });
        Ok(id)
    }

    /// Runs thread `idx` until it blocks, finishes or errors. The thread is
    /// taken out of its slot while it runs, so a `signal` it sends never
    /// kills itself and threads it starts are appended safely. Returns
    /// `None` if the slot held no runnable thread.
    fn run_thread(&mut self, idx: usize, engine: &mut dyn Engine) -> Option<Result<Stop, VmError>> {
        let mut th = std::mem::replace(&mut self.threads[idx], Thread::placeholder());
        if th.wait != Wait::Ready || th.frames.is_empty() {
            self.threads[idx] = th;
            return None;
        }
        let result = self.exec(&mut th, engine);
        match &result {
            Ok(Stop::Blocked) => {}
            Ok(Stop::Finished { .. }) => th.wait = Wait::Dead,
            Err(e) => {
                let (function, pc) = th.frames.last().map(|f| (f.func, f.pc)).unwrap_or((0, 0));
                self.error_count += 1;
                if self.errors.len() < MAX_LOGGED_ERRORS {
                    self.errors.push(ThreadError {
                        thread: th.id,
                        function,
                        pc,
                        tick: self.tick,
                        error: e.clone(),
                    });
                }
                th.wait = Wait::Dead;
            }
        }
        self.threads[idx] = th;
        Some(result)
    }

    fn exec(&mut self, th: &mut Thread, engine: &mut dyn Engine) -> Result<Stop, VmError> {
        let program = Arc::clone(&self.program);
        let limits = self.limits;
        let mut budget = limits.max_instructions_per_tick;
        loop {
            if budget == 0 {
                return Err(VmError::InstructionLimit(limits.max_instructions_per_tick));
            }
            budget -= 1;
            let frame = th.frames.last_mut().ok_or(VmError::StackUnderflow)?;
            let ins = program.instr(frame.pc)?;
            let [a0, a1] = ins.args;
            // On error, the frame's pc still names the failing instruction.
            let next = ins.next();
            match ins.op {
                Op::Move | Op::Turn => {
                    let (p, ax) = self.piece_axis(a0, a1)?;
                    let target = pop(th)?;
                    let speed = pop(th)?;
                    let piece = &mut self.pieces[p];
                    if ins.op == Op::Move {
                        piece.move_to(ax, target, speed);
                    } else {
                        piece.turn_to(ax, target, speed);
                    }
                }
                Op::MoveNow | Op::TurnNow => {
                    let (p, ax) = self.piece_axis(a0, a1)?;
                    let target = pop(th)?;
                    if ins.op == Op::MoveNow {
                        self.pieces[p].move_now(ax, target);
                    } else {
                        self.pieces[p].turn_now(ax, target);
                    }
                }
                Op::Spin => {
                    let (p, ax) = self.piece_axis(a0, a1)?;
                    let speed = pop(th)?;
                    let accel = pop(th)?;
                    self.pieces[p].spin(ax, speed, accel);
                }
                Op::StopSpin => {
                    let (p, ax) = self.piece_axis(a0, a1)?;
                    let decel = pop(th)?;
                    self.pieces[p].stop_spin(ax, decel);
                }
                Op::Show | Op::Hide | Op::Cache | Op::DontCache | Op::DontShadow => {
                    let p = self.piece(a0)?;
                    let f = &mut self.pieces[p].flags;
                    match ins.op {
                        Op::Show => f.visible = true,
                        Op::Hide => f.visible = false,
                        Op::Cache => f.cached = true,
                        Op::DontCache => f.cached = false,
                        _ => f.shadow = false,
                    }
                }
                Op::Shade | Op::DontShade => {
                    let p = self.piece(a0)?;
                    self.pieces[p].flags.shaded = ins.op == Op::Shade;
                }
                Op::LegacyEffect => {
                    self.piece(a0)?;
                    pop(th)?;
                    pop(th)?;
                }
                Op::EmitSfx => {
                    let p = self.piece(a0)?;
                    let sfx = pop(th)?;
                    engine.emit_sfx(p as u32, sfx);
                }
                Op::Explode => {
                    let p = self.piece(a0)?;
                    let flags = pop(th)?;
                    engine.explode(p as u32, flags);
                }
                Op::WaitForTurn | Op::WaitForMove => {
                    let (p, ax) = self.piece_axis(a0, a1)?;
                    let busy = if ins.op == Op::WaitForTurn {
                        self.pieces[p].turns[ax].busy()
                    } else {
                        self.pieces[p].moves[ax].busy()
                    };
                    if busy {
                        set_pc(th, next);
                        th.wait = if ins.op == Op::WaitForTurn {
                            Wait::Turn(p as u32, ax as u8)
                        } else {
                            Wait::Move(p as u32, ax as u8)
                        };
                        return Ok(Stop::Blocked);
                    }
                }
                Op::Sleep => {
                    let ms = pop(th)?;
                    set_pc(th, next);
                    th.wait = Wait::Sleep(self.tick + sleep_ticks(ms));
                    return Ok(Stop::Blocked);
                }
                Op::PushConst => push(th, a0 as i32, limits)?,
                Op::PushLocal => {
                    let v = *frame_of(th)?
                        .locals
                        .get(a0 as usize)
                        .ok_or(VmError::BadLocal(a0))?;
                    push(th, v, limits)?;
                }
                Op::PushStatic => {
                    let v = *self
                        .statics
                        .get(a0 as usize)
                        .ok_or(VmError::BadStatic(a0))?;
                    push(th, v, limits)?;
                }
                Op::AllocLocal => {
                    let f = frame_of(th)?;
                    if f.locals.len() >= limits.max_locals {
                        return Err(VmError::TooManyLocals(limits.max_locals));
                    }
                    f.locals.push(0);
                }
                Op::PopLocal => {
                    let v = pop(th)?;
                    let slot = frame_of(th)?
                        .locals
                        .get_mut(a0 as usize)
                        .ok_or(VmError::BadLocal(a0))?;
                    *slot = v;
                }
                Op::PopStatic => {
                    let v = pop(th)?;
                    let slot = self
                        .statics
                        .get_mut(a0 as usize)
                        .ok_or(VmError::BadStatic(a0))?;
                    *slot = v;
                }
                Op::Discard => {
                    pop(th)?;
                }
                Op::PopN => {
                    // Negative counts discard nothing.
                    for _ in 0..(a1 as i32).max(0) {
                        pop(th)?;
                    }
                }
                Op::BitNot => {
                    let a = pop(th)?;
                    push(th, !a, limits)?;
                }
                Op::Not => {
                    let a = pop(th)?;
                    push(th, i32::from(a == 0), limits)?;
                }
                Op::Add
                | Op::Sub
                | Op::Mul
                | Op::Div
                | Op::Mod
                | Op::BitAnd
                | Op::BitOr
                | Op::BitXor
                | Op::ShiftLeft
                | Op::ShiftRight
                | Op::Lt
                | Op::Le
                | Op::Gt
                | Op::Ge
                | Op::Eq
                | Op::Ne
                | Op::And
                | Op::Or
                | Op::Xor => {
                    let b = pop(th)?;
                    let a = pop(th)?;
                    push(th, binary(ins.op, a, b)?, limits)?;
                }
                Op::Rand => {
                    let high = pop(th)?;
                    let low = pop(th)?;
                    let r = engine.rand(low, high);
                    push(th, r, limits)?;
                }
                Op::GetUnitValue => {
                    let port = pop(th)?;
                    let v = engine.get_unit_value(port);
                    push(th, v, limits)?;
                }
                Op::Get => {
                    let d = pop(th)?;
                    let c = pop(th)?;
                    let b = pop(th)?;
                    let a = pop(th)?;
                    let port = pop(th)?;
                    let v = engine.get(port, [a, b, c, d]);
                    push(th, v, limits)?;
                }
                Op::CargoHas => {
                    // TA only; never used in Kingdoms. ( unit -- 0 )
                    pop(th)?;
                    push(th, 0, limits)?;
                }
                Op::Carrier => push(th, 0, limits)?,
                Op::SetUnitValue => {
                    let v = pop(th)?;
                    let port = pop(th)?;
                    engine.set(port, v);
                }
                Op::AttachUnit => {
                    let extra = pop(th)?;
                    let piece = pop(th)?;
                    let unit = pop(th)?;
                    engine.attach_unit(unit, piece, extra);
                }
                Op::DropUnit => {
                    let unit = pop(th)?;
                    engine.drop_unit(unit);
                }
                Op::PlaySound => {
                    let priority = pop(th)?;
                    let r = engine.play_sound(a0, priority);
                    push(th, r, limits)?;
                }
                Op::MissionCommand => {
                    let args = pop_args(th, a1, limits)?;
                    let r = engine.mission_command(a0, &args);
                    push(th, r, limits)?;
                }
                Op::StartScript => {
                    let args = pop_args(th, a1, limits)?;
                    // Believed (TA): the new thread inherits the signal mask.
                    self.spawn(a0, &args, th.signal_mask)?;
                }
                Op::CallScript => {
                    let args = pop_args(th, a1, limits)?;
                    let entry = program
                        .script
                        .functions
                        .get(a0 as usize)
                        .ok_or(VmError::BadFunction(a0))?
                        .entry;
                    if th.frames.len() >= limits.max_call_depth {
                        return Err(VmError::CallDepth(limits.max_call_depth));
                    }
                    set_pc(th, next);
                    let base = th.stack.len();
                    th.frames.push(Frame {
                        func: a0,
                        pc: entry,
                        locals: args,
                        base,
                    });
                    continue;
                }
                Op::Jump => {
                    set_pc(th, a0);
                    continue;
                }
                Op::JumpIfFalse => {
                    if pop(th)? == 0 {
                        set_pc(th, a0);
                        continue;
                    }
                }
                Op::Return => {
                    let frame = th.frames.pop().ok_or(VmError::StackUnderflow)?;
                    // `alloc-local; return` with nothing pushed returns 0
                    // (the retail epilogue); see cob-vm.md.
                    let value = if th.stack.len() > frame.base {
                        th.stack.pop().unwrap_or(0)
                    } else {
                        0
                    };
                    th.stack.truncate(frame.base);
                    if th.frames.is_empty() {
                        return Ok(Stop::Finished {
                            value,
                            locals: frame.locals,
                        });
                    }
                    // call-script discards the callee's return value.
                    continue;
                }
                Op::Signal => {
                    let mask = pop(th)?;
                    self.signal(mask);
                }
                Op::SetSignalMask => {
                    th.signal_mask = pop(th)?;
                }
            }
            set_pc(th, next);
        }
    }

    fn piece(&self, p: u32) -> Result<usize, VmError> {
        if (p as usize) < self.pieces.len() {
            Ok(p as usize)
        } else {
            Err(VmError::BadPiece(p))
        }
    }

    fn piece_axis(&self, p: u32, axis: u32) -> Result<(usize, usize), VmError> {
        let p = self.piece(p)?;
        if axis < 3 {
            Ok((p, axis as usize))
        } else {
            Err(VmError::BadAxis(axis))
        }
    }
}

fn frame_of(th: &mut Thread) -> Result<&mut Frame, VmError> {
    th.frames.last_mut().ok_or(VmError::StackUnderflow)
}

fn set_pc(th: &mut Thread, pc: u32) {
    if let Some(f) = th.frames.last_mut() {
        f.pc = pc;
    }
}

/// Pops one value, never below the current frame's base: a callee cannot
/// eat its caller's operands.
fn pop(th: &mut Thread) -> Result<i32, VmError> {
    let base = th.frames.last().map_or(0, |f| f.base);
    if th.stack.len() <= base {
        return Err(VmError::StackUnderflow);
    }
    th.stack.pop().ok_or(VmError::StackUnderflow)
}

fn push(th: &mut Thread, v: i32, limits: Limits) -> Result<(), VmError> {
    if th.stack.len() >= limits.max_stack {
        return Err(VmError::StackOverflow(limits.max_stack));
    }
    th.stack.push(v);
    Ok(())
}

/// Pops `argc` values and returns them in push order (first argument first).
fn pop_args(th: &mut Thread, argc: u32, limits: Limits) -> Result<Vec<i32>, VmError> {
    let n = argc as usize;
    if n > limits.max_locals {
        return Err(VmError::TooManyLocals(limits.max_locals));
    }
    let base = th.frames.last().map_or(0, |f| f.base);
    if th.stack.len() < base + n {
        return Err(VmError::StackUnderflow);
    }
    let at = th.stack.len() - n;
    Ok(th.stack.split_off(at))
}

/// Binary operators. Arithmetic wraps; comparisons and logic give 0 or 1.
fn binary(op: Op, a: i32, b: i32) -> Result<i32, VmError> {
    Ok(match op {
        Op::Add => a.wrapping_add(b),
        Op::Sub => a.wrapping_sub(b),
        Op::Mul => a.wrapping_mul(b),
        Op::Div => {
            if b == 0 {
                return Err(VmError::DivideByZero);
            }
            a.wrapping_div(b)
        }
        Op::Mod => {
            if b == 0 {
                return Err(VmError::DivideByZero);
            }
            // Believed; sign follows the dividend (never negative in retail).
            a.wrapping_rem(b)
        }
        Op::BitAnd => a & b,
        Op::BitOr => a | b,
        Op::BitXor => a ^ b,
        // Believed. Shift counts outside 0..32 give 0 (left) or the sign
        // fill (right); retail only shifts by small constants.
        Op::ShiftLeft => u32::try_from(b)
            .ok()
            .and_then(|s| a.checked_shl(s))
            .unwrap_or(0),
        Op::ShiftRight => u32::try_from(b)
            .ok()
            .and_then(|s| a.checked_shr(s))
            .unwrap_or(if a < 0 { -1 } else { 0 }),
        Op::Lt => i32::from(a < b),
        Op::Le => i32::from(a <= b),
        Op::Gt => i32::from(a > b),
        Op::Ge => i32::from(a >= b),
        Op::Eq => i32::from(a == b),
        Op::Ne => i32::from(a != b),
        Op::And => i32::from(a != 0 && b != 0),
        Op::Or => i32::from(a != 0 || b != 0),
        Op::Xor => i32::from((a != 0) != (b != 0)),
        _ => unreachable!("binary() called with {op:?}"),
    })
}
