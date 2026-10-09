//! Decoding of the 8-byte DVD navigation commands into a typed form.
//!
//! Field positions follow the mpucoder VM instruction tables. Where libdvdnav's
//! evaluator disagrees with those tables (instruction types 5 and 6, flagged
//! FIXME in its decoder.c), [`Quirks::libdvdnav_compound_cmds`] reproduces
//! libdvdnav's decoding so that differential tests can compare like for like.

use std::fmt;

use crate::ifo::RawCmd;

/// Register reference: bit 7 set means SPRM (low 5 bits), else GPRM (low 4 bits).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reg(pub u8);

impl Reg {
    pub fn gprm(n: u8) -> Self {
        Reg(n & 0x0f)
    }
}

impl fmt::Display for Reg {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.0 & 0x80 != 0 {
            write!(f, "s{}", self.0 & 0x1f)
        } else {
            write!(f, "g{}", self.0 & 0x0f)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operand {
    Reg(Reg),
    Imm(u16),
}

impl fmt::Display for Operand {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Operand::Reg(r) => write!(f, "{r}"),
            Operand::Imm(v) => write!(f, "{v}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    /// Bitwise test: `a & b != 0`.
    Bc,
    Eq,
    Ne,
    Ge,
    Gt,
    Le,
    Lt,
}

impl CmpOp {
    fn from_code(c: u8) -> Option<Self> {
        Some(match c {
            1 => CmpOp::Bc,
            2 => CmpOp::Eq,
            3 => CmpOp::Ne,
            4 => CmpOp::Ge,
            5 => CmpOp::Gt,
            6 => CmpOp::Le,
            7 => CmpOp::Lt,
            _ => return None,
        })
    }

    pub fn eval(self, a: u16, b: u16) -> bool {
        match self {
            CmpOp::Bc => a & b != 0,
            CmpOp::Eq => a == b,
            CmpOp::Ne => a != b,
            CmpOp::Ge => a >= b,
            CmpOp::Gt => a > b,
            CmpOp::Le => a <= b,
            CmpOp::Lt => a < b,
        }
    }

    fn symbol(self) -> &'static str {
        match self {
            CmpOp::Bc => "&",
            CmpOp::Eq => "==",
            CmpOp::Ne => "!=",
            CmpOp::Ge => ">=",
            CmpOp::Gt => ">",
            CmpOp::Le => "<=",
            CmpOp::Lt => "<",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cond {
    pub op: CmpOp,
    pub lhs: Reg,
    pub rhs: Operand,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetOp {
    Mov,
    Swp,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Rnd,
    And,
    Or,
    Xor,
}

impl SetOp {
    fn from_code(c: u8) -> Option<Self> {
        Some(match c {
            1 => SetOp::Mov,
            2 => SetOp::Swp,
            3 => SetOp::Add,
            4 => SetOp::Sub,
            5 => SetOp::Mul,
            6 => SetOp::Div,
            7 => SetOp::Mod,
            8 => SetOp::Rnd,
            9 => SetOp::And,
            10 => SetOp::Or,
            11 => SetOp::Xor,
            _ => return None,
        })
    }

    fn symbol(self) -> &'static str {
        match self {
            SetOp::Mov => "=",
            SetOp::Swp => "<->",
            SetOp::Add => "+=",
            SetOp::Sub => "-=",
            SetOp::Mul => "*=",
            SetOp::Div => "/=",
            SetOp::Mod => "%=",
            SetOp::Rnd => "= rnd",
            SetOp::And => "&=",
            SetOp::Or => "|=",
            SetOp::Xor => "^=",
        }
    }
}

/// `G[dst] op src`. For `Swp` with a register source, `src` is also written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Set {
    pub op: SetOp,
    pub dst: u8,
    pub src: Operand,
}

/// Link-subset operations (also the targets of most links).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkSub {
    Nop,
    TopC,
    NextC,
    PrevC,
    TopPg,
    NextPg,
    PrevPg,
    TopPgc,
    NextPgc,
    PrevPgc,
    GoUpPgc,
    TailPgc,
    Rsm,
}

impl LinkSub {
    fn from_code(c: u8) -> Option<Self> {
        Some(match c {
            0x00 => LinkSub::Nop,
            0x01 => LinkSub::TopC,
            0x02 => LinkSub::NextC,
            0x03 => LinkSub::PrevC,
            0x05 => LinkSub::TopPg,
            0x06 => LinkSub::NextPg,
            0x07 => LinkSub::PrevPg,
            0x09 => LinkSub::TopPgc,
            0x0a => LinkSub::NextPgc,
            0x0b => LinkSub::PrevPgc,
            0x0c => LinkSub::GoUpPgc,
            0x0d => LinkSub::TailPgc,
            0x10 => LinkSub::Rsm,
            _ => return None,
        })
    }
}

/// A transfer of control. `button` (where present) sets HL_BTNN if non-zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    Sub { op: LinkSub, button: u8 },
    Pgcn(u16),
    Pttn { pttn: u16, button: u8 },
    Pgn { pgn: u8, button: u8 },
    Cn { cn: u8, button: u8 },
    Exit,
    JumpTt(u8),
    JumpVtsTt(u8),
    JumpVtsPtt { ttn: u8, pttn: u16 },
    JumpSsFp,
    JumpSsVmgmMenu(u8),
    JumpSsVtsm { vts: u8, ttn: u8, menu: u8 },
    JumpSsVmgmPgc(u16),
    CallSsFp { rsm_cell: u8 },
    CallSsVmgmMenu { menu: u8, rsm_cell: u8 },
    CallSsVtsm { menu: u8, rsm_cell: u8 },
    CallSsVmgmPgc { pgcn: u16, rsm_cell: u8 },
}

/// SetSystem operations (instruction type 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetSystem {
    /// Set audio / subpicture / angle stream numbers (SPRM 1..=3).
    Stn { audio: Option<Operand>, subp: Option<Operand>, angle: Option<Operand> },
    /// Navigation timer (SPRM 9) and the PGC to jump to when it expires (SPRM 10).
    NvTmr { time: Operand, pgcn: u16 },
    /// Set a GPRM and its register/counter mode.
    GprmMd { reg: u8, value: Operand, counter: bool },
    /// Karaoke audio mixing mode (SPRM 11).
    AmxMd(Operand),
    /// Highlighted button (SPRM 8, already shifted: button << 10).
    HlBtnn(Operand),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Special {
    Nop,
    Goto(u8),
    Break,
    SetTmpPml { level: u8, line: u8 },
}

/// A decoded instruction. Execution order is encoded by the variant:
/// see [`crate::vm`] for the semantics of each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Instr {
    /// Type 0: `if (cond) special`.
    Special { cond: Option<Cond>, op: Special },
    /// Type 1: `if (cond) link`. `link` is `None` for NOP.
    Link { cond: Option<Cond>, link: Option<Link> },
    /// Type 2: `if (cond) { setsystem; link }`.
    SetSystem { cond: Option<Cond>, set: Option<SetSystem>, link: Option<Link> },
    /// Type 3: `if (cond) { set; link }`.
    Set { cond: Option<Cond>, set: Option<Set>, link: Option<Link> },
    /// Type 4: `set; if (cond) link`.
    SetCLnk { set: Option<Set>, cond: Option<Cond>, link: Option<Link> },
    /// Type 5: `if (cond) { set; link }`.
    CSetCLnk { cond: Option<Cond>, set: Option<Set>, link: Option<Link> },
    /// Type 6: `if (cond) set; link`.
    CmpSetLnk { cond: Option<Cond>, set: Option<Set>, link: Option<Link> },
    /// Type 7 or an undecodable command; executed as a NOP.
    Invalid,
}

/// Bit-level decoding quirks. Default is the spec (mpucoder) behaviour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Quirks {
    /// Decode types 5 and 6 the way libdvdnav's decoder.c does (compare
    /// `G[sr1]` against bytes 4..5, set source from bytes 2..3 / byte 3).
    pub libdvdnav_compound_cmds: bool,
}

/// Extract `count` bits whose most significant bit is bit `start` (63 = MSB).
fn bits(v: u64, start: u32, count: u32) -> u32 {
    ((v >> (start + 1 - count)) & ((1u64 << count) - 1)) as u32
}

fn cmp_op(v: u64) -> Option<CmpOp> {
    CmpOp::from_code(bits(v, 54, 3) as u8)
}

/// 16-bit immediate at `start` or 8-bit register at `start - 8`.
fn reg_or_data(v: u64, imm: bool, start: u32) -> Operand {
    if imm {
        Operand::Imm(bits(v, start, 16) as u16)
    } else {
        Operand::Reg(Reg(bits(v, start - 8, 8) as u8))
    }
}

/// Type 0/1(link)/2-link conditions: `reg(byte 3) op reg_or_data(bytes 4..5)`.
fn cond_v1(v: u64) -> Option<Cond> {
    Some(Cond { op: cmp_op(v)?, lhs: Reg(bits(v, 39, 8) as u8), rhs: reg_or_data(v, bits(v, 55, 1) != 0, 31) })
}

/// Jump / SetSystem conditions: `reg(byte 6) op reg(byte 7)`.
fn cond_v2(v: u64) -> Option<Cond> {
    Some(Cond {
        op: cmp_op(v)?,
        lhs: Reg(bits(v, 15, 8) as u8),
        rhs: Operand::Reg(Reg(bits(v, 7, 8) as u8)),
    })
}

/// Set conditions: `reg(byte 2) op reg_or_data(bytes 6..7)`.
fn cond_v3(v: u64) -> Option<Cond> {
    Some(Cond { op: cmp_op(v)?, lhs: Reg(bits(v, 47, 8) as u8), rhs: reg_or_data(v, bits(v, 55, 1) != 0, 15) })
}

/// Compound conditions: `G[byte 1 low nibble] op reg_or_data(bytes 4..5)`.
fn cond_v4(v: u64) -> Option<Cond> {
    Some(Cond { op: cmp_op(v)?, lhs: Reg::gprm(bits(v, 51, 4) as u8), rhs: reg_or_data(v, bits(v, 55, 1) != 0, 31) })
}

/// Types 5/6 per mpucoder: with an immediate set source the compare is
/// `reg(byte 4) op reg(byte 5)`, otherwise `reg(byte 3) op reg_or_data(bytes 4..5)`.
fn cond_v5(v: u64) -> Option<Cond> {
    let op = cmp_op(v)?;
    if bits(v, 60, 1) != 0 {
        Some(Cond { op, lhs: Reg(bits(v, 31, 8) as u8), rhs: Operand::Reg(Reg(bits(v, 23, 8) as u8)) })
    } else {
        Some(Cond { op, lhs: Reg(bits(v, 39, 8) as u8), rhs: reg_or_data(v, bits(v, 55, 1) != 0, 31) })
    }
}

/// Link subset in bytes 6..7 of compound instructions; `None` for an invalid code.
fn link_sub(v: u64) -> Option<Link> {
    let op = LinkSub::from_code(bits(v, 4, 5) as u8)?;
    Some(Link::Sub { op, button: bits(v, 15, 6) as u8 })
}

/// Link instruction selected by bits 51..48 (types 1, 2, 3).
fn link_instr(v: u64) -> Option<Link> {
    let button = bits(v, 15, 6) as u8;
    Some(match bits(v, 51, 4) {
        1 => {
            let op = LinkSub::from_code(bits(v, 4, 5) as u8)?;
            Link::Sub { op, button }
        }
        4 => Link::Pgcn(bits(v, 14, 15) as u16),
        5 => Link::Pttn { pttn: bits(v, 9, 10) as u16, button },
        6 => Link::Pgn { pgn: bits(v, 6, 7) as u8, button },
        7 => Link::Cn { cn: bits(v, 7, 8) as u8, button },
        _ => return None,
    })
}

fn jump_instr(v: u64) -> Option<Link> {
    let ss = bits(v, 23, 2);
    let rsm_cell = bits(v, 31, 8) as u8;
    Some(match bits(v, 51, 4) {
        1 => Link::Exit,
        2 => Link::JumpTt(bits(v, 22, 7) as u8),
        3 => Link::JumpVtsTt(bits(v, 22, 7) as u8),
        5 => Link::JumpVtsPtt { ttn: bits(v, 22, 7) as u8, pttn: bits(v, 41, 10) as u16 },
        6 => match ss {
            0 => Link::JumpSsFp,
            1 => Link::JumpSsVmgmMenu(bits(v, 19, 4) as u8),
            2 => Link::JumpSsVtsm {
                vts: bits(v, 31, 8) as u8,
                ttn: bits(v, 39, 8) as u8,
                menu: bits(v, 19, 4) as u8,
            },
            _ => Link::JumpSsVmgmPgc(bits(v, 46, 15) as u16),
        },
        8 => match ss {
            0 => Link::CallSsFp { rsm_cell },
            1 => Link::CallSsVmgmMenu { menu: bits(v, 19, 4) as u8, rsm_cell },
            2 => Link::CallSsVtsm { menu: bits(v, 19, 4) as u8, rsm_cell },
            _ => Link::CallSsVmgmPgc { pgcn: bits(v, 46, 15) as u16, rsm_cell },
        },
        _ => return None,
    })
}

fn set_system(v: u64) -> Option<SetSystem> {
    let imm = bits(v, 60, 1) != 0;
    Some(match bits(v, 59, 4) {
        1 => {
            // Enable flag at bit 39/31/23; value is 7-bit immediate or GPRM nibble.
            let field = |i: u32| -> Option<Operand> {
                let start = 47 - 8 * i;
                if bits(v, 63 - (2 + i) * 8, 1) == 0 {
                    None
                } else if imm {
                    Some(Operand::Imm(bits(v, start - 1, 7) as u16))
                } else {
                    Some(Operand::Reg(Reg::gprm(bits(v, start - 4, 4) as u8)))
                }
            };
            SetSystem::Stn { audio: field(1), subp: field(2), angle: field(3) }
        }
        2 => SetSystem::NvTmr { time: reg_or_data(v, imm, 47), pgcn: bits(v, 31, 16) as u16 },
        3 => SetSystem::GprmMd {
            reg: bits(v, 19, 4) as u8,
            value: reg_or_data(v, imm, 47),
            counter: bits(v, 23, 1) != 0,
        },
        4 => SetSystem::AmxMd(reg_or_data(v, imm, 31)),
        6 => SetSystem::HlBtnn(reg_or_data(v, imm, 31)),
        _ => return None,
    })
}

/// Set with destination `G[dst]`; source is 16-bit immediate at `imm_start`
/// or an 8-bit register whose position depends on the instruction type.
fn set(v: u64, dst: u32, src: Operand) -> Option<Set> {
    Some(Set { op: SetOp::from_code(bits(v, 59, 4) as u8)?, dst: bits(v, dst, 4) as u8, src })
}

pub fn decode(cmd: RawCmd, quirks: Quirks) -> Instr {
    let v = cmd.as_u64();
    let imm = bits(v, 60, 1) != 0;
    match bits(v, 63, 3) {
        0 => {
            let op = match bits(v, 51, 4) {
                0 => Special::Nop,
                1 => Special::Goto(bits(v, 7, 8) as u8),
                2 => Special::Break,
                3 => Special::SetTmpPml { level: bits(v, 11, 4) as u8, line: bits(v, 7, 8) as u8 },
                _ => return Instr::Invalid,
            };
            Instr::Special { cond: cond_v1(v), op }
        }
        1 if imm => Instr::Link { cond: cond_v2(v), link: jump_instr(v) },
        1 => Instr::Link { cond: cond_v1(v), link: link_instr(v) },
        2 => Instr::SetSystem { cond: cond_v2(v), set: set_system(v), link: link_instr(v) },
        3 => Instr::Set { cond: cond_v3(v), set: set(v, 35, reg_or_data(v, imm, 31)), link: link_instr(v) },
        4 => Instr::SetCLnk { set: set(v, 51, reg_or_data(v, imm, 47)), cond: cond_v4(v), link: link_sub(v) },
        5 | 6 => {
            let (cond, src) = if quirks.libdvdnav_compound_cmds {
                (cond_v4(v), reg_or_data(v, imm, 47))
            } else if imm {
                (cond_v5(v), Operand::Imm(bits(v, 47, 16) as u16))
            } else {
                (cond_v5(v), Operand::Reg(Reg(bits(v, 47, 8) as u8)))
            };
            let set = set(v, 51, src);
            if bits(v, 63, 3) == 5 {
                Instr::CSetCLnk { cond, set, link: link_sub(v) }
            } else {
                Instr::CmpSetLnk { cond, set, link: link_sub(v) }
            }
        }
        _ => Instr::Invalid,
    }
}

// ---- Disassembly (dvdauthor-flavoured pseudo code) ----

impl fmt::Display for Cond {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} {} {}", self.lhs, self.op.symbol(), self.rhs)
    }
}

impl fmt::Display for Set {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "g{} {} {}", self.dst, self.op.symbol(), self.src)
    }
}

fn button_suffix(f: &mut fmt::Formatter, button: u8) -> fmt::Result {
    if button != 0 {
        write!(f, " button {button}")?;
    }
    Ok(())
}

const MENU_NAMES: [&str; 8] = ["menu0", "menu1", "title", "root", "subtitle", "audio", "angle", "chapter"];

fn menu_name(m: u8) -> &'static str {
    MENU_NAMES.get(m as usize).copied().unwrap_or("menu?")
}

impl fmt::Display for Link {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Link::Sub { op, button } => {
                write!(f, "link {op:?}")?;
                button_suffix(f, button)
            }
            Link::Pgcn(n) => write!(f, "link pgc {n}"),
            Link::Pttn { pttn, button } => {
                write!(f, "link ptt {pttn}")?;
                button_suffix(f, button)
            }
            Link::Pgn { pgn, button } => {
                write!(f, "link program {pgn}")?;
                button_suffix(f, button)
            }
            Link::Cn { cn, button } => {
                write!(f, "link cell {cn}")?;
                button_suffix(f, button)
            }
            Link::Exit => write!(f, "exit"),
            Link::JumpTt(t) => write!(f, "jump title {t}"),
            Link::JumpVtsTt(t) => write!(f, "jump vts title {t}"),
            Link::JumpVtsPtt { ttn, pttn } => write!(f, "jump vts title {ttn} chapter {pttn}"),
            Link::JumpSsFp => write!(f, "jump fp"),
            Link::JumpSsVmgmMenu(m) => write!(f, "jump vmgm {}", menu_name(m)),
            Link::JumpSsVtsm { vts, ttn, menu } => write!(f, "jump vts {vts} title {ttn} {} menu", menu_name(menu)),
            Link::JumpSsVmgmPgc(p) => write!(f, "jump vmgm pgc {p}"),
            Link::CallSsFp { rsm_cell } => write!(f, "call fp resume {rsm_cell}"),
            Link::CallSsVmgmMenu { menu, rsm_cell } => write!(f, "call vmgm {} resume {rsm_cell}", menu_name(menu)),
            Link::CallSsVtsm { menu, rsm_cell } => write!(f, "call vtsm {} resume {rsm_cell}", menu_name(menu)),
            Link::CallSsVmgmPgc { pgcn, rsm_cell } => write!(f, "call vmgm pgc {pgcn} resume {rsm_cell}"),
        }
    }
}

impl fmt::Display for SetSystem {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            SetSystem::Stn { audio, subp, angle } => {
                let mut parts = vec![];
                if let Some(a) = audio {
                    parts.push(format!("s1 = {a}"));
                }
                if let Some(s) = subp {
                    parts.push(format!("s2 = {s}"));
                }
                if let Some(n) = angle {
                    parts.push(format!("s3 = {n}"));
                }
                write!(f, "{}", parts.join(", "))
            }
            SetSystem::NvTmr { time, pgcn } => write!(f, "s9 = {time}, s10 = {pgcn}"),
            SetSystem::GprmMd { reg, value, counter } => {
                write!(f, "g{reg} = {value}{}", if *counter { " (counter)" } else { "" })
            }
            SetSystem::AmxMd(v) => write!(f, "s11 = {v}"),
            SetSystem::HlBtnn(v) => write!(f, "s8 = {v}"),
        }
    }
}

fn write_link(f: &mut fmt::Formatter, link: &Option<Link>) -> fmt::Result {
    match link {
        Some(l) => write!(f, "{l}"),
        None => write!(f, "nop"),
    }
}

fn write_if(f: &mut fmt::Formatter, cond: &Option<Cond>) -> fmt::Result {
    if let Some(c) = cond {
        write!(f, "if ({c}) ")?;
    }
    Ok(())
}

impl fmt::Display for Instr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Instr::Special { cond, op } => {
                write_if(f, cond)?;
                match op {
                    Special::Nop => write!(f, "nop"),
                    Special::Goto(l) => write!(f, "goto {l}"),
                    Special::Break => write!(f, "break"),
                    Special::SetTmpPml { level, line } => write!(f, "set parental level {level}, goto {line}"),
                }
            }
            Instr::Link { cond, link } => {
                write_if(f, cond)?;
                match link {
                    Some(l) => write!(f, "{l}"),
                    None => write!(f, "nop"),
                }
            }
            Instr::SetSystem { cond, set, link } => {
                write_if(f, cond)?;
                let mut parts = vec![];
                if let Some(s) = set {
                    parts.push(s.to_string());
                }
                if let Some(l) = link {
                    parts.push(l.to_string());
                }
                if parts.is_empty() {
                    parts.push("nop".into());
                }
                write!(f, "{}", parts.join("; "))
            }
            Instr::Set { cond, set, link } => {
                write_if(f, cond)?;
                let mut parts = vec![];
                if let Some(s) = set {
                    parts.push(s.to_string());
                }
                if let Some(l) = link {
                    parts.push(l.to_string());
                }
                if parts.is_empty() {
                    parts.push("nop".into());
                }
                write!(f, "{}", parts.join("; "))
            }
            Instr::SetCLnk { set, cond, link } => {
                if let Some(s) = set {
                    write!(f, "{s}; ")?;
                }
                write_if(f, cond)?;
                write_link(f, link)
            }
            Instr::CSetCLnk { cond, set, link } => {
                write_if(f, cond)?;
                write!(f, "{{ ")?;
                if let Some(s) = set {
                    write!(f, "{s}; ")?;
                }
                write_link(f, link)?;
                write!(f, " }}")
            }
            Instr::CmpSetLnk { cond, set, link } => {
                write_if(f, cond)?;
                write!(f, "{{ ")?;
                if let Some(s) = set {
                    write!(f, "{s}; ")?;
                }
                write!(f, "}} ")?;
                write_link(f, link)
            }
            Instr::Invalid => write!(f, "invalid"),
        }
    }
}
