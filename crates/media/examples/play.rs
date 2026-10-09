//! Play a bundle (a VIDEO_TS directory without title VOBs) headlessly with a
//! simulated clock and scripted input, and summarise what came out.
//!
//! usage: play <VIDEO_TS dir> [seconds] ['<time>:<input> ...']
//! inputs: up down left right activate menu
use std::collections::BTreeMap;
use std::path::Path;

use menuseum_core::navigator::{Disc, MenuCall, Vobs};
use menuseum_engine::Engine;
use menuseum_core::ifo::{Vmgi, Vtsi, SECTOR};
use menuseum_core::sectors::SectorCache;
use menuseum_media::player::{Config, Input, Output, Player, Step};

fn vob_path(dir: &Path, v: Vobs) -> std::path::PathBuf {
    match v {
        Vobs::Vmgm => dir.join("VIDEO_TS.VOB"),
        Vobs::Menu(n) => dir.join(format!("VTS_{n:02}_0.VOB")),
        Vobs::Title(n) => dir.join(format!("VTS_{n:02}_1.VOB")),
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let dir = Path::new(&a[1]);
    let seconds: f64 = a.get(2).map_or(60.0, |s| s.parse().unwrap());
    let mut script: BTreeMap<u64, Input> = BTreeMap::new();
    for tok in a.get(3).map_or("", |s| s.as_str()).split_whitespace() {
        let (t, i) = tok.split_once(':').unwrap();
        let input = match i {
            "up" => Input::Up,
            "down" => Input::Down,
            "left" => Input::Left,
            "right" => Input::Right,
            "activate" => Input::Activate,
            "menu" => Input::Menu(MenuCall::Escape),
            _ => panic!("bad input {i}"),
        };
        script.insert((t.parse::<f64>().unwrap() * 1000.0) as u64, input);
    }
    let vmgi = Vmgi::parse(&std::fs::read(dir.join("VIDEO_TS.IFO")).unwrap()).unwrap();
    let vts = (1..=vmgi.nr_of_title_sets)
        .map(|n| std::fs::read(dir.join(format!("VTS_{n:02}_0.IFO"))).ok().and_then(|d| Vtsi::parse(&d).ok()))
        .collect();
    let mut cache = SectorCache::new(64 << 20);
    let mut sets = vec![Vobs::Vmgm];
    sets.extend((1..=vmgi.nr_of_title_sets as u8).map(Vobs::Menu));
    for v in sets {
        if let Ok(m) = std::fs::metadata(vob_path(dir, v)) {
            cache.declare(v, (m.len() / SECTOR as u64) as u32);
        }
    }
    let engine = Engine::new(Disc { vmgi, vts }, Default::default(), 0);
    let mut player = Player::new(engine, cache, Config::default());

    let (mut frames, mut audio_s, mut fetched, mut late) = (0, 0.0, 0, 0);
    let mut still_until: Option<(u32, f64)> = None;
    let t0 = std::time::Instant::now();
    let mut now = 0.0;
    while now < seconds * 1000.0 {
        // Present whatever is due; hold the clock during stills.
        while let Some((&t, &input)) = script.iter().next() {
            if t as f64 > now {
                break;
            }
            script.remove(&t);
            let ok = player.input(input);
            println!("{:7.1}s input {input:?} -> {ok}", now / 1000.0);
        }
        loop {
            match player.step(now) {
                Step::NeedData => {
                    for (v, c) in player.wanted() {
                        let path = vob_path(dir, v);
                        let (start, end) = player.cache.chunk_range(v, c).unwrap();
                        let mut f = std::fs::File::open(&path).unwrap();
                        let mut buf = vec![0u8; (end - start) as usize];
                        use std::io::{Read, Seek, SeekFrom};
                        f.seek(SeekFrom::Start(start)).unwrap();
                        f.read_exact(&mut buf).unwrap();
                        player.cache.insert_chunk(v, c, buf);
                        fetched += 1;
                    }
                }
                _ => break,
            }
        }
        for o in player.take_outputs() {
            match o {
                Output::Frame { at_ms, .. } => {
                    frames += 1;
                    if at_ms < now - 50.0 {
                        late += 1;
                    }
                }
                Output::Audio { samples, sample_rate, .. } => audio_s += samples.len() as f64 / 2.0 / sample_rate as f64,
                Output::Still { epoch, at_ms, seconds } => {
                    println!("{:7.1}s still {seconds}s at {:.1}s", now / 1000.0, at_ms / 1000.0);
                    if seconds != 0xff {
                        still_until = Some((epoch, at_ms + seconds as f64 * 1000.0));
                    }
                }
                Output::Placeholder { at_ms, duration_ms, title, chapter, .. } => {
                    println!("{:7.1}s placeholder title {title} chapter {chapter} at {:.1}s for {duration_ms}ms", now / 1000.0, at_ms / 1000.0)
                }
                Output::State(s) => println!(
                    "{:7.1}s state d={:?} vts={} pgc={} cell={} btn={}",
                    now / 1000.0,
                    s.domain,
                    s.vts,
                    s.pgcn,
                    s.cell,
                    s.button
                ),
                Output::Buttons { at_ms, buttons, .. } => {
                    println!("{:7.1}s buttons x{} from {:.1}s", now / 1000.0, buttons.len(), at_ms / 1000.0)
                }
                Output::Subpicture { at_ms, spu, .. } => println!(
                    "{:7.1}s subpicture {}x{} at {:.1}s",
                    now / 1000.0,
                    spu.width,
                    spu.height,
                    at_ms / 1000.0
                ),
                Output::Flush { epoch } => println!("{:7.1}s flush -> epoch {epoch}", now / 1000.0),
                Output::Stopped => println!("{:7.1}s stopped", now / 1000.0),
                Output::Error(e) => println!("{:7.1}s ERROR {e}", now / 1000.0),
            }
        }
        if let Some((epoch, until)) = still_until {
            if now >= until {
                player.still_done(epoch);
                still_until = None;
            }
        }
        now += 40.0;
    }
    println!(
        "{frames} frames, {audio_s:.1}s audio, {fetched} chunks fetched, {late} frames late, wall {:.1}s for {seconds}s",
        t0.elapsed().as_secs_f64()
    );
}
