// SPDX-License-Identifier: GPL-2.0-or-later
//
// Derived from libdvdnav (src/dvdnav.c, highlight.c, searching.c and
// src/vm/vm.c, play.c, getset.c, vmget.c), which carry:
//   Copyright (C) 2000, 2001 Håkan Hjort
//   Copyright (C) 2000, 2001 Rich Wareham <richwareham@users.sourceforge.net>
//
// This program is free software; you can redistribute it and/or modify it
// under the terms of the GNU General Public License as published by the Free
// Software Foundation; either version 2 of the License, or (at your option)
// any later version. See COPYING at the repository root.

//! The navigation engine: domains, program chains, cells, resume state and the
//! VOBU-level playback loop.
//!
//! Structure and behaviour follow libdvdnav (vm.c, play.c, getset.c, dvdnav.c,
//! highlight.c) closely so that event traces can be compared against it. The
//! one structural difference is granularity: libdvdnav hands out every 2048-byte
//! block, this engine only visits the NAV pack at the start of each VOBU.
//!
//! Places where libdvdnav would trip an assertion or read out of bounds are
//! handled defensively and marked `// libdvdnav: ...`.

use menuseum_core::cmd::{Link, LinkSub, Quirks};
use menuseum_core::ifo::{Pgc, PgciUt, Pgcit, VideoAttr};
use menuseum_core::nav::{Dsi, Pci, SRI_END_OF_CELL};
use menuseum_core::navigator::{Disc, Domain, Event, MenuCall, NavRead, NavSource, Navigator, Status, Vobs};
use menuseum_core::vm::{Ctx, Registers, Rng};

/// Identity of the PGC being played: which table it came from and its index.
/// This mirrors libdvdnav's `state.pgc` pointer, which can (on odd discs)
/// disagree with the current domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PgcSpace {
    FirstPlay,
    /// VMGM menu language unit index.
    Vmgm(usize),
    /// VTSM: (vts, language unit index).
    Vtsm(u8, usize),
    Title(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PgcRef {
    space: PgcSpace,
    /// 1-based index within the table (0 for the first-play PGC).
    pgcn: u16,
}

#[derive(Clone, Debug, Default)]
struct Resume {
    vtsn: u8,
    blockn: u32,
    /// SPRM 4..=8.
    regs: [u16; 5],
    pgcn: u16,
    celln: u8,
}

/// VM state; cheap to clone so speculative jumps (menu calls) can be rolled back.
#[derive(Clone, Debug)]
pub struct VmState {
    pub regs: Registers,
    pub domain: Domain,
    /// 0 = no VTS opened yet.
    pub vtsn: u8,
    pgc: Option<PgcRef>,
    pub pgn: u8,
    pub celln: u8,
    pub cell_restart: u32,
    pub blockn: u32,
    rsm: Resume,
    pub stopped: bool,
    pub hop_channel: u32,
    rng: Rng,
}

/// Where control goes after a navigation step.
enum Flow {
    Link(Link),
    /// Start playing the current cell at this block offset.
    Play(u32),
}

const NO_LINK: Flow = Flow::Link(Link::Sub { op: LinkSub::Nop, button: 0 });

/// Mirrors libdvdnav's `vm_position_t`. `-1` forces a change event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Position {
    button: i32,
    vts: i32,
    domain: i32,
    spu_channel: i32,
    audio_channel: i32,
    hop_channel: u32,
    cell: i32,
    cell_restart: u32,
    cell_start: u32,
    still: u32,
    block: u32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Vobu {
    start: u32,
    next: u32,
    length: u32,
    blockn: u32,
}

pub struct Engine {
    disc: Disc,
    pub vm: VmState,
    pub quirks: Quirks,
    /// Clock for counter-mode GPRMs, in milliseconds. Advanced by the host.
    pub now_ms: u64,
    started: bool,
    file_open: bool,
    cur: Position,
    vobu: Vobu,
    pci: Pci,
    dsi: Dsi,
    last_cmd_nav_lbn: u32,
    sync_wait: bool,
    sync_wait_skip: bool,
    skip_still: bool,
    spu_clut_changed: bool,
}

impl VmState {
    fn new(seed: u64) -> Self {
        VmState {
            regs: Registers::default(),
            domain: Domain::FirstPlay,
            vtsn: 0,
            pgc: None,
            pgn: 0,
            celln: 0,
            cell_restart: 0,
            blockn: 0,
            rsm: Resume::default(),
            stopped: false,
            hop_channel: 0,
            rng: Rng::new(seed),
        }
    }
}

/// Navigation operations on the VM state (libdvdnav vm.c / play.c / getset.c).
struct Nav<'a> {
    disc: &'a Disc,
    s: &'a mut VmState,
    quirks: Quirks,
    now_ms: u64,
}

impl<'a> Nav<'a> {
    // ---- table lookups (getset.c) ----

    fn menu_lu(ut: &PgciUt, lang: u16) -> Option<usize> {
        if ut.lus.is_empty() {
            return None;
        }
        Some(ut.lus.iter().position(|lu| lu.lang_code == lang).unwrap_or(0))
    }

    /// The PGC table of the current domain (`get_PGCIT`).
    fn domain_space(&self) -> Option<PgcSpace> {
        let lang = self.s.regs.sprm[0];
        match self.s.domain {
            Domain::Title => {
                self.disc.vts(self.s.vtsn)?;
                Some(PgcSpace::Title(self.s.vtsn))
            }
            Domain::Vtsm => {
                let ut = self.disc.vts(self.s.vtsn)?.menu_pgci_ut.as_ref()?;
                Some(PgcSpace::Vtsm(self.s.vtsn, Self::menu_lu(ut, lang)?))
            }
            Domain::Vmgm | Domain::FirstPlay => {
                let ut = self.disc.vmgi.menu_pgci_ut.as_ref()?;
                Some(PgcSpace::Vmgm(Self::menu_lu(ut, lang)?))
            }
        }
    }

    fn pgcit(disc: &'a Disc, space: PgcSpace) -> Option<&'a Pgcit> {
        match space {
            PgcSpace::FirstPlay => None,
            PgcSpace::Vmgm(lu) => Some(&disc.vmgi.menu_pgci_ut.as_ref()?.lus.get(lu)?.pgcit),
            PgcSpace::Vtsm(vts, lu) => Some(&disc.vts(vts)?.menu_pgci_ut.as_ref()?.lus.get(lu)?.pgcit),
            PgcSpace::Title(vts) => Some(&disc.vts(vts)?.pgcit),
        }
    }

    fn pgc(&self) -> Option<&'a Pgc> {
        let r = self.s.pgc?;
        match r.space {
            PgcSpace::FirstPlay => self.disc.vmgi.first_play_pgc.as_ref(),
            space => Self::pgcit(self.disc, space)?.pgc(r.pgcn),
        }
    }

    /// `get_PGCN`: the current PGC's number in the current domain's table, or 0.
    fn pgcn(&self) -> u16 {
        match (self.s.pgc, self.domain_space()) {
            (Some(r), Some(space)) if r.space == space => r.pgcn,
            _ => 0,
        }
    }

    fn set_pgcn(&mut self, pgcn: u16) -> bool {
        let Some(space) = self.domain_space() else { return false };
        let Some(pgcit) = Self::pgcit(self.disc, space) else { return false };
        if pgcn < 1 || pgcn as usize > pgcit.srps.len() {
            return false;
        }
        self.s.pgc = Some(PgcRef { space, pgcn });
        self.s.pgn = 1;
        if self.s.domain == Domain::Title {
            self.s.regs.sprm[6] = pgcn;
        }
        true
    }

    fn set_fp_pgc(&mut self) -> bool {
        self.s.domain = Domain::FirstPlay;
        if self.disc.vmgi.first_play_pgc.is_none() {
            return self.set_pgcn(1);
        }
        self.s.pgc = Some(PgcRef { space: PgcSpace::FirstPlay, pgcn: 0 });
        true
    }

    fn menu_pgcn(&self, menu: u8) -> u16 {
        self.domain_space()
            .and_then(|sp| Self::pgcit(self.disc, sp))
            .and_then(|t| t.menu_pgcn(menu))
            .unwrap_or(0)
    }

    fn set_menu(&mut self, menu: u8) -> bool {
        let pgcn = self.menu_pgcn(menu);
        self.set_pgcn(pgcn)
    }

    /// Open a title set (libdvdnav `ifoOpenNewVTSI`).
    fn open_vts(&mut self, vtsn: u8) -> bool {
        if self.disc.vts(vtsn).is_none() {
            return false;
        }
        self.s.vtsn = vtsn;
        true
    }

    fn get_tt(&self, vtsn: u8, vts_ttn: u8) -> u16 {
        self.disc.vmgi.ttn_of(vtsn, vts_ttn)
    }

    fn set_vts_ptt(&mut self, vtsn: u8, vts_ttn: u8, part: u16) -> bool {
        self.s.domain = Domain::Title;
        if vtsn != self.s.vtsn && !self.open_vts(vtsn) {
            return false;
        }
        let Some(vtsi) = self.disc.vts(self.s.vtsn) else { return false };
        let Some(ptt) = vtsi.ptt(vts_ttn, part) else { return false };
        self.s.regs.sprm[6] = ptt.pgcn;
        self.s.regs.sprm[7] = part;
        let ttn = self.get_tt(vtsn, vts_ttn);
        self.s.regs.sprm[4] = ttn;
        if ttn == 0 {
            return false;
        }
        self.s.regs.sprm[5] = vts_ttn as u16;
        self.s.vtsn = vtsn;
        let res = self.set_pgcn(ptt.pgcn);
        self.s.pgn = ptt.pgn as u8;
        res
    }

    fn set_tt(&mut self, ttn: u16) -> bool {
        let Some(t) = self.disc.vmgi.title(ttn).copied() else { return false };
        self.set_vts_ptt(t.title_set_nr, t.vts_ttn, 1)
    }

    /// `vm_get_current_title_part`, including its blind spot past the last chapter.
    fn title_part(&self) -> Option<(u16, u16)> {
        let vtsi = self.disc.vts(self.s.vtsn)?;
        let pgcn = self.pgcn();
        let pgn = self.s.pgn as u16;
        for (ti, ptts) in vtsi.ptts.iter().enumerate() {
            for (pi, ptt) in ptts.iter().enumerate() {
                if ptt.pgcn != pgcn {
                    continue;
                }
                let part = if ptt.pgn == pgn {
                    pi
                } else if pi > 0 && ptt.pgn > pgn && ptts[pi - 1].pgn < pgn {
                    pi - 1
                } else {
                    continue;
                };
                let title = self.get_tt(self.s.vtsn, ti as u8 + 1);
                return Some((title, part as u16 + 1));
            }
        }
        None
    }

    /// `set_PGN`: derive the program (and in titles, the chapter) from the cell.
    fn set_pgn(&mut self) -> bool {
        let Some(pgc) = self.pgc() else { return false };
        let mut new_pgn = 0usize;
        while new_pgn < pgc.program_map.len() && self.s.celln >= pgc.program_map[new_pgn] {
            new_pgn += 1;
        }
        if new_pgn == pgc.program_map.len() && self.s.celln > pgc.nr_of_cells {
            return false;
        }
        self.s.pgn = new_pgn as u8;
        if self.s.domain == Domain::Title {
            if self.s.regs.sprm[4] as usize > self.disc.vmgi.titles.len() {
                return false;
            }
            self.s.regs.sprm[7] = self.title_part().map_or(0, |(_, part)| part);
        }
        true
    }

    fn set_rsm_info(&mut self, celln: u8, blockn: u32) {
        self.s.rsm.celln = if celln != 0 { celln } else { self.s.celln };
        self.s.rsm.blockn = blockn;
        self.s.rsm.vtsn = self.s.vtsn;
        self.s.rsm.pgcn = self.pgcn();
        self.s.rsm.regs.copy_from_slice(&self.s.regs.sprm[4..9]);
    }

    fn run(&mut self, cmds: &[menuseum_core::ifo::RawCmd]) -> Option<Link> {
        let mut ctx = Ctx { regs: &mut self.s.regs, rng: &mut self.s.rng, now_ms: self.now_ms, quirks: self.quirks };
        ctx.run(cmds)
    }

    fn set_button(&mut self, button: u8) {
        if button != 0 {
            self.s.regs.set_button(button);
        }
    }

    // ---- play.c ----

    fn play_pgc(&mut self) -> Flow {
        self.s.pgn = 1;
        self.s.celln = 0;
        self.s.blockn = 0;
        let Some(pgc) = self.pgc() else { return NO_LINK };
        // Random playback: pick the initial program at random.
        let mode = pgc.pg_playback_mode;
        if mode != 0 && mode & 0x80 == 0 {
            let count = ((mode & 0x7f) as u32 + 1).min(pgc.nr_of_programs as u32);
            if count > 1 {
                self.s.pgn = 1 + self.s.rng.below(count) as u8;
            }
        }
        if let Some(link) = self.run(&pgc.pre_cmds) {
            return Flow::Link(link);
        }
        self.play_pg()
    }

    fn play_pgc_pg(&mut self, pgn: u8) -> Flow {
        self.s.pgn = pgn;
        self.s.celln = 0;
        self.s.blockn = 0;
        let Some(pgc) = self.pgc() else { return NO_LINK };
        if let Some(link) = self.run(&pgc.pre_cmds) {
            return Flow::Link(link);
        }
        self.play_pg()
    }

    fn play_pgc_post(&mut self) -> Flow {
        let Some(pgc) = self.pgc() else { return Flow::Link(Link::Exit) };
        if let Some(link) = self.run(&pgc.post_cmds) {
            return Flow::Link(link);
        }
        if !self.set_pgcn(pgc.next_pgc_nr) {
            return Flow::Link(Link::Exit);
        }
        self.play_pgc()
    }

    fn play_pg(&mut self) -> Flow {
        let Some(pgc) = self.pgc() else { return NO_LINK };
        if self.s.pgn == 0 {
            return NO_LINK; // libdvdnav: assert(pgN > 0)
        }
        if self.s.pgn > pgc.nr_of_programs {
            if self.s.pgn > pgc.nr_of_programs + 1 {
                return NO_LINK; // bogus link, ignored
            }
            return self.play_pgc_post();
        }
        self.s.celln = pgc.program_map[self.s.pgn as usize - 1];
        self.play_cell()
    }

    fn play_cell(&mut self) -> Flow {
        let Some(pgc) = self.pgc() else { return NO_LINK };
        if self.s.celln == 0 {
            return NO_LINK; // libdvdnav: assert(cellN > 0)
        }
        if self.s.celln > pgc.nr_of_cells {
            if self.s.celln > pgc.nr_of_cells + 1 {
                return NO_LINK;
            }
            return self.play_pgc_post();
        }
        if pgc.cells.is_empty() {
            return self.play_pgc_post();
        }
        let cell = pgc.cells[self.s.celln as usize - 1];
        if cell.block_mode == 1 && cell.block_type == 1 {
            // Angle block: jump to the cell for the current angle.
            let angle = self.s.regs.sprm[3].max(1);
            let target = self.s.celln as u16 + angle - 1;
            let ok = pgc
                .cell(target.min(255) as u8)
                .is_some_and(|c| target <= pgc.nr_of_cells as u16 && c.block_mode != 0 && c.block_type == 1);
            if ok {
                self.s.celln = target as u8;
            }
        }
        if !self.set_pgn() {
            return self.play_pgc_post();
        }
        self.s.cell_restart += 1;
        self.s.blockn = 0;
        Flow::Play(0)
    }

    fn play_cell_post(&mut self) -> Flow {
        let Some(pgc) = self.pgc() else { return Flow::Link(Link::Exit) };
        let Some(cell) = pgc.cell(self.s.celln).copied() else { return self.play_pgc_post() };
        if cell.cell_cmd_nr != 0 && pgc.cell_cmds.len() >= cell.cell_cmd_nr as usize {
            let i = cell.cell_cmd_nr as usize - 1;
            if let Some(link) = self.run(&pgc.cell_cmds[i..=i]) {
                return Flow::Link(link);
            }
        }
        if cell.block_mode == 0 {
            self.s.celln += 1;
        } else {
            // Angle block: skip the other angles.
            self.s.celln += 1;
            while let Some(c) = pgc.cell(self.s.celln) {
                if c.block_mode < 2 {
                    break;
                }
                self.s.celln += 1;
            }
        }
        if !self.set_pgn() {
            return self.play_pgc_post();
        }
        self.play_cell()
    }

    // ---- vm.c: process_command ----

    /// Follow links until playback of a cell starts. Returns `true` if it did
    /// (a "jump"), `false` for no-ops and exits.
    fn process(&mut self, mut flow: Flow) -> bool {
        // Guard against discs whose links loop without ever playing a cell.
        let mut budget = 10_000;
        loop {
            budget -= 1;
            if budget == 0 {
                self.s.stopped = true;
                return false;
            }
            let link = match flow {
                Flow::Play(block) => {
                    self.s.blockn = block;
                    return true;
                }
                Flow::Link(l) => l,
            };
            flow = match link {
                Link::Sub { op, button } => {
                    self.set_button(button);
                    match op {
                        LinkSub::Nop => return false,
                        LinkSub::TopC => self.play_cell(),
                        LinkSub::NextC => {
                            self.s.celln = self.s.celln.wrapping_add(1);
                            self.play_cell()
                        }
                        LinkSub::PrevC => {
                            if self.s.celln <= 1 {
                                return false; // libdvdnav: assert(cellN > 1)
                            }
                            self.s.celln -= 1;
                            self.play_cell()
                        }
                        LinkSub::TopPg => self.play_pg(),
                        LinkSub::NextPg => {
                            self.s.pgn = self.s.pgn.wrapping_add(1);
                            self.play_pg()
                        }
                        LinkSub::PrevPg => {
                            if self.s.pgn <= 1 {
                                return false; // libdvdnav: assert(pgN > 1)
                            }
                            self.s.pgn -= 1;
                            self.play_pg()
                        }
                        LinkSub::TopPgc => self.play_pgc(),
                        LinkSub::NextPgc | LinkSub::PrevPgc | LinkSub::GoUpPgc => {
                            let Some(pgc) = self.pgc() else { return false };
                            let target = match op {
                                LinkSub::NextPgc => pgc.next_pgc_nr,
                                LinkSub::PrevPgc => pgc.prev_pgc_nr,
                                _ => pgc.goup_pgc_nr,
                            };
                            if self.set_pgcn(target) {
                                self.play_pgc()
                            } else {
                                Flow::Link(Link::Exit)
                            }
                        }
                        LinkSub::TailPgc => self.play_pgc_post(),
                        LinkSub::Rsm => {
                            if self.s.rsm.vtsn == 0 {
                                Flow::Link(Link::Exit)
                            } else {
                                self.s.domain = Domain::Title;
                                if !self.open_vts(self.s.rsm.vtsn) {
                                    return false;
                                }
                                self.set_pgcn(self.s.rsm.pgcn);
                                self.s.regs.sprm[4..9].copy_from_slice(&self.s.rsm.regs);
                                self.set_button(button);
                                if self.s.rsm.celln == 0 {
                                    self.s.pgn = 1;
                                    self.play_pg()
                                } else {
                                    self.s.celln = self.s.rsm.celln;
                                    if self.set_pgn() {
                                        Flow::Play(self.s.rsm.blockn)
                                    } else {
                                        Flow::Link(Link::Sub { op: LinkSub::TailPgc, button: 0 })
                                    }
                                }
                            }
                        }
                    }
                }
                Link::Pgcn(pgcn) => {
                    self.set_pgcn(pgcn); // libdvdnav: failure asserts; we replay the current PGC
                    self.play_pgc()
                }
                Link::Pttn { pttn, button } => {
                    self.set_button(button);
                    let (vtsn, vts_ttn) = (self.s.vtsn, self.s.regs.sprm[5] as u8);
                    if self.set_vts_ptt(vtsn, vts_ttn, pttn) {
                        self.play_pg()
                    } else {
                        Flow::Link(Link::Exit)
                    }
                }
                Link::Pgn { pgn, button } => {
                    self.set_button(button);
                    self.s.pgn = pgn;
                    self.play_pg()
                }
                Link::Cn { cn, button } => {
                    self.set_button(button);
                    self.s.celln = cn;
                    self.play_cell()
                }
                Link::Exit => {
                    self.s.stopped = true;
                    return false;
                }
                Link::JumpTt(ttn) => {
                    if self.set_tt(ttn as u16) {
                        self.play_pgc()
                    } else {
                        Flow::Link(Link::Exit)
                    }
                }
                Link::JumpVtsTt(ttn) => {
                    if self.set_vts_ptt(self.s.vtsn, ttn, 1) {
                        self.play_pgc()
                    } else {
                        Flow::Link(Link::Exit)
                    }
                }
                Link::JumpVtsPtt { ttn, pttn } => {
                    if self.set_vts_ptt(self.s.vtsn, ttn, pttn) {
                        self.play_pgc_pg(self.s.pgn)
                    } else {
                        Flow::Link(Link::Exit)
                    }
                }
                Link::JumpSsFp => {
                    self.set_fp_pgc();
                    self.play_pgc()
                }
                Link::JumpSsVmgmMenu(menu) => {
                    if self.disc.vmgi.menu_pgci_ut.is_none() {
                        Flow::Link(Link::Exit)
                    } else {
                        self.s.domain = Domain::Vmgm;
                        self.set_menu(menu);
                        self.play_pgc()
                    }
                }
                Link::JumpSsVtsm { vts, ttn, menu } => {
                    if vts != 0 {
                        if vts != self.s.vtsn && !self.open_vts(vts) {
                            return self.exit();
                        }
                        if self.disc.vts(self.s.vtsn).and_then(|v| v.menu_pgci_ut.as_ref()).is_none() {
                            return self.exit();
                        }
                        self.s.domain = Domain::Vtsm;
                    }
                    self.s.regs.sprm[5] = ttn as u16;
                    self.s.regs.sprm[4] = self.get_tt(self.s.vtsn, ttn);
                    self.set_menu(menu);
                    self.play_pgc()
                }
                Link::JumpSsVmgmPgc(pgcn) => {
                    if self.disc.vmgi.menu_pgci_ut.is_none() {
                        Flow::Link(Link::Exit)
                    } else {
                        self.s.domain = Domain::Vmgm;
                        self.set_pgcn(pgcn);
                        self.play_pgc()
                    }
                }
                Link::CallSsFp { rsm_cell } => {
                    self.set_rsm_info(rsm_cell, 0);
                    self.set_fp_pgc();
                    self.play_pgc()
                }
                Link::CallSsVmgmMenu { menu, rsm_cell } => {
                    if self.disc.vmgi.menu_pgci_ut.is_none() {
                        Flow::Link(Link::Exit)
                    } else {
                        self.set_rsm_info(rsm_cell, 0);
                        self.s.domain = Domain::Vmgm;
                        self.set_menu(menu);
                        self.play_pgc()
                    }
                }
                Link::CallSsVtsm { menu, rsm_cell } => {
                    if self.disc.vts(self.s.vtsn).and_then(|v| v.menu_pgci_ut.as_ref()).is_none() {
                        Flow::Link(Link::Exit)
                    } else {
                        self.set_rsm_info(rsm_cell, 0);
                        self.s.domain = Domain::Vtsm;
                        self.set_menu(menu);
                        self.play_pgc()
                    }
                }
                Link::CallSsVmgmPgc { pgcn, rsm_cell } => {
                    if self.disc.vmgi.menu_pgci_ut.is_none() {
                        Flow::Link(Link::Exit)
                    } else {
                        self.set_rsm_info(rsm_cell, 0);
                        self.s.domain = Domain::Vmgm;
                        self.set_pgcn(pgcn);
                        self.play_pgc()
                    }
                }
            };
        }
    }

    fn exit(&mut self) -> bool {
        self.process(Flow::Link(Link::Exit))
    }

    // ---- vm.c: entry points ----

    fn start(&mut self) -> bool {
        self.set_fp_pgc();
        let flow = self.play_pgc();
        self.process(flow);
        !self.s.stopped
    }

    fn get_next_cell(&mut self) {
        let flow = self.play_cell_post();
        self.process(flow);
    }

    fn jump_menu(&mut self, menu: MenuCall) -> bool {
        let old_domain = self.s.domain;
        match self.s.domain {
            Domain::FirstPlay => return false,
            Domain::Title => {
                let blockn = self.s.blockn;
                self.set_rsm_info(0, blockn);
            }
            _ => {}
        }
        match menu {
            MenuCall::Title | MenuCall::Escape => {
                if self.disc.vmgi.menu_pgci_ut.is_none() {
                    return false;
                }
                self.s.domain = Domain::Vmgm;
            }
            _ => {
                if self.disc.vts(self.s.vtsn).and_then(|v| v.menu_pgci_ut.as_ref()).is_none() {
                    return false;
                }
                self.s.domain = Domain::Vtsm;
            }
        }
        if self.domain_space().is_some() && self.set_menu(menu.id()) {
            let flow = self.play_pgc();
            self.process(flow);
            true
        } else {
            self.s.domain = old_domain;
            false
        }
    }

    fn jump_resume(&mut self) -> bool {
        if self.s.rsm.vtsn == 0 {
            return false;
        }
        self.process(Flow::Link(Link::Sub { op: LinkSub::Rsm, button: 0 }))
    }

    /// Run a single command (a button command). Returns `true` on a jump.
    fn exec_cmd(&mut self, cmd: menuseum_core::ifo::RawCmd) -> bool {
        match self.run(&[cmd]) {
            Some(link) => self.process(Flow::Link(link)),
            None => false,
        }
    }
}

impl Engine {
    pub fn new(disc: Disc, quirks: Quirks, seed: u64) -> Self {
        Engine {
            disc,
            vm: VmState::new(seed),
            quirks,
            now_ms: 0,
            started: false,
            file_open: false,
            cur: Position::default(),
            vobu: Vobu::default(),
            pci: Pci::default(),
            dsi: Dsi::default(),
            last_cmd_nav_lbn: SRI_END_OF_CELL,
            sync_wait: false,
            sync_wait_skip: false,
            skip_still: false,
            spu_clut_changed: false,
        }
    }

    pub fn disc(&self) -> &Disc {
        &self.disc
    }

    pub fn pci(&self) -> &Pci {
        &self.pci
    }

    pub fn dsi(&self) -> &Dsi {
        &self.dsi
    }

    fn nav(&mut self) -> Nav<'_> {
        Nav { disc: &self.disc, s: &mut self.vm, quirks: self.quirks, now_ms: self.now_ms }
    }

    /// The PGC currently playing.
    pub fn pgc(&self) -> Option<&Pgc> {
        // Nav needs &mut state; build a read-only lookup by hand.
        let r = self.vm.pgc?;
        match r.space {
            PgcSpace::FirstPlay => self.disc.vmgi.first_play_pgc.as_ref(),
            space => Nav::pgcit(&self.disc, space)?.pgc(r.pgcn),
        }
    }

    /// Current PGC number in the current domain (0 for the first-play PGC).
    pub fn pgcn(&mut self) -> u16 {
        self.nav().pgcn()
    }

    /// The VOB set the current domain plays from.
    pub fn vobs(&self) -> Vobs {
        match self.vm.domain {
            Domain::FirstPlay | Domain::Vmgm => Vobs::Vmgm,
            Domain::Vtsm => Vobs::Menu(self.vm.vtsn),
            Domain::Title => Vobs::Title(self.vm.vtsn),
        }
    }

    // ---- vmget.c: stream selection ----

    /// Video attributes of the current domain.
    pub fn video_attr(&self) -> VideoAttr {
        let vts = self.disc.vts(self.vm.vtsn);
        match self.vm.domain {
            Domain::Title => vts.map(|v| v.video_attr).unwrap_or_default(),
            Domain::Vtsm => vts.map(|v| v.menu_video_attr).unwrap_or_default(),
            Domain::Vmgm | Domain::FirstPlay => self.disc.vmgi.menu_video_attr,
        }
    }

    /// Display aspect of the current domain (0 = 4:3, 3 = 16:9). Like libdvdnav,
    /// this records the aspect in SPRM 14 bits 10..11.
    pub fn video_aspect(&mut self) -> Option<u8> {
        let aspect = self.video_attr().display_aspect_ratio;
        if aspect != 0 && aspect != 3 {
            return None;
        }
        self.vm.regs.sprm[14] = (self.vm.regs.sprm[14] & !(3 << 10)) | (aspect as u16) << 10;
        Some(aspect)
    }

    /// Physical audio stream for logical stream `audio_n`, if present.
    pub fn audio_stream(&self, audio_n: u16) -> Option<u8> {
        let pgc = self.pgc()?;
        let n = if self.vm.domain == Domain::Title { audio_n as usize } else { 0 };
        let ctl = pgc.audio_control.get(n).copied().unwrap_or(0);
        if ctl & 0x8000 != 0 {
            Some(((ctl >> 8) & 7) as u8)
        } else if self.vm.domain != Domain::Title {
            Some(0)
        } else {
            None
        }
    }

    /// Physical audio stream to play: SPRM 1's stream or the first one present.
    pub fn audio_active_stream(&self) -> Option<u8> {
        self.audio_stream(self.vm.regs.sprm[1]).or_else(|| {
            let pgc = self.pgc()?;
            (0..8u16).filter(|&n| pgc.audio_control[n as usize] & 0x8000 != 0).find_map(|n| self.audio_stream(n))
        })
    }

    /// Physical subpicture stream for logical stream `subp_n` in display `mode`
    /// (0 widescreen, 1 letterbox, 2 pan&scan).
    pub fn subp_stream(&mut self, subp_n: u16, mode: u8) -> Option<u8> {
        let aspect = self.video_aspect();
        let pgc = self.pgc()?;
        let n = if self.vm.domain == Domain::Title { subp_n as usize } else { 0 };
        let mut stream = None;
        if let Some(&ctl) = pgc.subp_control.get(n) {
            if ctl & 0x8000_0000 != 0 {
                stream = match (aspect, mode) {
                    (Some(0), _) => Some((ctl >> 24) & 0x1f),
                    (Some(3), 0) => Some((ctl >> 16) & 0x1f),
                    (Some(3), 1) => Some((ctl >> 8) & 0x1f),
                    (Some(3), 2) => Some(ctl & 0x1f),
                    _ => None,
                };
            }
        }
        match stream {
            Some(s) => Some(s as u8),
            None if self.vm.domain != Domain::Title => Some(0),
            None => None,
        }
    }

    /// Subpicture stream to decode for `mode`; bit 7 set means only forced
    /// subpictures should be shown (subtitles switched off).
    pub fn subp_active_stream(&mut self, mode: u8) -> Option<u8> {
        let spst = self.vm.regs.sprm[2];
        let mut stream = self.subp_stream(spst & !0x40, mode);
        if stream.is_none() {
            let present: Vec<u16> = match self.pgc() {
                Some(pgc) => (0..32u16).filter(|&n| pgc.subp_control[n as usize] & 0x8000_0000 != 0).collect(),
                None => vec![],
            };
            stream = present.into_iter().find_map(|n| self.subp_stream(n, mode));
        }
        if self.vm.domain == Domain::Title && spst & 0x40 == 0 {
            stream.map(|s| s | 0x80)
        } else {
            stream
        }
    }

    /// `vm_position_get`.
    fn position(&self) -> Option<Position> {
        let pgc = self.pgc()?;
        let cell = pgc.cell(self.vm.celln)?;
        let mut still = cell.still_time as u32;
        if self.vm.celln == pgc.nr_of_cells {
            still += pgc.still_time as u32;
        }
        if still == 0 {
            // libdvdnav heuristic for discs that encode stills only as an
            // implausibly long playback time on a tiny cell.
            let size = cell.last_sector.wrapping_sub(cell.first_sector);
            if cell.last_sector == cell.last_vobu_start_sector && size < 1024 {
                let time = cell.playback_time.whole_seconds();
                if time != 0 && size / time <= 30 {
                    still = time.min(0xff);
                }
            }
        }
        Some(Position {
            button: self.vm.regs.button() as i32,
            vts: self.vm.vtsn as i32,
            domain: self.vm.domain as i32,
            spu_channel: self.vm.regs.sprm[2] as i32,
            audio_channel: self.vm.regs.sprm[1] as i32,
            hop_channel: self.vm.hop_channel,
            cell: self.vm.celln as i32,
            cell_restart: self.vm.cell_restart,
            cell_start: cell.first_sector,
            still,
            block: self.vm.blockn,
        })
    }

    fn angle_info(&self) -> (u16, u8) {
        if self.vm.domain == Domain::Title {
            let ttn = self.vm.regs.sprm[4];
            if let Some(t) = self.disc.vmgi.title(ttn) {
                if t.title_set_nr == self.vm.vtsn && t.vts_ttn as u16 == self.vm.regs.sprm[5] {
                    return (self.vm.regs.sprm[3], t.nr_of_angles);
                }
            }
        }
        (1, 1)
    }

    /// `dvdnav_get_vobu`.
    fn update_vobu(&mut self) {
        self.vobu.start = self.dsi.nv_pck_lbn;
        self.vobu.length = self.dsi.vobu_ea;
        self.vobu.next = self.dsi.next_vobu & 0x3fff_ffff;
        let (angle, num_angles) = self.angle_info();
        if num_angles != 0 && self.dsi.sml_category & 0xf000 == 0x5000 {
            let a = (angle as usize).clamp(1, 9) - 1;
            let next = self.pci.nsml_agl_dsta[a];
            if next != 0 {
                if next & 0x3fff_ffff != 0 {
                    let off = next & 0x3fff_ffff;
                    self.vobu.next = if next & 0x8000_0000 != 0 { off.wrapping_neg() } else { off };
                }
            } else {
                let next = self.dsi.sml_agl_address[a];
                if next != 0 {
                    self.vobu.length = self.dsi.ilvu_ea;
                    let off = next & 0x3fff_ffff;
                    self.vobu.next =
                        if next & 0x8000_0000 != 0 && next != 0x7fff_ffff { off.wrapping_neg() } else { off };
                }
            }
        }
    }

    /// Advance playback by one step (`dvdnav_get_next_block`, minus the blocks).
    pub fn next_event(&mut self, src: &mut dyn NavSource) -> Event {
        if !self.started {
            if self.vm.stopped {
                // libdvdnav restarts the VM from scratch after a stop.
                let seed = self.vm.rng.clone();
                self.vm = VmState::new(0);
                self.vm.rng = seed;
            }
            if !self.nav().start() {
                return Event::Error("faulty disc: first play stopped".into());
            }
            self.started = true;
        }
        if self.vm.pgc.is_none() {
            return Event::Error("no current PGC".into());
        }
        if self.vm.stopped {
            self.started = false;
            return Event::Stop;
        }
        let Some(next) = self.position() else {
            return Event::Error(format!("invalid cell {} in current PGC", self.vm.celln));
        };

        if self.cur.hop_channel != next.hop_channel {
            self.cur.hop_channel = next.hop_channel;
            self.vobu = Vobu { start: next.cell_start.wrapping_add(next.block), ..Vobu::default() };
            self.sync_wait = false;
            return Event::HopChannel;
        }
        if self.cur.button != next.button {
            self.cur.button = next.button;
            return Event::Highlight { button: next.button as u8 };
        }
        if self.sync_wait {
            return Event::Wait;
        }
        if !self.file_open || self.cur.vts != next.vts || self.cur.domain != next.domain {
            let old_domain = [Domain::FirstPlay, Domain::Title, Domain::Vmgm, Domain::Vtsm]
                .into_iter()
                .find(|d| *d as i32 == self.cur.domain);
            let old_vts = self.cur.vts as u8;
            self.cur.vts = next.vts;
            self.cur.domain = next.domain;
            self.file_open = true;
            self.spu_clut_changed = true;
            self.cur.cell = -1;
            self.cur.spu_channel = -1;
            self.cur.audio_channel = -1;
            return Event::VtsChange { old_domain, old_vts, new_domain: self.vm.domain, new_vts: self.vm.vtsn };
        }
        if self.cur.cell != next.cell
            || self.cur.cell_restart != next.cell_restart
            || self.cur.cell_start != next.cell_start
        {
            self.cur.cell = next.cell;
            self.cur.cell_restart = next.cell_restart;
            self.cur.cell_start = next.cell_start;
            self.cur.block = next.block;
            self.vobu = Vobu { start: next.cell_start.wrapping_add(next.block), ..Vobu::default() };
            self.spu_clut_changed = true;
            self.cur.spu_channel = -1;
            self.cur.audio_channel = -1;
            return Event::CellChange { celln: self.vm.celln, pgn: self.vm.pgn };
        }
        if self.spu_clut_changed {
            self.spu_clut_changed = false;
            return Event::SpuClutChange;
        }
        if self.cur.spu_channel != next.spu_channel {
            self.cur.spu_channel = next.spu_channel;
            // libdvdnav applies the forced-select button here rather than at HLI_S_PTM.
            if self.pci.hli.fosl_btnn != 0 {
                self.vm.regs.set_button(self.pci.hli.fosl_btnn);
            }
            // libdvdnav resolves the physical streams here, updating SPRM 14 on the way.
            for mode in 0..3 {
                self.subp_active_stream(mode);
            }
            return Event::SpuStreamChange;
        }
        if self.cur.audio_channel != next.audio_channel {
            self.cur.audio_channel = next.audio_channel;
            return Event::AudioStreamChange;
        }
        if self.cur.still != 0 {
            return Event::Still(self.cur.still.min(0xff) as u8);
        }
        if self.vobu.blockn >= self.vobu.length {
            if self.vobu.next == SRI_END_OF_CELL {
                self.cur.still = next.still;
                if (self.cur.still != 0 || self.pci.hli.hli_ss != 0) && !self.sync_wait_skip {
                    self.sync_wait = true;
                }
                if self.cur.still == 0 || self.skip_still {
                    self.nav().get_next_cell();
                    self.cur.still = 0;
                    self.skip_still = false;
                    self.sync_wait_skip = false;
                }
                return Event::Nop;
            }
            let lba = self.vobu.start.wrapping_add(self.vobu.next);
            let pack = match src.nav_pack(self.vobs(), lba) {
                Ok(NavRead::Pack(p)) => p,
                Ok(NavRead::Pending) => return Event::Pending { vobs: self.vobs(), lba },
                Ok(NavRead::Stripped) => {
                    // Stripped data: skip straight to the end of the cell.
                    self.pci = Pci::default();
                    self.vobu = Vobu { start: self.vobu.start, next: SRI_END_OF_CELL, length: 0, blockn: 0 };
                    return Event::Nop;
                }
                Err(e) => return Event::Error(format!("reading NAV pack at {lba}: {e}")),
            };
            self.pci = pack.pci;
            self.dsi = pack.dsi;
            self.vm.blockn = self.vobu.start.wrapping_sub(self.cur.cell_start);
            self.update_vobu();
            // We never hand out the VOBU's data blocks, so consume them at once.
            self.vobu.blockn = self.vobu.length;
            if self.last_cmd_nav_lbn == self.pci.nv_pck_lbn {
                self.last_cmd_nav_lbn = SRI_END_OF_CELL;
            }
            return Event::Nav { lbn: self.pci.nv_pck_lbn };
        }
        // Unreachable in practice: blocks are consumed with their NAV pack.
        self.vobu.blockn = self.vobu.length;
        Event::Nop
    }

    pub fn still_skip(&mut self) {
        self.cur.still = 0;
        self.skip_still = true;
        self.sync_wait = false;
        self.sync_wait_skip = true;
    }

    pub fn wait_skip(&mut self) {
        self.sync_wait = false;
        self.sync_wait_skip = true;
    }

    // ---- highlight.c: buttons ----

    fn buttons_live(&self) -> bool {
        self.pci.hli.hli_ss != 0 && self.last_cmd_nav_lbn != self.pci.nv_pck_lbn
    }

    fn current_button(&self) -> Option<menuseum_core::nav::Button> {
        if !self.buttons_live() {
            return None;
        }
        let n = self.vm.regs.button() as usize;
        self.pci.hli.buttons.get(n.checked_sub(1)?).copied()
    }

    /// Highlight button `n` (1-based). Returns false if no such button is live.
    pub fn button_select(&mut self, n: u8) -> bool {
        if !self.buttons_live() || n == 0 || n > self.pci.hli.btn_ns {
            return false;
        }
        self.vm.regs.set_button(n);
        self.cur.button = -1;
        true
    }

    /// Activate the highlighted button.
    pub fn button_activate(&mut self) -> bool {
        if self.pci.hli.hli_ss == 0 || self.last_cmd_nav_lbn == self.pci.nv_pck_lbn {
            return false;
        }
        if self.vm.pgc.is_none() {
            return false;
        }
        let n = self.vm.regs.button();
        if n == 0 || n > self.pci.hli.btn_ns {
            if self.cur.still != 0 {
                self.nav().get_next_cell();
                self.cur.still = 0;
                self.sync_wait = false;
                self.last_cmd_nav_lbn = self.pci.nv_pck_lbn;
                return true;
            }
            return false;
        }
        let Some(button) = self.current_button() else { return false };
        if self.nav().exec_cmd(button.cmd) {
            self.vm.hop_channel += 1;
            self.cur.still = 0;
            self.last_cmd_nav_lbn = self.pci.nv_pck_lbn;
        }
        true
    }

    fn button_move(&mut self, pick: impl Fn(&menuseum_core::nav::Button) -> u8) -> bool {
        let Some(b) = self.current_button() else { return false };
        self.button_select(pick(&b));
        // Auto-action buttons activate as soon as they are selected.
        match self.current_button() {
            Some(b) if b.auto_action != 0 => self.button_activate(),
            Some(_) => true,
            None => false,
        }
    }

    pub fn button_up(&mut self) -> bool {
        self.button_move(|b| b.up)
    }

    pub fn button_down(&mut self) -> bool {
        self.button_move(|b| b.down)
    }

    pub fn button_left(&mut self) -> bool {
        self.button_move(|b| b.left)
    }

    pub fn button_right(&mut self) -> bool {
        self.button_move(|b| b.right)
    }

    /// Select the button under (x, y) nearest its centre (`dvdnav_mouse_select`).
    pub fn mouse_select(&mut self, x: u16, y: u16) -> bool {
        if !self.buttons_live() {
            return false;
        }
        let cur = self.vm.regs.button();
        let mut best = 0u8;
        let mut dist = i32::MAX;
        for n in 1..=self.pci.hli.btn_ns {
            let b = &self.pci.hli.buttons[n as usize - 1];
            if (b.x_start..=b.x_end).contains(&x) && (b.y_start..=b.y_end).contains(&y) {
                let dx = (b.x_start as i32 + b.x_end as i32) / 2 - x as i32;
                let dy = (b.y_start as i32 + b.y_end as i32) / 2 - y as i32;
                let d = dx * dx + dy * dy;
                if d < dist {
                    dist = d;
                    best = n;
                }
            }
        }
        if best != 0 && best != cur {
            self.button_select(best);
        }
        best != 0
    }

    // ---- searching.c: menu calls ----

    /// Jump to a menu on user request (`dvdnav_menu_call`). The jump is tried on
    /// a copy of the VM and only kept if it succeeds.
    pub fn menu_call(&mut self, menu: MenuCall) -> bool {
        if self.vm.pgc.is_none() {
            return false;
        }
        let backup = self.vm.clone();
        if menu == MenuCall::Escape && self.vm.domain != Domain::Title {
            if self.nav().jump_resume() && !self.vm.stopped {
                self.cur.still = 0;
                self.vm.hop_channel += 1;
                return true;
            }
            self.vm = backup.clone();
        }
        let menu = if menu == MenuCall::Escape { MenuCall::Root } else { menu };
        if self.nav().jump_menu(menu) && !self.vm.stopped {
            self.cur.still = 0;
            self.vm.hop_channel += 1;
            true
        } else {
            self.vm = backup;
            false
        }
    }
}

impl Navigator for Engine {
    fn disc(&self) -> &Disc {
        Engine::disc(self)
    }

    fn status(&self) -> Status {
        let vm = &self.vm;
        Status {
            domain: vm.domain,
            vtsn: vm.vtsn,
            pgn: vm.pgn,
            celln: vm.celln,
            cell_restart: vm.cell_restart,
            hop_channel: vm.hop_channel,
            stopped: vm.stopped,
        }
    }

    fn regs(&self) -> &Registers {
        &self.vm.regs
    }

    fn set_now_ms(&mut self, now_ms: u64) {
        self.now_ms = now_ms;
    }

    fn next_event(&mut self, src: &mut dyn NavSource) -> Event {
        Engine::next_event(self, src)
    }

    fn pci(&self) -> &Pci {
        Engine::pci(self)
    }

    fn dsi(&self) -> &Dsi {
        Engine::dsi(self)
    }

    fn pgc(&self) -> Option<&Pgc> {
        Engine::pgc(self)
    }

    fn pgcn(&mut self) -> u16 {
        Engine::pgcn(self)
    }

    fn vobs(&self) -> Vobs {
        Engine::vobs(self)
    }

    fn video_attr(&self) -> VideoAttr {
        Engine::video_attr(self)
    }

    fn audio_active_stream(&self) -> Option<u8> {
        Engine::audio_active_stream(self)
    }

    fn subp_active_stream(&mut self, mode: u8) -> Option<u8> {
        Engine::subp_active_stream(self, mode)
    }

    fn still_skip(&mut self) {
        Engine::still_skip(self)
    }

    fn wait_skip(&mut self) {
        Engine::wait_skip(self)
    }

    fn button_select(&mut self, n: u8) -> bool {
        Engine::button_select(self, n)
    }

    fn button_activate(&mut self) -> bool {
        Engine::button_activate(self)
    }

    fn button_up(&mut self) -> bool {
        Engine::button_up(self)
    }

    fn button_down(&mut self) -> bool {
        Engine::button_down(self)
    }

    fn button_left(&mut self) -> bool {
        Engine::button_left(self)
    }

    fn button_right(&mut self) -> bool {
        Engine::button_right(self)
    }

    fn mouse_select(&mut self, x: u16, y: u16) -> bool {
        Engine::mouse_select(self, x, y)
    }

    fn menu_call(&mut self, menu: MenuCall) -> bool {
        Engine::menu_call(self, menu)
    }
}
