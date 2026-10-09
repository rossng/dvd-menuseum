//! Register file and command-list evaluation.
//!
//! Arithmetic edge cases (saturating add/mul, flooring sub, division by zero
//! yielding 0xffff, RND range 1..=n) follow libdvdnav, the only implementation
//! validated against a large body of real discs.

use crate::cmd::{decode, Cond, Instr, Link, Operand, Quirks, Reg, Set, SetOp, SetSystem, Special};
use crate::ifo::RawCmd;

pub const SPRM_COUNT: usize = 24;

/// Deterministic random source for `RND` and random-playback PGCs.
///
/// Produces 31-bit values; the oracle build of libdvdnav uses the same
/// generator in place of `rand()` so traces stay comparable.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    /// SplitMix64, truncated to 31 bits.
    pub fn next_u31(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 33) as u32
    }

    /// Uniform value in `0..n`, computed as `floor(n * r / 2^31)` like libdvdnav's
    /// `n * rand() / (RAND_MAX + 1.0)`.
    pub fn below(&mut self, n: u32) -> u32 {
        ((n as u64 * self.next_u31() as u64) >> 31) as u32
    }
}

#[derive(Clone, Debug)]
pub struct Registers {
    pub sprm: [u16; SPRM_COUNT],
    pub gprm: [u16; 16],
    /// GPRMs in counter mode count seconds since they were last set.
    pub gprm_counter: [bool; 16],
    /// For counter-mode GPRMs: the clock time (ms) at which the counter read 0.
    gprm_epoch: [u64; 16],
}

impl Default for Registers {
    /// Power-on values, matching libdvdnav's `vm_reset`.
    fn default() -> Self {
        let mut sprm = [0u16; SPRM_COUNT];
        sprm[0] = u16::from_be_bytes(*b"en"); // menu language
        sprm[1] = 15; // audio stream
        sprm[2] = 62; // subpicture stream
        sprm[3] = 1; // angle
        sprm[4] = 1; // title
        sprm[5] = 1; // VTS title
        sprm[7] = 1; // part of title
        sprm[8] = 1 << 10; // highlighted button
        sprm[12] = u16::from_be_bytes(*b"US"); // parental country
        sprm[13] = 15; // parental level
        sprm[14] = 0x100; // video preference: pan&scan
        sprm[15] = 0x7cfc; // audio capabilities
        sprm[16] = u16::from_be_bytes(*b"en"); // initial audio language
        sprm[18] = u16::from_be_bytes(*b"en"); // initial subpicture language
        sprm[20] = 1; // region mask
        Registers { sprm, gprm: [0; 16], gprm_counter: [false; 16], gprm_epoch: [0; 16] }
    }
}

impl Registers {
    pub fn gprm(&mut self, n: u8, now_ms: u64) -> u16 {
        let n = (n & 0x0f) as usize;
        if self.gprm_counter[n] {
            self.gprm[n] = (now_ms.saturating_sub(self.gprm_epoch[n]) / 1000) as u16;
        }
        self.gprm[n]
    }

    pub fn set_gprm(&mut self, n: u8, v: u16, now_ms: u64) {
        let n = (n & 0x0f) as usize;
        if self.gprm_counter[n] {
            self.gprm_epoch[n] = now_ms.saturating_sub(v as u64 * 1000);
        }
        self.gprm[n] = v;
    }

    pub fn sprm(&self, n: u8) -> u16 {
        self.sprm.get(n as usize).copied().unwrap_or(0)
    }

    pub fn reg(&mut self, r: Reg, now_ms: u64) -> u16 {
        if r.0 & 0x80 != 0 {
            self.sprm(r.0 & 0x1f)
        } else {
            self.gprm(r.0, now_ms)
        }
    }

    /// Highlighted button number (SPRM 8 holds it shifted left by 10).
    pub fn button(&self) -> u8 {
        (self.sprm[8] >> 10) as u8
    }

    pub fn set_button(&mut self, n: u8) {
        self.sprm[8] = (n as u16) << 10;
    }
}

/// Environment for evaluating commands: registers plus the clock, randomness
/// and decoding quirks.
pub struct Ctx<'a> {
    pub regs: &'a mut Registers,
    pub rng: &'a mut Rng,
    pub now_ms: u64,
    pub quirks: Quirks,
}

/// Maximum instructions per command list before assuming an infinite loop.
const MAX_STEPS: u32 = 100_000;

impl Ctx<'_> {
    fn operand(&mut self, o: Operand) -> u16 {
        match o {
            Operand::Imm(v) => v,
            Operand::Reg(r) => self.regs.reg(r, self.now_ms),
        }
    }

    fn cond(&mut self, c: &Option<Cond>) -> bool {
        match c {
            None => true,
            Some(c) => {
                let a = self.regs.reg(c.lhs, self.now_ms);
                let b = self.operand(c.rhs);
                c.op.eval(a, b)
            }
        }
    }

    fn set(&mut self, s: &Set) {
        let now = self.now_ms;
        let data = self.operand(s.src) as u32;
        let cur = self.regs.gprm(s.dst, now) as u32;
        let v = match s.op {
            SetOp::Mov => data,
            SetOp::Swp => {
                if let Operand::Reg(r) = s.src {
                    self.regs.set_gprm(r.0 & 0x0f, cur as u16, now);
                }
                data
            }
            SetOp::Add => (cur + data).min(0xffff),
            SetOp::Sub => cur.saturating_sub(data),
            SetOp::Mul => (cur * data).min(0xffff),
            SetOp::Div => cur.checked_div(data).unwrap_or(0xffff),
            SetOp::Mod => cur.checked_rem(data).unwrap_or(0xffff),
            SetOp::Rnd => 1 + self.rng.below(data),
            SetOp::And => cur & data,
            SetOp::Or => cur | data,
            SetOp::Xor => cur ^ data,
        };
        self.regs.set_gprm(s.dst, v as u16, now);
    }

    fn set_system(&mut self, s: &SetSystem, cond: bool) {
        match *s {
            SetSystem::Stn { audio, subp, angle } => {
                for (i, o) in [(1, audio), (2, subp), (3, angle)] {
                    if let Some(o) = o {
                        let v = self.operand(o);
                        if cond {
                            self.regs.sprm[i] = v;
                        }
                    }
                }
            }
            SetSystem::NvTmr { time, pgcn } => {
                let t = self.operand(time);
                if cond {
                    self.regs.sprm[9] = t;
                    self.regs.sprm[10] = pgcn;
                }
            }
            SetSystem::GprmMd { reg, value, counter } => {
                let v = self.operand(value);
                // libdvdnav switches the mode even when the condition is false.
                self.regs.gprm_counter[reg as usize & 0x0f] = counter;
                if cond {
                    self.regs.set_gprm(reg, v, self.now_ms);
                }
            }
            SetSystem::AmxMd(v) => {
                let v = self.operand(v);
                if cond {
                    self.regs.sprm[11] = v;
                }
            }
            SetSystem::HlBtnn(v) => {
                let v = self.operand(v);
                if cond {
                    self.regs.sprm[8] = v;
                }
            }
        }
    }

    /// Execute one instruction. Returns the link to follow (which ends the
    /// command list) or the 1-based line to go to (256 = break).
    fn step(&mut self, instr: &Instr) -> Step {
        match instr {
            Instr::Special { cond, op } => {
                let c = self.cond(cond);
                match *op {
                    Special::Nop => Step::Next,
                    Special::Goto(line) if c && line > 0 => Step::Goto(line as u16),
                    Special::Break if c => Step::Goto(256),
                    Special::SetTmpPml { level, line } if c => {
                        self.regs.sprm[13] = level as u16;
                        if line > 0 {
                            Step::Goto(line as u16)
                        } else {
                            Step::Next
                        }
                    }
                    _ => Step::Next,
                }
            }
            Instr::Link { cond, link } => {
                let c = self.cond(cond);
                match link {
                    Some(l) if c => Step::Link(*l),
                    _ => Step::Next,
                }
            }
            Instr::SetSystem { cond, set, link } => {
                let c = self.cond(cond);
                if let Some(s) = set {
                    self.set_system(s, c);
                }
                match link {
                    Some(l) if c => Step::Link(*l),
                    _ => Step::Next,
                }
            }
            Instr::Set { cond, set, link } => {
                let c = self.cond(cond);
                if c {
                    if let Some(s) = set {
                        self.set(s);
                    }
                }
                match link {
                    Some(l) if c => Step::Link(*l),
                    _ => Step::Next,
                }
            }
            Instr::SetCLnk { set, cond, link } => {
                if let Some(s) = set {
                    self.set(s);
                }
                match link {
                    Some(l) if self.cond(cond) => Step::Link(*l),
                    _ => Step::Next,
                }
            }
            Instr::CSetCLnk { cond, set, link } => {
                if !self.cond(cond) {
                    return Step::Next;
                }
                if let Some(s) = set {
                    self.set(s);
                }
                link.map_or(Step::Next, Step::Link)
            }
            Instr::CmpSetLnk { cond, set, link } => {
                if self.cond(cond) {
                    if let Some(s) = set {
                        self.set(s);
                    }
                }
                link.map_or(Step::Next, Step::Link)
            }
            Instr::Invalid => Step::Next,
        }
    }

    /// Run a command list (pre, post, cell or button commands). Returns the
    /// link that ended it, or `None` if execution fell off the end.
    pub fn run(&mut self, cmds: &[RawCmd]) -> Option<Link> {
        let mut i = 0usize;
        let mut steps = 0;
        while i < cmds.len() && steps < MAX_STEPS {
            match self.step(&decode(cmds[i], self.quirks)) {
                Step::Link(l) => return Some(l),
                Step::Goto(line) => i = line as usize - 1,
                Step::Next => i += 1,
            }
            steps += 1;
        }
        None
    }
}

enum Step {
    Next,
    Goto(u16),
    Link(Link),
}
