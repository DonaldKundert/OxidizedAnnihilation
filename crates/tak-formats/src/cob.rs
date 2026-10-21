//! COB v6 unit and mission scripts (`.cob`): header, script table, pieces,
//! sounds, code words, the opcode table and a disassembler. Work package
//! WP-D (M5). Format notes: `docs/formats/cob.md`.
//!
//! The opcode table ([`Op`]) is the single source of truth for mnemonics,
//! raw values and inline operands. The VM (WP-G) decodes with [`decode`] and
//! dispatches on [`Op`]; it should not keep a second table.
//!
//! Opcode values are taken from Nanolathe's `research/formats/cob.md`
//! (MIT, <https://github.com/nanolathe-gg/nanolathe>) and checked against
//! every retail Kingdoms script. No Nanolathe code is used.

use crate::reader::{ReadError, Reader};
use std::collections::BTreeSet;
use std::fmt::Write as _;

/// The only version signature Kingdoms ships.
pub const VERSION: u32 = 6;
/// 13 little-endian `u32` words.
pub const HEADER_LEN: usize = 52;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CobError {
    #[error(transparent)]
    Read(#[from] ReadError),
    #[error("unsupported COB version {0} (Kingdoms uses 6)")]
    Version(u32),
    #[error("{what}: offset {offset} is not 4-byte aligned")]
    Misaligned { what: &'static str, offset: u32 },
    #[error("script {index} ({name}) entry word {entry} is outside the {code_words}-word code")]
    EntryOutOfRange {
        index: usize,
        name: String,
        entry: u32,
        code_words: u32,
    },
    #[error("{what} {index}: empty name")]
    EmptyName { what: &'static str, index: usize },
}

/// Errors from decoding one instruction.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    #[error("address {addr} is past the end of the code ({len} words)")]
    OutOfRange { addr: usize, len: usize },
    #[error("unknown opcode {value:#010x} at word {addr}")]
    UnknownOpcode { addr: usize, value: u32 },
    #[error("{op} at word {addr} needs {need} operand words, only {have} remain")]
    Truncated {
        addr: usize,
        op: &'static str,
        need: usize,
        have: usize,
    },
}

/// The raw header, field for field. Offsets are absolute byte offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// Word 0. Always 6 in Kingdoms.
    pub version: u32,
    /// Word 1. Number of named entry points (functions).
    pub script_count: u32,
    /// Word 2. Number of piece names.
    pub piece_count: u32,
    /// Word 3. Code length in `u32` words.
    pub code_words: u32,
    /// Word 4. Number of static-variable slots. No data in the file.
    pub static_count: u32,
    /// Word 5. TA's trailing 8-byte record count. Always 0 in Kingdoms.
    pub trailing_record_count: u32,
    /// Word 6. `u32[script_count]` entry points, as code word indexes.
    pub script_entries_at: u32,
    /// Word 7. `u32[script_count]` offsets of script name strings.
    pub script_names_at: u32,
    /// Word 8. `u32[piece_count]` offsets of piece name strings.
    pub piece_names_at: u32,
    /// Word 9. Code start. Always 52.
    pub code_at: u32,
    /// Word 10. TA's trailing-record pointer. Equals `sound_names_at` in
    /// every retail Kingdoms file; nothing is read through it.
    pub trailing_records_at: u32,
    /// Word 11. `u32[sound_count]` offsets of sound name strings.
    pub sound_names_at: u32,
    /// Word 12. Number of sound names.
    pub sound_count: u32,
}

/// One named entry point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    /// Code word index where the function starts.
    pub entry: u32,
}

/// A parsed `.cob` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script {
    pub header: Header,
    /// In file order: `call-script`/`start-script` operands index this list.
    pub functions: Vec<Function>,
    /// Piece operands index this list (case-insensitive 3DO piece names).
    pub pieces: Vec<String>,
    /// Unit scripts: sound names for `play-sound`. Mission scripts: the
    /// command strings `mission-command` indexes (e.g. `create ARAARCH`).
    pub sounds: Vec<String>,
    pub static_count: u32,
    pub code: Vec<u32>,
}

impl Script {
    /// The first function with exactly this name. Retail files contain one
    /// duplicate (`walk_legs` in `npcpeas2.cob`); this returns the first.
    pub fn function(&self, name: &str) -> Option<usize> {
        self.functions.iter().position(|f| f.name == name)
    }

    /// Writes the canonical retail layout: header, code, entry table, name
    /// tables, then the strings. Lets tests (and the VM) build synthetic
    /// scripts. `header` is recomputed; the stored one is ignored.
    pub fn to_bytes(&self) -> Vec<u8> {
        let ns = self.functions.len() as u32;
        let np = self.pieces.len() as u32;
        let nsnd = self.sounds.len() as u32;
        let code_at = HEADER_LEN as u32;
        let entries_at = code_at + 4 * self.code.len() as u32;
        let names_at = entries_at + 4 * ns;
        let pieces_at = names_at + 4 * ns;
        let sounds_at = pieces_at + 4 * np;
        let strings_at = sounds_at + 4 * nsnd;

        let mut strings = Vec::new();
        let mut offs = |s: &str| {
            let o = strings_at + strings.len() as u32;
            strings.extend(s.bytes());
            strings.push(0);
            o
        };
        let name_offs: Vec<u32> = self.functions.iter().map(|f| offs(&f.name)).collect();
        let piece_offs: Vec<u32> = self.pieces.iter().map(|p| offs(p)).collect();
        let sound_offs: Vec<u32> = self.sounds.iter().map(|s| offs(s)).collect();

        let words = [
            VERSION,
            ns,
            np,
            self.code.len() as u32,
            self.static_count,
            0,
            entries_at,
            names_at,
            pieces_at,
            code_at,
            sounds_at,
            sounds_at,
            nsnd,
        ];
        let mut out = Vec::new();
        let mut put = |w: u32| out.extend(w.to_le_bytes());
        words.iter().for_each(|&w| put(w));
        self.code.iter().for_each(|&w| put(w));
        self.functions.iter().for_each(|f| put(f.entry));
        name_offs
            .iter()
            .chain(&piece_offs)
            .chain(&sound_offs)
            .for_each(|&w| put(w));
        out.extend(strings);
        while out.len() % 4 != 0 {
            out.push(0);
        }
        out
    }
}

/// Parses a COB v6 file. Validates every offset, count and string; does not
/// decode instructions (see [`decode`] and [`disassemble`]).
pub fn parse(data: &[u8]) -> Result<Script, CobError> {
    let mut r = Reader::new(data);
    let header = Header {
        version: r.u32("version")?,
        script_count: r.u32("script count")?,
        piece_count: r.u32("piece count")?,
        code_words: r.u32("code length")?,
        static_count: r.u32("static count")?,
        trailing_record_count: r.u32("trailing record count")?,
        script_entries_at: r.u32("script entry table offset")?,
        script_names_at: r.u32("script name table offset")?,
        piece_names_at: r.u32("piece name table offset")?,
        code_at: r.u32("code offset")?,
        trailing_records_at: r.u32("trailing record offset")?,
        sound_names_at: r.u32("sound name table offset")?,
        sound_count: r.u32("sound count")?,
    };
    if header.version != VERSION {
        return Err(CobError::Version(header.version));
    }
    // TA's trailing table: 8-byte records; Kingdoms has none, but bound it.
    table(
        data,
        header.trailing_records_at,
        u64::from(header.trailing_record_count) * 2,
        "trailing records",
    )?;
    let code = table(data, header.code_at, header.code_words.into(), "code")?;
    let entries = table(
        data,
        header.script_entries_at,
        header.script_count.into(),
        "script entry table",
    )?;
    let names = strings(
        data,
        header.script_names_at,
        header.script_count,
        "script name",
    )?;
    let pieces = strings(
        data,
        header.piece_names_at,
        header.piece_count,
        "piece name",
    )?;
    let sounds = strings(
        data,
        header.sound_names_at,
        header.sound_count,
        "sound name",
    )?;

    let mut functions = Vec::with_capacity(names.len());
    for (index, (name, entry)) in names.into_iter().zip(entries).enumerate() {
        if entry >= header.code_words {
            return Err(CobError::EntryOutOfRange {
                index,
                name,
                entry,
                code_words: header.code_words,
            });
        }
        functions.push(Function { name, entry });
    }
    Ok(Script {
        header,
        functions,
        pieces,
        sounds,
        static_count: header.static_count,
        code,
    })
}

fn table(data: &[u8], offset: u32, count: u64, what: &'static str) -> Result<Vec<u32>, CobError> {
    if offset % 4 != 0 {
        return Err(CobError::Misaligned { what, offset });
    }
    let mut r = Reader::at(data, offset.into(), what)?;
    let n = r.check_count(count, 4, what)?;
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        v.push(r.u32(what)?);
    }
    Ok(v)
}

fn strings(
    data: &[u8],
    offset: u32,
    count: u32,
    what: &'static str,
) -> Result<Vec<String>, CobError> {
    let offs = table(data, offset, count.into(), what)?;
    let mut v = Vec::with_capacity(offs.len());
    for (index, o) in offs.into_iter().enumerate() {
        let s = Reader::at(data, o.into(), what)?.cstr(what)?;
        if s.is_empty() {
            return Err(CobError::EmptyName { what, index });
        }
        v.push(s);
    }
    Ok(v)
}

/// What an inline operand word means. Drives the disassembler and tells the
/// VM how to interpret each word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operand {
    /// Index into [`Script::pieces`].
    Piece,
    /// 0 = x, 1 = y, 2 = z.
    Axis,
    /// Signed 32-bit immediate.
    Const,
    /// Local variable slot.
    Local,
    /// Static variable slot, `< static_count`.
    Static,
    /// Code word index of an instruction (jump target).
    Target,
    /// Index into [`Script::functions`].
    Function,
    /// Number of arguments popped from the stack and passed to a function.
    ArgCount,
    /// Index into [`Script::sounds`] (sound name).
    Sound,
    /// Index into [`Script::sounds`] (mission command string).
    MissionCommand,
    /// Signed count of stack values to discard.
    Count,
    /// Present in the encoding, ignored by the interpreter.
    Ignored,
}

/// How sure we are of an opcode's meaning in Kingdoms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// TA-documented and its operand shape is confirmed by every Kingdoms use.
    Known,
    /// Meaning inferred from Kingdoms usage; not documented anywhere.
    Believed,
    /// In the table for completeness; never seen in Kingdoms data.
    Unused,
}

macro_rules! ops {
    ($( $(#[$m:meta])* $name:ident = $raw:literal, $mn:literal, [$($k:ident),*], $conf:ident; )*) => {
        /// Every COB opcode Kingdoms is known to use, plus TA's unused ones.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[repr(u32)]
        pub enum Op { $( $(#[$m])* $name = $raw, )* }

        impl Op {
            /// All opcodes, in table order.
            pub const ALL: &'static [Op] = &[$(Op::$name,)*];

            pub fn from_raw(raw: u32) -> Option<Op> {
                match raw { $( $raw => Some(Op::$name), )* _ => None }
            }
            pub fn raw(self) -> u32 { self as u32 }
            pub fn mnemonic(self) -> &'static str {
                match self { $( Op::$name => $mn, )* }
            }
            /// Kinds of the inline operand words that follow the opcode.
            pub fn operands(self) -> &'static [Operand] {
                match self { $( Op::$name => &[$(Operand::$k),*], )* }
            }
            pub fn confidence(self) -> Confidence {
                match self { $( Op::$name => Confidence::$conf, )* }
            }
        }
    };
}

ops! {
    // Piece animation. Stack effects in docs/formats/cob.md.
    Move = 0x1000_1000, "move", [Piece, Axis], Known;
    Turn = 0x1000_2000, "turn", [Piece, Axis], Known;
    Spin = 0x1000_3000, "spin", [Piece, Axis], Known;
    StopSpin = 0x1000_4000, "stop-spin", [Piece, Axis], Known;
    Show = 0x1000_5000, "show", [Piece], Known;
    Hide = 0x1000_6000, "hide", [Piece], Known;
    Cache = 0x1000_7000, "cache", [Piece], Known;
    DontCache = 0x1000_8000, "dont-cache", [Piece], Known;
    /// TA "legacy effect": pops two values, no effect in TA.
    LegacyEffect = 0x1000_9000, "legacy-effect", [Piece], Unused;
    DontShadow = 0x1000_A000, "dont-shadow", [Piece], Known;
    MoveNow = 0x1000_B000, "move-now", [Piece, Axis], Known;
    TurnNow = 0x1000_C000, "turn-now", [Piece, Axis], Known;
    Shade = 0x1000_D000, "shade", [Piece], Known;
    DontShade = 0x1000_E000, "dont-shade", [Piece], Known;
    EmitSfx = 0x1000_F000, "emit-sfx", [Piece], Known;
    WaitForTurn = 0x1001_1000, "wait-for-turn", [Piece, Axis], Known;
    WaitForMove = 0x1001_2000, "wait-for-move", [Piece, Axis], Known;
    Sleep = 0x1001_3000, "sleep", [], Known;
    // Values and variables.
    PushConst = 0x1002_1001, "push-constant", [Const], Known;
    PushLocal = 0x1002_1002, "push-local", [Local], Known;
    PushStatic = 0x1002_1004, "push-static", [Static], Known;
    AllocLocal = 0x1002_2000, "alloc-local", [], Known;
    PopLocal = 0x1002_3002, "pop-local", [Local], Known;
    PopStatic = 0x1002_3004, "pop-static", [Static], Known;
    Discard = 0x1002_4000, "discard", [], Known;
    // Arithmetic: ( a b -- r ) unless noted.
    Add = 0x1003_1000, "add", [], Known;
    Sub = 0x1003_2000, "sub", [], Known;
    Mul = 0x1003_3000, "mul", [], Known;
    Div = 0x1003_4000, "div", [], Known;
    BitAnd = 0x1003_5000, "bit-and", [], Known;
    BitOr = 0x1003_6000, "bit-or", [], Known;
    BitXor = 0x1003_7000, "bit-xor", [], Unused;
    /// ( a -- ~a )
    BitNot = 0x1003_8000, "bit-not", [], Unused;
    /// Kingdoms-only in practice; Scriptor placeholder `??`. `a << b`.
    ShiftLeft = 0x1003_9000, "shift-left", [], Believed;
    /// Scriptor placeholder `???`. `a >> b`.
    ShiftRight = 0x1003_A000, "shift-right", [], Believed;
    /// Scriptor placeholder `????`. `a % b`.
    Mod = 0x1003_B000, "mod", [], Believed;
    // Engine queries.
    /// ( low high -- r )
    Rand = 0x1004_1000, "rand", [], Known;
    /// ( id -- value )
    GetUnitValue = 0x1004_2000, "get-unit-value", [], Known;
    /// ( id a b c d -- value )
    Get = 0x1004_3000, "get", [], Known;
    /// TA cargo-membership read ( unit -- bool ).
    CargoHas = 0x1004_4000, "cargo-has", [], Unused;
    /// TA carrier-identity read ( -- unit ).
    Carrier = 0x1004_5000, "carrier", [], Unused;
    // Comparison and logic: ( a b -- 0|1 ) unless noted.
    Lt = 0x1005_1000, "lt", [], Known;
    Le = 0x1005_2000, "le", [], Known;
    Gt = 0x1005_3000, "gt", [], Known;
    Ge = 0x1005_4000, "ge", [], Known;
    Eq = 0x1005_5000, "eq", [], Known;
    Ne = 0x1005_6000, "ne", [], Known;
    And = 0x1005_7000, "and", [], Known;
    Or = 0x1005_8000, "or", [], Known;
    Xor = 0x1005_9000, "xor", [], Unused;
    /// ( a -- !a )
    Not = 0x1005_A000, "not", [], Known;
    // Flow control.
    StartScript = 0x1006_1000, "start-script", [Function, ArgCount], Known;
    CallScript = 0x1006_2000, "call-script", [Function, ArgCount], Known;
    /// TA reserved pop-N: discards `count` stack values.
    PopN = 0x1006_3000, "pop-n", [Ignored, Count], Unused;
    Jump = 0x1006_4000, "jump", [Target], Known;
    Return = 0x1006_5000, "return", [], Known;
    JumpIfFalse = 0x1006_6000, "jump-if-false", [Target], Known;
    Signal = 0x1006_7000, "signal", [], Known;
    SetSignalMask = 0x1006_8000, "set-signal-mask", [], Known;
    /// ( flags -- )
    Explode = 0x1007_1000, "explode", [Piece], Known;
    /// Kingdoms only. ( priority -- result ); the result is always discarded.
    PlaySound = 0x1007_2000, "play-sound", [Sound], Known;
    /// Kingdoms only, mission scripts. ( args... -- result ).
    MissionCommand = 0x1007_3000, "mission-command", [MissionCommand, ArgCount], Known;
    /// ( id value -- )
    SetUnitValue = 0x1008_2000, "set", [], Known;
    /// ( unit piece extra -- )
    AttachUnit = 0x1008_3000, "attach-unit", [], Unused;
    /// ( unit -- )
    DropUnit = 0x1008_4000, "drop-unit", [], Unused;
}

/// One decoded instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instr {
    /// Word index of the opcode.
    pub addr: u32,
    pub op: Op,
    /// Inline operands; only the first `op.operands().len()` are meaningful.
    pub args: [u32; 2],
}

impl Instr {
    /// Word index of the next instruction.
    pub fn next(&self) -> u32 {
        self.addr + 1 + self.op.operands().len() as u32
    }
    /// The meaningful inline operand words.
    pub fn operands(&self) -> &[u32] {
        &self.args[..self.op.operands().len()]
    }
}

/// Decodes the instruction at word `addr` of `code`.
pub fn decode(code: &[u32], addr: usize) -> Result<Instr, DecodeError> {
    let &value = code.get(addr).ok_or(DecodeError::OutOfRange {
        addr,
        len: code.len(),
    })?;
    let op = Op::from_raw(value).ok_or(DecodeError::UnknownOpcode { addr, value })?;
    let need = op.operands().len();
    let have = code.len() - addr - 1;
    if need > have {
        return Err(DecodeError::Truncated {
            addr,
            op: op.mnemonic(),
            need,
            have,
        });
    }
    let mut args = [0; 2];
    args[..need].copy_from_slice(&code[addr + 1..addr + 1 + need]);
    Ok(Instr {
        addr: addr as u32,
        op,
        args,
    })
}

/// Appends an instruction's words to `code`. For building synthetic scripts.
pub fn encode(code: &mut Vec<u32>, op: Op, operands: &[u32]) {
    assert_eq!(
        operands.len(),
        op.operands().len(),
        "{} takes {} operands",
        op.mnemonic(),
        op.operands().len()
    );
    code.push(op.raw());
    code.extend_from_slice(operands);
}

/// A problem found while walking the code. The walk never skips silently:
/// an unknown word is reported and stepped over one word at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Issue {
    Decode(DecodeError),
    /// A jump target, entry point or function index that does not land on
    /// an instruction / existing entry.
    BadReference {
        addr: u32,
        what: &'static str,
        value: u32,
    },
}

impl std::fmt::Display for Issue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Issue::Decode(e) => e.fmt(f),
            Issue::BadReference { addr, what, value } => {
                write!(f, "bad {what} {value} at word {addr}")
            }
        }
    }
}

/// Linear decode of the whole code array.
#[derive(Debug, Clone, Default)]
pub struct Listing {
    pub instrs: Vec<Instr>,
    /// Unknown or truncated words, by address, as `(addr, raw)`.
    pub raw_words: Vec<(u32, u32)>,
    pub issues: Vec<Issue>,
}

/// Decodes `script.code` front to back and checks every reference. Retail
/// code is one contiguous instruction stream starting at word 0.
pub fn decode_all(script: &Script) -> Listing {
    let code = &script.code;
    let mut l = Listing::default();
    let mut addr = 0;
    while addr < code.len() {
        match decode(code, addr) {
            Ok(i) => {
                addr = i.next() as usize;
                l.instrs.push(i);
            }
            Err(e) => {
                l.raw_words.push((addr as u32, code[addr]));
                l.issues.push(Issue::Decode(e));
                addr += 1;
            }
        }
    }
    let starts: BTreeSet<u32> = l.instrs.iter().map(|i| i.addr).collect();
    for f in &script.functions {
        if !starts.contains(&f.entry) {
            l.issues.push(Issue::BadReference {
                addr: f.entry,
                what: "entry point",
                value: f.entry,
            });
        }
    }
    for i in &l.instrs {
        for (&kind, &v) in i.op.operands().iter().zip(i.operands()) {
            let (ok, what) = match kind {
                Operand::Target => (starts.contains(&v), "jump target"),
                Operand::Function => ((v as usize) < script.functions.len(), "function index"),
                Operand::Piece => ((v as usize) < script.pieces.len(), "piece index"),
                Operand::Axis => (v < 3, "axis"),
                Operand::Static => (v < script.static_count, "static index"),
                Operand::Sound | Operand::MissionCommand => {
                    ((v as usize) < script.sounds.len(), "sound index")
                }
                _ => (true, ""),
            };
            if !ok {
                l.issues.push(Issue::BadReference {
                    addr: i.addr,
                    what,
                    value: v,
                });
            }
        }
    }
    l
}

/// Disassembly text plus everything that went wrong.
#[derive(Debug, Clone)]
pub struct Disassembly {
    pub text: String,
    pub issues: Vec<Issue>,
}

/// Stable, diffable text: a header comment, the piece/sound tables, then
/// every instruction with function labels and `.Lnnnnn` jump labels.
pub fn disassemble(script: &Script) -> Disassembly {
    let listing = decode_all(script);
    let mut by_addr = std::collections::BTreeMap::<u32, Vec<usize>>::new();
    for (k, f) in script.functions.iter().enumerate() {
        by_addr.entry(f.entry).or_default().push(k);
    }
    let jump_targets: BTreeSet<u32> = listing
        .instrs
        .iter()
        .filter(|i| matches!(i.op.operands().first(), Some(Operand::Target)))
        .map(|i| i.args[0])
        .collect();
    // A jump to a function entry uses the function's name; others get `.L`.
    let label = |t: u32| -> String {
        match by_addr.get(&t) {
            Some(ks) => script.functions[ks[0]].name.clone(),
            None => format!(".L{t:05}"),
        }
    };

    let mut out = String::new();
    let h = &script.header;
    let _ = writeln!(
        out,
        "; cob v{}: {} functions, {} pieces, {} statics, {} sounds, {} code words",
        h.version,
        script.functions.len(),
        script.pieces.len(),
        script.static_count,
        script.sounds.len(),
        script.code.len()
    );
    for (k, p) in script.pieces.iter().enumerate() {
        let _ = writeln!(out, ".piece {k} {p}");
    }
    for (k, s) in script.sounds.iter().enumerate() {
        let _ = writeln!(out, ".sound {k} {s:?}");
    }
    for (k, f) in script.functions.iter().enumerate() {
        let _ = writeln!(out, ".function {k} {} @{:05}", f.name, f.entry);
    }

    // Interleave instructions and raw (unknown) words by address.
    let mut raw = listing.raw_words.iter().peekable();
    let emit_labels = |out: &mut String, addr: u32| {
        if let Some(ks) = by_addr.get(&addr) {
            for &k in ks {
                let _ = writeln!(out, "\n{}:", script.functions[k].name);
            }
        }
        if jump_targets.contains(&addr) && !by_addr.contains_key(&addr) {
            let _ = writeln!(out, ".L{addr:05}:");
        }
    };
    for i in &listing.instrs {
        while let Some(&&(a, w)) = raw.peek().filter(|(a, _)| *a < i.addr) {
            emit_labels(&mut out, a);
            let _ = writeln!(out, "  {a:05}  .word {w:#010x}  ; {}", raw_note(w));
            raw.next();
        }
        emit_labels(&mut out, i.addr);
        let ops: Vec<String> =
            i.op.operands()
                .iter()
                .zip(i.operands())
                .map(|(&k, &v)| operand_text(script, k, v, &label))
                .collect();
        let _ = writeln!(
            out,
            "  {:05}  {}{}{}",
            i.addr,
            i.op.mnemonic(),
            if ops.is_empty() { "" } else { " " },
            ops.join(", ")
        );
    }
    for &(a, w) in raw {
        emit_labels(&mut out, a);
        let _ = writeln!(out, "  {a:05}  .word {w:#010x}  ; {}", raw_note(w));
    }
    if !listing.issues.is_empty() {
        let _ = writeln!(out, "\n; {} issues", listing.issues.len());
        for e in &listing.issues {
            let _ = writeln!(out, ";   {e}");
        }
    }
    Disassembly {
        text: out,
        issues: listing.issues,
    }
}

fn raw_note(w: u32) -> &'static str {
    if Op::from_raw(w).is_some() {
        "TRUNCATED"
    } else {
        "UNKNOWN"
    }
}

fn operand_text(script: &Script, kind: Operand, v: u32, label: &dyn Fn(u32) -> String) -> String {
    let name = |list: &[String]| list.get(v as usize).cloned();
    match kind {
        Operand::Piece => name(&script.pieces).unwrap_or_else(|| format!("piece#{v}")),
        Operand::Axis => match v {
            0 => "x-axis".into(),
            1 => "y-axis".into(),
            2 => "z-axis".into(),
            _ => format!("axis#{v}"),
        },
        Operand::Const => (v as i32).to_string(),
        Operand::Local => format!("local[{v}]"),
        Operand::Static => format!("static[{v}]"),
        Operand::Target => label(v),
        Operand::Function => script
            .functions
            .get(v as usize)
            .map(|f| f.name.clone())
            .unwrap_or_else(|| format!("function#{v}")),
        Operand::ArgCount => format!("argc={v}"),
        Operand::Sound | Operand::MissionCommand => name(&script.sounds)
            .map(|s| format!("{s:?}"))
            .unwrap_or_else(|| format!("sound#{v}")),
        Operand::Count => format!("count={}", v as i32),
        Operand::Ignored => format!("{v:#x}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Op::*;

    fn func(name: &str, entry: u32) -> Function {
        Function {
            name: name.into(),
            entry,
        }
    }

    /// A hand-assembled walk cycle in the style of a Kingdoms infantry unit.
    fn synthetic() -> Script {
        let mut c = Vec::new();
        // Create: start-script walk; return 0
        encode(&mut c, StartScript, &[1, 0]);
        encode(&mut c, AllocLocal, &[]);
        encode(&mut c, Return, &[]);
        // walk: loop { turn leg to x <30> speed <90>; wait; sleep 100; sound; }
        let walk = c.len() as u32;
        encode(&mut c, AllocLocal, &[]);
        let top = c.len() as u32;
        encode(&mut c, PushStatic, &[0]);
        encode(&mut c, Not, &[]);
        encode(&mut c, JumpIfFalse, &[0]); // patched below
        let jf = c.len() - 1;
        encode(&mut c, PushConst, &[16380]);
        encode(&mut c, PushConst, &[5460]);
        encode(&mut c, Turn, &[1, 0]);
        encode(&mut c, WaitForTurn, &[1, 0]);
        encode(&mut c, PushConst, &[(-229376i32) as u32]);
        encode(&mut c, MoveNow, &[0, 1]);
        encode(&mut c, PushConst, &[4]);
        encode(&mut c, PlaySound, &[0]);
        encode(&mut c, Discard, &[]);
        encode(&mut c, PushConst, &[7]);
        encode(&mut c, PushConst, &[3]);
        encode(&mut c, Mod, &[]);
        encode(&mut c, PopLocal, &[0]);
        encode(&mut c, PushConst, &[100]);
        encode(&mut c, Sleep, &[]);
        encode(&mut c, Jump, &[top]);
        c[jf] = c.len() as u32;
        encode(&mut c, AllocLocal, &[]);
        encode(&mut c, Return, &[]);
        Script {
            header: parse_header_placeholder(),
            functions: vec![func("Create", 0), func("walk", walk)],
            pieces: vec!["base".into(), "lleg".into()],
            sounds: vec!["footstep".into()],
            static_count: 1,
            code: c,
        }
    }

    fn parse_header_placeholder() -> Header {
        Header {
            version: VERSION,
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
        }
    }

    #[test]
    fn opcode_table_is_consistent() {
        let mut seen = BTreeSet::new();
        for &op in Op::ALL {
            assert!(seen.insert(op.raw()), "duplicate {:#x}", op.raw());
            assert_eq!(Op::from_raw(op.raw()), Some(op));
            assert!(op.operands().len() <= 2);
        }
        assert_eq!(Op::from_raw(0x1007_2000), Some(PlaySound));
        assert_eq!(Op::from_raw(0), None);
    }

    #[test]
    fn round_trips_and_header_matches_retail_layout() {
        let s = synthetic();
        let bytes = s.to_bytes();
        let p = parse(&bytes).unwrap();
        assert_eq!(p.functions, s.functions);
        assert_eq!(p.pieces, s.pieces);
        assert_eq!(p.sounds, s.sounds);
        assert_eq!(p.code, s.code);
        assert_eq!(p.static_count, 1);
        let h = p.header;
        assert_eq!(h.code_at, 52);
        assert_eq!(h.script_entries_at, 52 + 4 * h.code_words);
        assert_eq!(h.trailing_records_at, h.sound_names_at);
        assert_eq!(p.to_bytes(), bytes);
        assert!(decode_all(&p).issues.is_empty());
    }

    #[test]
    fn disassembly_snapshot() {
        let p = parse(&synthetic().to_bytes()).unwrap();
        let d = disassemble(&p);
        assert!(d.issues.is_empty(), "{:?}", d.issues);
        insta::assert_snapshot!(d.text);
    }

    #[test]
    fn mission_command_snapshot() {
        let mut c = Vec::new();
        encode(&mut c, PushConst, &[150]);
        encode(&mut c, PushConst, &[68]);
        encode(&mut c, MissionCommand, &[0, 2]);
        encode(&mut c, PopStatic, &[0]);
        encode(&mut c, PushStatic, &[0]);
        encode(&mut c, MissionCommand, &[1, 1]);
        encode(&mut c, Discard, &[]);
        encode(&mut c, AllocLocal, &[]);
        encode(&mut c, Return, &[]);
        let s = Script {
            header: parse_header_placeholder(),
            functions: vec![func("Start", 0)],
            pieces: vec![],
            sounds: vec!["create TESTUNIT".into(), "SetTrigger".into()],
            static_count: 1,
            code: c,
        };
        let d = disassemble(&parse(&s.to_bytes()).unwrap());
        assert!(d.issues.is_empty());
        insta::assert_snapshot!(d.text);
    }

    #[test]
    fn unknown_and_bad_references_are_reported_not_skipped() {
        let mut s = synthetic();
        s.code = vec![
            PushConst.raw(),
            1,
            0xDEAD_BEEF,
            Jump.raw(),
            1, // middle of push-constant
            Turn.raw(),
            9,
            7,
            Move.raw(),
        ];
        s.functions = vec![func("Create", 0), func("bad", 1)];
        let d = disassemble(&parse(&s.to_bytes()).unwrap());
        let unknown = d
            .issues
            .iter()
            .filter(|e| matches!(e, Issue::Decode(DecodeError::UnknownOpcode { .. })))
            .count();
        assert_eq!(unknown, 1);
        assert!(
            d.issues
                .iter()
                .any(|e| matches!(e, Issue::Decode(DecodeError::Truncated { addr: 8, .. })))
        );
        assert!(d.issues.iter().any(|e| matches!(
            e,
            Issue::BadReference {
                what: "jump target",
                ..
            }
        )));
        assert!(d.issues.iter().any(|e| matches!(
            e,
            Issue::BadReference {
                what: "entry point",
                ..
            }
        )));
        assert!(d.issues.iter().any(|e| matches!(
            e,
            Issue::BadReference {
                what: "piece index",
                ..
            }
        )));
        insta::assert_snapshot!(d.text);
    }

    #[test]
    fn rejects_malformed_headers() {
        let good = synthetic().to_bytes();
        assert!(parse(&good[..40]).is_err());
        let mut v = good.clone();
        v[0] = 4;
        assert_eq!(parse(&v), Err(CobError::Version(4)));
        let set = |word: usize, val: u32| {
            let mut v = good.clone();
            v[word * 4..word * 4 + 4].copy_from_slice(&val.to_le_bytes());
            parse(&v)
        };
        assert!(set(1, u32::MAX).is_err()); // script count
        assert!(set(3, 0x4000_0000).is_err()); // code length
        assert!(set(6, 3).is_err()); // misaligned
        assert!(set(7, u32::MAX - 3).is_err());
        assert!(set(12, 1000).is_err());
        assert!(set(5, 0x1000_0000).is_err()); // trailing records
        // Entry point past the code.
        let entries_at = u32::from_le_bytes(good[24..28].try_into().unwrap()) as usize;
        let mut v = good.clone();
        v[entries_at..entries_at + 4].copy_from_slice(&9999u32.to_le_bytes());
        assert!(matches!(parse(&v), Err(CobError::EntryOutOfRange { .. })));
        // Unterminated string: chop the trailing NUL padding and last NUL.
        let mut v = good.clone();
        while v.last() == Some(&0) {
            v.pop();
        }
        assert!(parse(&v).is_err());
    }

    proptest::proptest! {
        #[test]
        fn never_panics_on_garbage(data in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..512)) {
            if let Ok(s) = parse(&data) {
                let _ = disassemble(&s);
            }
        }

        #[test]
        fn never_panics_on_mutations(idx in 0usize..4096, val in proptest::prelude::any::<u8>()) {
            let mut v = synthetic().to_bytes();
            let n = v.len();
            v[idx % n] = val;
            if let Ok(s) = parse(&v) {
                let _ = disassemble(&s);
            }
        }
    }

    /// Corpus: every retail script loads, decodes with zero unknown opcodes
    /// and zero bad references. Prints an opcode frequency table
    /// (`cargo test -p tak-formats cob -- --nocapture`).
    #[test]
    fn corpus_loads_and_decodes() {
        let Some(dir) = tak_vfs::testing::data_dir_or_skip("cob corpus") else {
            return;
        };
        let vfs = tak_vfs::open_kingdoms(&dir).unwrap();
        let files: Vec<_> = vfs
            .list("")
            .into_iter()
            .filter(|f| f.path.ends_with(".cob"))
            .collect();
        let mut freq = std::collections::BTreeMap::<Op, u64>::new();
        let mut failures = Vec::new();
        let (mut scripts, mut missions) = (0, 0);
        for f in &files {
            let data = vfs.read(&f.path).unwrap();
            match parse(&data) {
                Ok(s) => {
                    let l = decode_all(&s);
                    if !l.issues.is_empty() {
                        failures.push(format!("{}: {}", f.path, l.issues[0]));
                    }
                    for i in &l.instrs {
                        *freq.entry(i.op).or_default() += 1;
                    }
                    assert_eq!(s.header.trailing_record_count, 0, "{}", f.path);
                    assert_eq!(s.header.trailing_records_at, s.header.sound_names_at);
                }
                Err(e) => failures.push(format!("{}: {e}", f.path)),
            }
            if f.path.starts_with("scripts/") {
                scripts += 1;
            } else if f.path.starts_with("missions/") {
                missions += 1;
            }
        }
        let mut rows: Vec<_> = freq.iter().collect();
        rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        for (op, n) in rows {
            eprintln!("{n:>8}  {:#010x}  {}", op.raw(), op.mnemonic());
        }
        assert!(failures.is_empty(), "{failures:#?}");
        assert_eq!((files.len(), scripts, missions), (267, 204, 63));
    }
}
