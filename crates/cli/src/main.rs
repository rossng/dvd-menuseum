mod bundle;
mod png;
mod render;
mod source;
mod trace;

use std::path::PathBuf;
use std::process::ExitCode;

use menuseum_core::cmd::{decode, Quirks};
use menuseum_core::navigator::Disc;
use menuseum_engine::Engine;
use menuseum_core::ifo::{Pgc, Pgcit, PgciUt};

use source::DvdFiles;

const USAGE: &str = "usage: menuseum <command> <disc> [options]

commands:
  info <disc>                    summarise titles, title sets and menus
  disasm <disc>                  disassemble every PGC's commands
  trace <disc> [--inputs FILE|-i 'down activate ...'] [--max-events N] [--compat] [--seed N] [--settle N]
                                 play headlessly with scripted input
  extract <disc> <vobs> <first> <last> --out FILE
                                 copy a sector range of a VOB set (vmgm, menu-N, title-N)
  bundle <disc> --out DIR       strip the disc to a menu bundle: its IFOs and menu VOBs
  aspect <disc>                  the display aspect of the first picture: 16:9 or 4:3
  render <disc> --out DIR [trace options]
                                 like trace, also writing a PNG at every input point

<disc> is a disc image (ISO 9660) or a VIDEO_TS directory.";

fn lang(code: u16) -> String {
    let [a, b] = code.to_be_bytes();
    if a.is_ascii_alphanumeric() && b.is_ascii_alphanumeric() {
        format!("{}{}", a as char, b as char)
    } else {
        format!("{code:#06x}")
    }
}

fn menu_name(entry_id: u8) -> &'static str {
    if entry_id & 0x80 == 0 {
        return "";
    }
    match entry_id & 0x0f {
        2 => " [title menu]",
        3 => " [root menu]",
        4 => " [subpicture menu]",
        5 => " [audio menu]",
        6 => " [angle menu]",
        7 => " [chapter menu]",
        _ => " [entry]",
    }
}

fn print_pgc_summary(indent: &str, label: &str, pgc: &Pgc) {
    let stills = pgc.cells.iter().filter(|c| c.still_time != 0).count();
    println!(
        "{indent}{label}: {} programs, {} cells{}, {}+{}+{} cmds, next/prev/up {}/{}/{}",
        pgc.nr_of_programs,
        pgc.nr_of_cells,
        if stills > 0 { format!(" ({stills} still)") } else { String::new() },
        pgc.pre_cmds.len(),
        pgc.post_cmds.len(),
        pgc.cell_cmds.len(),
        pgc.next_pgc_nr,
        pgc.prev_pgc_nr,
        pgc.goup_pgc_nr
    );
}

fn print_pgcit(indent: &str, pgcit: &Pgcit) {
    for (i, srp) in pgcit.srps.iter().enumerate() {
        print_pgc_summary(indent, &format!("PGC {}{}", i + 1, menu_name(srp.entry_id)), &srp.pgc);
    }
}

fn print_ut(indent: &str, ut: &PgciUt) {
    for lu in &ut.lus {
        println!("{indent}language {} ({} PGCs)", lang(lu.lang_code), lu.pgcit.srps.len());
        print_pgcit(&format!("{indent}  "), &lu.pgcit);
    }
}

fn info(disc: &Disc) {
    let v = &disc.vmgi;
    println!("provider: {:?}", v.provider_identifier);
    println!("title sets: {}, titles: {}", v.nr_of_title_sets, v.titles.len());
    for (i, t) in v.titles.iter().enumerate() {
        println!(
            "  title {}: VTS {} title {}, {} chapters, {} angles",
            i + 1,
            t.title_set_nr,
            t.vts_ttn,
            t.nr_of_ptts,
            t.nr_of_angles
        );
    }
    if let Some(fp) = &v.first_play_pgc {
        print_pgc_summary("", "first play", fp);
    }
    if let Some(ut) = &v.menu_pgci_ut {
        println!("VMGM menus:");
        print_ut("  ", ut);
    }
    for (i, vts) in disc.vts.iter().enumerate() {
        let Some(vts) = vts else {
            println!("VTS {}: unreadable", i + 1);
            continue;
        };
        let (w, h) = vts.video_attr.resolution();
        println!(
            "VTS {}: {}x{} {}, {} audio, {} subpicture streams",
            i + 1,
            w,
            h,
            if vts.video_attr.is_widescreen() { "16:9" } else { "4:3" },
            vts.audio_attr.len(),
            vts.subp_attr.len()
        );
        if let Some(ut) = &vts.menu_pgci_ut {
            println!("  menus:");
            print_ut("    ", ut);
        }
        println!("  titles:");
        print_pgcit("    ", &vts.pgcit);
    }
}

fn disasm_pgc(label: &str, pgc: &Pgc) {
    println!("{label}");
    for (name, cmds) in [("pre", &pgc.pre_cmds), ("post", &pgc.post_cmds), ("cell", &pgc.cell_cmds)] {
        for (i, c) in cmds.iter().enumerate() {
            let hex: Vec<String> = c.0.iter().map(|b| format!("{b:02x}")).collect();
            println!("  {name:>4} {:3}  {}  {}", i + 1, hex.join(" "), decode(*c, Quirks::default()));
        }
    }
}

fn disasm(disc: &Disc) {
    if let Some(fp) = &disc.vmgi.first_play_pgc {
        disasm_pgc("first play", fp);
    }
    let ut_disasm = |prefix: &str, ut: &PgciUt| {
        for lu in &ut.lus {
            for (i, srp) in lu.pgcit.srps.iter().enumerate() {
                disasm_pgc(&format!("{prefix} {} PGC {}{}", lang(lu.lang_code), i + 1, menu_name(srp.entry_id)), &srp.pgc);
            }
        }
    };
    if let Some(ut) = &disc.vmgi.menu_pgci_ut {
        ut_disasm("VMGM", ut);
    }
    for (n, vts) in disc.vts.iter().enumerate() {
        let Some(vts) = vts else { continue };
        if let Some(ut) = &vts.menu_pgci_ut {
            ut_disasm(&format!("VTSM {}", n + 1), ut);
        }
        for (i, srp) in vts.pgcit.srps.iter().enumerate() {
            disasm_pgc(&format!("VTS {} PGC {}", n + 1, i + 1), &srp.pgc);
        }
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (cmd, path) = match args.as_slice() {
        [cmd, path, ..] => (cmd.as_str(), PathBuf::from(path)),
        _ => return Err(USAGE.into()),
    };
    let mut files = DvdFiles::open(&path)?;
    let disc = files.load_disc()?;
    match cmd {
        "info" => info(&disc),
        "extract" => {
            let [vobs, first, last, flag, out] = &args[2..] else { return Err(USAGE.into()) };
            if flag != "--out" {
                return Err(USAGE.into());
            }
            let vobs = source::parse_vobs(vobs).ok_or("bad VOB set id")?;
            let (first, last): (u32, u32) = (first.parse().map_err(|_| "bad first")?, last.parse().map_err(|_| "bad last")?);
            let mut data = vec![];
            for s in first..=last {
                data.extend(files.read_vob_sector(vobs, s)?);
            }
            std::fs::write(out, data).map_err(|e| format!("{out}: {e}"))?;
        }
        "bundle" => {
            let [flag, out] = &args[2..] else { return Err(USAGE.into()) };
            if flag != "--out" {
                return Err(USAGE.into());
            }
            bundle::bundle(&mut files, &disc, &PathBuf::from(out))?;
        }
        "disasm" => disasm(&disc),
        "aspect" => {
            // The domain the disc first plays in, and its video attributes.
            let mut engine = Engine::new(disc, Quirks::default(), 0);
            let opts = trace::Options { max_events: 1, settle: 0 };
            trace::run(&mut engine, &mut files, &[], &opts, &mut |_, _, _| {});
            println!("{}", if engine.video_attr().is_widescreen() { "16:9" } else { "4:3" });
        }
        "trace" | "render" => {
            let mut script = vec![];
            let mut max_events = 200_000;
            let mut quirks = Quirks::default();
            let mut seed = 0;
            let mut settle = 0;
            let mut out_dir: Option<PathBuf> = None;
            let mut rest = args[2..].iter();
            while let Some(a) = rest.next() {
                let mut val = || rest.next().cloned().ok_or(format!("{a} needs a value"));
                match a.as_str() {
                    "--inputs" => {
                        let f = val()?;
                        script = trace::parse_script(&std::fs::read_to_string(&f).map_err(|e| format!("{f}: {e}"))?)?
                    }
                    "-i" => script = trace::parse_script(&val()?)?,
                    "--max-events" => max_events = val()?.parse().map_err(|_| "bad --max-events")?,
                    "--seed" => seed = val()?.parse().map_err(|_| "bad --seed")?,
                    "--compat" => quirks.libdvdnav_compound_cmds = true,
                    "--out" => out_dir = Some(PathBuf::from(val()?)),
                    "--settle" => settle = val()?.parse().map_err(|_| "bad --settle")?,
                    _ => return Err(format!("unknown option {a}\n\n{USAGE}")),
                }
            }
            let mut engine = Engine::new(disc, quirks, seed);
            let opts = trace::Options { max_events, settle };
            let lines = if cmd == "render" {
                let dir = out_dir.ok_or("render needs --out DIR")?;
                std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                let mut n = 0;
                trace::run(&mut engine, &mut files, &script, &opts, &mut |engine, files, input| {
                    n += 1;
                    let label = input.map_or("end".to_string(), trace::input_name).replace(':', "-");
                    let pgcn = engine.pgcn();
                    let vm = &engine.vm;
                    let path = dir.join(format!(
                        "{n:03}-d{}-vts{}-pgc{pgcn}-btn{}-then-{label}.png",
                        vm.domain as u8,
                        vm.vtsn,
                        vm.regs.button()
                    ));
                    match render::render(files, engine) {
                        Ok(f) => {
                            let png = png::encode_rgb(f.width as u32, f.height as u32, &f.rgb);
                            if let Err(e) = std::fs::write(&path, png) {
                                eprintln!("{}: {e}", path.display());
                            }
                        }
                        Err(e) => eprintln!("render failed: {e}"),
                    }
                })
            } else {
                trace::run(&mut engine, &mut files, &script, &opts, &mut |_, _, _| {})
            };
            for line in lines {
                println!("{line}");
            }
        }
        _ => return Err(USAGE.into()),
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
