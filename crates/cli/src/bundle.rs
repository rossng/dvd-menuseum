//! `menuseum bundle`: strip a disc to its menus.
//!
//! A bundle is the disc's `VIDEO_TS` without the film: every IFO and every
//! menu VOB set, byte for byte, plus a manifest for the web player.
//!
//! ```text
//! <out>/VIDEO_TS/VIDEO_TS.IFO, VIDEO_TS.VOB, VTS_nn_0.IFO, VTS_nn_0.VOB
//! <out>/bundle.json
//! ```
//!
//! The title VOB sets (`VTS_nn_1..9.VOB`) are left out; players treat their
//! cells as stripped. A bundle is itself a valid input to every `menuseum`
//! command.

use std::fmt::Write as _;
use std::path::Path;

use menuseum_core::navigator::{Disc, Vobs};
use menuseum_core::ifo::SECTOR;

use crate::source::{vobs_id, DvdFiles};

fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

pub fn bundle(files: &mut DvdFiles, disc: &Disc, out: &Path) -> Result<(), String> {
    let dir = out.join("VIDEO_TS");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut written: Vec<(String, u64)> = vec![];
    let mut copy = |files: &mut DvdFiles, name: &str, fallback: Option<&str>| -> Result<bool, String> {
        let data = match files.read(name) {
            Ok(d) => d,
            Err(_) => match fallback.map(|f| files.read(f)) {
                Some(Ok(d)) => d,
                _ => return Ok(false),
            },
        };
        let path = dir.join(name);
        std::fs::write(&path, &data).map_err(|e| format!("{}: {e}", path.display()))?;
        written.push((name.to_string(), data.len() as u64));
        Ok(true)
    };

    if !copy(files, "VIDEO_TS.IFO", Some("VIDEO_TS.BUP"))? {
        return Err("no VIDEO_TS.IFO".into());
    }
    let mut vobs = vec![];
    if copy(files, "VIDEO_TS.VOB", None)? {
        vobs.push(Vobs::Vmgm);
    }
    for n in 1..=disc.vmgi.nr_of_title_sets as u8 {
        let stem = format!("VTS_{n:02}_0");
        if !copy(files, &format!("{stem}.IFO"), Some(&format!("{stem}.BUP")))? {
            eprintln!("warning: {stem}.IFO missing");
        }
        if copy(files, &format!("{stem}.VOB"), None)? {
            vobs.push(Vobs::Menu(n));
        }
    }

    let mut j = String::from("{\n  \"version\": 1,\n");
    let _ = writeln!(j, "  \"provider\": {},", json_str(&disc.vmgi.provider_identifier));
    j.push_str("  \"files\": [\n");
    for (i, (name, size)) in written.iter().enumerate() {
        let comma = if i + 1 < written.len() { "," } else { "" };
        let _ = writeln!(j, "    {{\"name\": {}, \"size\": {size}}}{comma}", json_str(&format!("VIDEO_TS/{name}")));
    }
    j.push_str("  ],\n  \"vobs\": [\n");
    for (i, v) in vobs.iter().enumerate() {
        let name = match v {
            Vobs::Vmgm => "VIDEO_TS.VOB".to_string(),
            Vobs::Menu(n) => format!("VTS_{n:02}_0.VOB"),
            Vobs::Title(_) => unreachable!(),
        };
        let size = written.iter().find(|(f, _)| *f == name).map_or(0, |(_, s)| *s);
        let comma = if i + 1 < vobs.len() { "," } else { "" };
        let _ = writeln!(
            j,
            "    {{\"id\": {}, \"file\": {}, \"sectors\": {}}}{comma}",
            json_str(&vobs_id(*v)),
            json_str(&format!("VIDEO_TS/{name}")),
            size / SECTOR as u64
        );
    }
    j.push_str("  ]\n}\n");
    let path = out.join("bundle.json");
    std::fs::write(&path, j).map_err(|e| format!("{}: {e}", path.display()))?;
    let total: u64 = written.iter().map(|(_, s)| s).sum();
    eprintln!("{} files, {:.1} MB", written.len(), total as f64 / 1e6);
    Ok(())
}
