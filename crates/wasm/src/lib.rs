//! The menu player for the browser, as a plain C ABI.
//!
//! Driven by web/player-worker.js:
//!
//! 1. `mm_add_ifo` for every IFO, `mm_declare_vobs` for every menu VOB set
//!    in the bundle, then `mm_start`.
//! 2. `mm_step(presented_ms)`; on 1 (need data), `mm_wanted` lists the chunks
//!    to fetch, each supplied with `mm_put_chunk`.
//! 3. `mm_next_output` until it returns 0: each output's JSON metadata is in
//!    the result buffer (`mm_result_ptr` / `mm_result_len`) and any bulk data
//!    (RGBA or planar YCbCr, f32 samples, subpicture indices) in the data
//!    buffer (`mm_data_ptr` / `mm_data_len`).
//! 4. `mm_input` for user input, `mm_still_done` when a still has elapsed.
//!
//! One player per module instance.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fmt::Write as _;

use menuseum_core::cmd::Quirks;
use menuseum_core::navigator::{Disc, MenuCall, Vobs};
use menuseum_engine::Engine;
use menuseum_core::ifo::{Vmgi, Vtsi};
use menuseum_core::sectors::SectorCache;
use menuseum_core::spu::Highlight;
use menuseum_media::player::{ButtonInfo, Config, Input, Output, Player, Step};

struct State {
    vmgi: Option<Vmgi>,
    vts: Vec<Option<Vtsi>>,
    sets: Vec<(Vobs, u32)>,
    player: Option<Player<Engine>>,
    outputs: VecDeque<Output>,
    result: Vec<u8>,
    data: Vec<u8>,
    /// Hand frames over as planar YCbCr rather than RGBA (`mm_frame_format`).
    yuv: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State {
        vmgi: None,
        vts: vec![],
        sets: vec![],
        player: None,
        outputs: VecDeque::new(),
        result: vec![],
        data: vec![],
        yuv: false,
    });
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

/// # Safety
/// `ptr`/`len` must describe readable memory.
unsafe fn bytes<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if len == 0 {
        &[]
    } else {
        std::slice::from_raw_parts(ptr, len)
    }
}

fn parse_vobs(s: &str) -> Option<Vobs> {
    if s == "vmgm" {
        return Some(Vobs::Vmgm);
    }
    let n: u8 = s.strip_prefix("menu-")?.parse().ok()?;
    Some(Vobs::Menu(n))
}

fn vobs_id(v: Vobs) -> String {
    match v {
        Vobs::Vmgm => "vmgm".into(),
        Vobs::Menu(n) => format!("menu-{n}"),
        Vobs::Title(n) => format!("title-{n}"),
    }
}

fn fail(s: &mut State, e: String) -> i32 {
    s.result = e.into_bytes();
    0
}

#[no_mangle]
pub extern "C" fn mm_alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `ptr`/`len` must be a live allocation from `mm_alloc`.
#[no_mangle]
pub unsafe extern "C" fn mm_free(ptr: *mut u8, len: usize) {
    drop(Vec::from_raw_parts(ptr, 0, len.max(1)));
}

#[no_mangle]
pub extern "C" fn mm_result_ptr() -> *const u8 {
    with(|s| s.result.as_ptr())
}

#[no_mangle]
pub extern "C" fn mm_result_len() -> usize {
    with(|s| s.result.len())
}

#[no_mangle]
pub extern "C" fn mm_data_ptr() -> *const u8 {
    with(|s| s.data.as_ptr())
}

#[no_mangle]
pub extern "C" fn mm_data_len() -> usize {
    with(|s| s.data.len())
}

/// Add an IFO: `kind` 0 = VIDEO_TS.IFO, n = VTS_nn_0.IFO. Returns 1 on success.
///
/// # Safety
/// `ptr`/`len` must describe readable memory.
#[no_mangle]
pub unsafe extern "C" fn mm_add_ifo(kind: u32, ptr: *const u8, len: usize) -> i32 {
    let data = bytes(ptr, len);
    with(|s| {
        if kind == 0 {
            match Vmgi::parse(data) {
                Ok(v) => s.vmgi = Some(v),
                Err(e) => return fail(s, format!("VIDEO_TS.IFO: {e}")),
            }
        } else {
            let i = kind as usize - 1;
            if s.vts.len() <= i {
                s.vts.resize(i + 1, None);
            }
            match Vtsi::parse(data) {
                Ok(v) => s.vts[i] = Some(v),
                Err(e) => return fail(s, format!("VTS_{kind:02}_0.IFO: {e}")),
            }
        }
        1
    })
}

/// Declare a menu VOB set of `sectors` sectors; the id is "vmgm" or "menu-N".
///
/// # Safety
/// `ptr`/`len` must describe readable memory.
#[no_mangle]
pub unsafe extern "C" fn mm_declare_vobs(ptr: *const u8, len: usize, sectors: u32) -> i32 {
    let id = String::from_utf8_lossy(bytes(ptr, len)).into_owned();
    with(|s| match parse_vobs(&id) {
        Some(v) => {
            s.sets.push((v, sectors));
            1
        }
        None => fail(s, format!("bad VOB set id {id}")),
    })
}

/// Create the player. `compat` selects libdvdnav's quirks; `cache_mb` bounds
/// the sector cache.
#[no_mangle]
pub extern "C" fn mm_start(seed: f64, compat: i32, lookahead_ms: f64, cache_mb: u32) -> i32 {
    with(|s| {
        let Some(vmgi) = s.vmgi.take() else { return fail(s, "VIDEO_TS.IFO missing".into()) };
        let mut vts = std::mem::take(&mut s.vts);
        vts.resize(vmgi.nr_of_title_sets as usize, None);
        let mut cache = SectorCache::new((cache_mb as usize) << 20);
        for &(v, n) in &s.sets {
            cache.declare(v, n);
        }
        let quirks = Quirks { libdvdnav_compound_cmds: compat != 0 };
        let engine = Engine::new(Disc { vmgi, vts }, quirks, seed as u64);
        s.player = Some(Player::new(engine, cache, Config { lookahead_ms, ..Config::default() }));
        1
    })
}

/// How frames are handed over: 0 RGBA (the default), 1 planar YCbCr 4:2:0
/// as decoded (Y, then Cb, then Cr, each row-major; chroma planes
/// `ceil(width / 2)` × `ceil(height / 2)`), for a host that converts on the
/// GPU. YCbCr frames' metadata adds `"format":"yuv"` and `interlaced`
/// (the host must deinterlace as `Picture::to_rgba` does).
#[no_mangle]
pub extern "C" fn mm_frame_format(format: u32) -> i32 {
    with(|s| match format {
        0 | 1 => {
            s.yuv = format == 1;
            1
        }
        _ => fail(s, format!("bad frame format {format}")),
    })
}

/// Decode ahead of `presented_ms`. Returns 0 ahead, 1 need data, 2 in a
/// still, 3 stopped, -1 not started.
#[no_mangle]
pub extern "C" fn mm_step(presented_ms: f64) -> i32 {
    with(|s| {
        let Some(p) = s.player.as_mut() else { return -1 };
        let step = p.step(presented_ms);
        s.outputs.extend(p.take_outputs());
        match step {
            Step::Ahead => 0,
            Step::NeedData => 1,
            Step::Still => 2,
            Step::Stopped => 3,
        }
    })
}

/// JSON list of chunks to fetch: `[[vobs id, chunk, byte start, byte end], ...]`.
#[no_mangle]
pub extern "C" fn mm_wanted() -> i32 {
    with(|s| {
        let Some(p) = s.player.as_ref() else { return 0 };
        let mut j = String::from("[");
        for (i, (v, c)) in p.wanted().into_iter().enumerate() {
            if let Some((a, b)) = p.cache.chunk_range(v, c) {
                let _ = write!(j, "{}[\"{}\",{c},{a},{b}]", if i > 0 { "," } else { "" }, vobs_id(v));
            }
        }
        j.push(']');
        s.result = j.into_bytes();
        1
    })
}

/// Supply a fetched chunk.
///
/// # Safety
/// Pointer/length pairs must describe readable memory.
#[no_mangle]
pub unsafe extern "C" fn mm_put_chunk(id_ptr: *const u8, id_len: usize, chunk: u32, ptr: *const u8, len: usize) -> i32 {
    let id = String::from_utf8_lossy(bytes(id_ptr, id_len)).into_owned();
    let data = bytes(ptr, len).to_vec();
    with(|s| match (s.player.as_mut(), parse_vobs(&id)) {
        (Some(p), Some(v)) => {
            p.cache.insert_chunk(v, chunk, data);
            1
        }
        _ => 0,
    })
}

fn highlight_json(h: &Option<Highlight>) -> String {
    match h {
        None => "null".into(),
        Some(h) => format!("{{\"color\":{:?},\"contrast\":{:?}}}", h.color, h.contrast),
    }
}

fn buttons_json(buttons: &[ButtonInfo]) -> String {
    let items: Vec<String> = buttons
        .iter()
        .map(|b| {
            format!(
                "{{\"x0\":{},\"y0\":{},\"x1\":{},\"y1\":{},\"selected\":{},\"activated\":{}}}",
                b.x0,
                b.y0,
                b.x1,
                b.y1,
                highlight_json(&b.selected),
                highlight_json(&b.activated)
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

/// Pop the next output. Returns 0 when there are none, else a kind:
/// 1 frame, 2 audio, 3 subpicture, 4 buttons, 5 still, 6 placeholder,
/// 7 flush, 8 state, 9 stopped, 10 error.
#[no_mangle]
pub extern "C" fn mm_next_output() -> i32 {
    with(|s| {
        let Some(o) = s.outputs.pop_front() else { return 0 };
        s.data.clear();
        let (kind, meta) = match o {
            Output::Frame { epoch, at_ms, duration_ms, picture } if s.yuv => {
                s.data.extend_from_slice(&picture.y);
                s.data.extend_from_slice(&picture.cb);
                s.data.extend_from_slice(&picture.cr);
                (
                    1,
                    format!(
                        r#"{{"epoch":{epoch},"at":{at_ms},"duration":{duration_ms},"width":{},"height":{},"format":"yuv","interlaced":{}}}"#,
                        picture.width, picture.height, picture.interlaced
                    ),
                )
            }
            Output::Frame { epoch, at_ms, duration_ms, picture } => {
                s.data = picture.to_rgba();
                (
                    1,
                    format!(
                        r#"{{"epoch":{epoch},"at":{at_ms},"duration":{duration_ms},"width":{},"height":{}}}"#,
                        picture.width, picture.height
                    ),
                )
            }
            Output::Audio { epoch, at_ms, sample_rate, samples } => {
                s.data = samples.iter().flat_map(|v| v.to_le_bytes()).collect();
                (2, format!(r#"{{"epoch":{epoch},"at":{at_ms},"rate":{sample_rate},"frames":{}}}"#, samples.len() / 2))
            }
            Output::Subpicture { epoch, at_ms, until_ms, spu, palette_rgb } => {
                s.data = spu.pixels.clone();
                (
                    3,
                    format!(
                        r#"{{"epoch":{epoch},"at":{at_ms},"until":{},"x":{},"y":{},"width":{},"height":{},"color":{:?},"contrast":{:?},"palette":{:?}}}"#,
                        until_ms.map_or("null".into(), |u| u.to_string()),
                        spu.x,
                        spu.y,
                        spu.width,
                        spu.height,
                        spu.color,
                        spu.contrast,
                        palette_rgb
                    ),
                )
            }
            Output::Buttons { epoch, at_ms, buttons } => {
                (4, format!(r#"{{"epoch":{epoch},"at":{at_ms},"buttons":{}}}"#, buttons_json(&buttons)))
            }
            Output::Still { epoch, at_ms, seconds } => {
                (5, format!(r#"{{"epoch":{epoch},"at":{at_ms},"seconds":{seconds}}}"#))
            }
            Output::Placeholder { epoch, at_ms, duration_ms, title, chapter } => (
                6,
                format!(r#"{{"epoch":{epoch},"at":{at_ms},"duration":{duration_ms},"title":{title},"chapter":{chapter}}}"#),
            ),
            Output::Flush { epoch } => (7, format!(r#"{{"epoch":{epoch}}}"#)),
            Output::State(st) => (
                8,
                format!(
                    r#"{{"domain":{},"vts":{},"pgc":{},"pg":{},"cell":{},"button":{},"width":{},"height":{},"widescreen":{},"sprm":{:?},"gprm":{:?}}}"#,
                    st.domain as u8,
                    st.vts,
                    st.pgcn,
                    st.pgn,
                    st.cell,
                    st.button,
                    st.width,
                    st.height,
                    st.widescreen,
                    st.sprm,
                    st.gprm
                ),
            ),
            Output::Stopped => (9, "{}".into()),
            Output::Error(e) => (10, format!(r#"{{"message":{e:?}}}"#)),
        };
        s.result = meta.into_bytes();
        kind
    })
}

/// Apply input. kind: 0 up, 1 down, 2 left, 3 right, 4 activate, 5 select(a),
/// 6 menu(a: 0 escape, 1 title, 2 root, 3 subpicture, 4 audio, 5 angle, 6 part),
/// 7 pointer(a, b), 8 click(a, b). Returns 1 if the input did something.
#[no_mangle]
pub extern "C" fn mm_input(kind: u32, a: u32, b: u32) -> i32 {
    let input = match kind {
        0 => Input::Up,
        1 => Input::Down,
        2 => Input::Left,
        3 => Input::Right,
        4 => Input::Activate,
        5 => Input::Select(a as u8),
        6 => Input::Menu(match a {
            0 => MenuCall::Escape,
            1 => MenuCall::Title,
            2 => MenuCall::Root,
            3 => MenuCall::Subpicture,
            4 => MenuCall::Audio,
            5 => MenuCall::Angle,
            _ => MenuCall::Part,
        }),
        7 => Input::Pointer(a as u16, b as u16),
        8 => Input::Click(a as u16, b as u16),
        _ => return 0,
    };
    with(|s| {
        let Some(p) = s.player.as_mut() else { return 0 };
        let ok = p.input(input);
        s.outputs.extend(p.take_outputs());
        ok as i32
    })
}

#[no_mangle]
pub extern "C" fn mm_still_done(epoch: u32) {
    with(|s| {
        if let Some(p) = s.player.as_mut() {
            p.still_done(epoch);
        }
    })
}
