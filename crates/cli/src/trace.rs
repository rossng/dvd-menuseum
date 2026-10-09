//! Headless playback with scripted input, producing a line-based event trace.
//!
//! The format and the input-injection policy are shared with the libdvdnav
//! oracle (`oracle/oracle.c`) so the two traces can be diffed directly.
//!
//! Policy:
//! - `Wait` is skipped immediately.
//! - Finite stills are skipped immediately (the clock advances by their length).
//! - An *input point* is a NAV pack whose highlight information has live
//!   buttons, or an infinite still. At each input point the next scripted input
//!   is applied; if the script is exhausted the trace ends.
//! - The clock advances by each VOBU's presentation duration.

use std::fmt::Write as _;

use menuseum_core::navigator::{Event, MenuCall, NavSource};
use menuseum_engine::Engine;
use menuseum_core::nav::SRI_END_OF_CELL;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    Up,
    Down,
    Left,
    Right,
    Activate,
    Select(u8),
    Menu(MenuCall),
}

pub fn parse_script(text: &str) -> Result<Vec<Input>, String> {
    let mut out = vec![];
    for tok in text.lines().map(|l| l.split('#').next().unwrap_or("")).flat_map(str::split_whitespace) {
        let (word, arg) = tok.split_once(':').unwrap_or((tok, ""));
        out.push(match word {
            "up" => Input::Up,
            "down" => Input::Down,
            "left" => Input::Left,
            "right" => Input::Right,
            "activate" | "ok" | "enter" => Input::Activate,
            "select" => Input::Select(arg.parse().map_err(|_| format!("bad button in {tok}"))?),
            "menu" => Input::Menu(match arg {
                "escape" => MenuCall::Escape,
                "title" => MenuCall::Title,
                "root" => MenuCall::Root,
                "subpicture" => MenuCall::Subpicture,
                "audio" => MenuCall::Audio,
                "angle" => MenuCall::Angle,
                "part" => MenuCall::Part,
                _ => return Err(format!("unknown menu in {tok}")),
            }),
            _ => return Err(format!("unknown input {tok}")),
        });
    }
    Ok(out)
}

pub fn input_name(i: &Input) -> String {
    match i {
        Input::Up => "up".into(),
        Input::Down => "down".into(),
        Input::Left => "left".into(),
        Input::Right => "right".into(),
        Input::Activate => "activate".into(),
        Input::Select(n) => format!("select:{n}"),
        Input::Menu(m) => format!("menu:{}", format!("{m:?}").to_lowercase()),
    }
}

fn apply(engine: &mut Engine, input: &Input) -> bool {
    match input {
        Input::Up => engine.button_up(),
        Input::Down => engine.button_down(),
        Input::Left => engine.button_left(),
        Input::Right => engine.button_right(),
        Input::Activate => engine.button_activate(),
        Input::Select(n) => engine.button_select(*n),
        Input::Menu(m) => engine.menu_call(*m),
    }
}

fn state(engine: &mut Engine, full: bool) -> String {
    let pgcn = engine.pgcn();
    let vm = &engine.vm;
    let mut s = format!(
        "d={} vts={} pgc={} pg={} cell={} btn={}",
        vm.domain as u8,
        vm.vtsn,
        pgcn,
        vm.pgn,
        vm.celln,
        vm.regs.button()
    );
    if full {
        s.push_str(" S=");
        for (i, r) in vm.regs.sprm.iter().enumerate() {
            let _ = write!(s, "{}{r:04x}", if i == 0 { "" } else { "," });
        }
        s.push_str(" G=");
        for (i, r) in vm.regs.gprm.iter().enumerate() {
            let _ = write!(s, "{}{r:04x}", if i == 0 { "" } else { "," });
        }
    }
    s
}

pub struct Options {
    pub max_events: usize,
    /// Treat a NAV pack as an input point only after this many NAV packs with
    /// live buttons in the same cell, or at the cell's last VOBU. Lets motion
    /// menus finish their intro before rendering. 0 = libdvdnav-comparable policy.
    pub settle: usize,
}

/// Run until the script is exhausted at an input point, the disc stops, or
/// `max_events` events have been produced. Returns the trace lines.
///
/// `at_input` is called at every input point, before the input (if any) is applied.
pub fn run<S: NavSource>(
    engine: &mut Engine,
    src: &mut S,
    script: &[Input],
    opts: &Options,
    at_input: &mut dyn FnMut(&mut Engine, &mut S, Option<&Input>),
) -> Vec<String> {
    let mut out = vec![];
    let mut inputs = script.iter();
    let mut events = 0;
    let mut live_navs = 0;
    loop {
        if events >= opts.max_events {
            out.push("END max-events".into());
            break;
        }
        events += 1;
        let ev = engine.next_event(src);
        let line = match &ev {
            Event::Nop => continue,
            Event::Stop => {
                out.push(format!("STOP {}", state(engine, true)));
                break;
            }
            Event::Error(e) => {
                out.push(format!("ERROR {e}"));
                break;
            }
            Event::Pending { lba, .. } => {
                // Disc files are always available; a pending read is a bug.
                out.push(format!("ERROR sector {lba} unexpectedly pending"));
                break;
            }
            Event::Wait => {
                engine.wait_skip();
                continue;
            }
            Event::HopChannel => {
                live_navs = 0;
                format!("HOP {}", state(engine, true))
            }
            Event::Highlight { button } => format!("HIGHLIGHT {button} {}", state(engine, true)),
            Event::VtsChange { old_domain, old_vts, new_domain, new_vts } => format!(
                "VTS {}:{}->{}:{} {}",
                old_domain.map_or(0, |d| d as u8),
                old_vts,
                *new_domain as u8,
                new_vts,
                state(engine, true)
            ),
            Event::CellChange { celln, pgn } => {
                live_navs = 0;
                format!("CELL {celln} pg={pgn} {}", state(engine, true))
            }
            Event::SpuClutChange => format!("CLUT {}", state(engine, false)),
            Event::SpuStreamChange => format!("SPU {}", state(engine, false)),
            Event::AudioStreamChange => format!("AUDIO {}", state(engine, false)),
            Event::Still(secs) => format!("STILL {secs} {}", state(engine, true)),
            Event::Nav { lbn } => {
                let (hli_ss, btn_ns) = (engine.pci().hli.hli_ss, engine.pci().hli.btn_ns);
                format!("NAV {lbn} hli={hli_ss} btns={btn_ns} {}", state(engine, false))
            }
        };
        out.push(line);

        match ev {
            Event::Still(secs) => {
                let input_point = secs == 0xff;
                if !input_point {
                    engine.now_ms += secs as u64 * 1000;
                    engine.still_skip();
                    continue;
                }
                let next = inputs.next();
                at_input(engine, src, next);
                match next {
                    Some(i) => {
                        let ok = apply(engine, i);
                        out.push(format!("INPUT {} {}", input_name(i), if ok { "ok" } else { "ignored" }));
                    }
                    None => {
                        out.push("END waiting-for-input".into());
                        break;
                    }
                }
            }
            Event::Nav { .. } => {
                let pci = engine.pci();
                let ptm = pci.vobu_e_ptm.wrapping_sub(pci.vobu_s_ptm);
                let live = pci.hli.hli_ss != 0 && pci.hli.btn_ns > 0;
                let last_vobu = engine.dsi().next_vobu & 0x3fff_ffff == SRI_END_OF_CELL;
                engine.now_ms += ptm as u64 / 90;
                if live {
                    live_navs += 1;
                }
                if live && (live_navs > opts.settle || last_vobu) {
                    let next = inputs.next();
                    at_input(engine, src, next);
                    match next {
                        Some(i) => {
                            let ok = apply(engine, i);
                            out.push(format!("INPUT {} {}", input_name(i), if ok { "ok" } else { "ignored" }));
                        }
                        None => {
                            out.push("END waiting-for-input".into());
                            break;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}
