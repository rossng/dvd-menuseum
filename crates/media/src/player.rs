//! The real-time menu player: a [`Navigator`] driving the decoders.
//!
//! The host calls [`Player::step`] with how far presentation has got, feeds
//! it sectors when it reports [`Step::NeedData`], and forwards user input.
//! The player runs ahead of presentation by up to [`Config::lookahead_ms`],
//! producing [`Output`]s placed on one continuous **timeline** in
//! milliseconds: MPEG timestamps restart at cell boundaries, the timeline
//! doesn't.
//!
//! Overlay state is not baked into frames. Subpictures and highlight
//! information come out as their own timed outputs, and the selected button
//! is live state ([`Output::State`]), so the host can draw the highlight the
//! moment the user moves it, even for frames decoded ahead.

use std::collections::{HashMap, VecDeque};

use menuseum_core::demux::{packets, SpuAssembler};
use menuseum_core::navigator::{Domain, Event, MenuCall, Navigator, Vobs};
use menuseum_core::nav::Hli;
use menuseum_core::sectors::SectorCache;
use menuseum_core::spu::{button_highlight, ycrcb_to_rgb, Highlight, HighlightMode, Spu};

use crate::audio::{AudioChunk, AudioDecoder, Codec};
use crate::video::{Picture, VideoDecoder};

#[derive(Clone, Debug)]
pub struct Config {
    /// How far ahead of presentation to decode.
    pub lookahead_ms: f64,
    /// How long the first stripped cell of a PGC is shown; later cells of the
    /// same PGC get a tenth of that.
    pub stripped_ms: f64,
}

impl Default for Config {
    fn default() -> Self {
        Config { lookahead_ms: 800.0, stripped_ms: 3000.0 }
    }
}

/// A button as the host needs it to draw highlights and hit-test.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ButtonInfo {
    pub selected: Option<Highlight>,
    pub activated: Option<Highlight>,
    pub x0: u16,
    pub y0: u16,
    pub x1: u16,
    pub y1: u16,
}

/// Navigation state, for the host's UI and debugging.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub domain: Domain,
    pub vts: u8,
    pub pgcn: u16,
    pub pgn: u8,
    pub cell: u8,
    pub button: u8,
    /// Picture size and display aspect of the current domain's video.
    pub width: u16,
    pub height: u16,
    pub widescreen: bool,
    pub sprm: [u16; 24],
    pub gprm: [u16; 16],
}

#[derive(Clone, Debug)]
pub enum Output {
    /// Discard everything from earlier epochs: playback jumped.
    Flush { epoch: u32 },
    Frame { epoch: u32, at_ms: f64, duration_ms: f64, picture: Picture },
    Audio { epoch: u32, at_ms: f64, sample_rate: u32, samples: Vec<f32> },
    /// A subpicture shown from `at_ms` until `until_ms` (or until replaced).
    Subpicture { epoch: u32, at_ms: f64, until_ms: Option<f64>, spu: Spu, palette_rgb: [[u8; 3]; 16] },
    /// Button information valid from `at_ms` (empty: no buttons).
    Buttons { epoch: u32, at_ms: f64, buttons: Vec<ButtonInfo> },
    /// Hold the picture at `at_ms` for `seconds` (0xff = until input), then
    /// call [`Player::still_done`].
    Still { epoch: u32, at_ms: f64, seconds: u8 },
    /// A cell whose data was stripped from the bundle (the film).
    Placeholder { epoch: u32, at_ms: f64, duration_ms: f64, title: u16, chapter: u16 },
    State(State),
    Stopped,
    Error(String),
}

/// What [`Player::step`] stopped on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Decoded as far ahead as allowed; call again as presentation advances.
    Ahead,
    /// Waiting for sectors: fetch [`Player::wanted`] and call again.
    NeedData,
    /// Waiting for the host to finish a still.
    Still,
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    Up,
    Down,
    Left,
    Right,
    Activate,
    Select(u8),
    Menu(MenuCall),
    /// Pointer moved to (x, y) in picture pixels.
    Pointer(u16, u16),
    /// Click at (x, y): select the button there and activate it.
    Click(u16, u16),
}

/// A VOBU being presented, keyed by tag.
#[derive(Clone, Debug)]
struct Vobu {
    epoch: u32,
    start_ms: f64,
    s_ptm: u32,
    /// Timeline position of the next frame from this VOBU.
    next_frame_ms: f64,
}

struct Job {
    tag: u64,
    vobs: Vobs,
    first: u32,
    last: u32,
}

pub struct Player<N: Navigator> {
    pub nav: N,
    pub cache: SectorCache,
    config: Config,
    video: VideoDecoder,
    audio: Option<(u8, Option<u8>, AudioDecoder)>,
    spu: SpuAssembler,
    vobus: HashMap<u64, Vobu>,
    next_tag: u64,
    job: Option<Job>,
    /// Timeline position where the next VOBU starts.
    cursor_ms: f64,
    presented_ms: f64,
    epoch: u32,
    in_still: Option<u32>,
    stopped: bool,
    last_buttons: Option<Vec<ButtonInfo>>,
    last_state: Option<State>,
    last_stripped_pgc: Option<(Domain, u8, u16)>,
    outputs: VecDeque<Output>,
}

fn rel_ms(pts: u32, base: u32) -> f64 {
    pts.wrapping_sub(base) as i32 as f64 / 90.0
}

impl<N: Navigator> Player<N> {
    pub fn new(nav: N, cache: SectorCache, config: Config) -> Self {
        Player {
            nav,
            cache,
            config,
            video: VideoDecoder::default(),
            audio: None,
            spu: SpuAssembler::default(),
            vobus: HashMap::new(),
            next_tag: 1,
            job: None,
            cursor_ms: 0.0,
            presented_ms: 0.0,
            epoch: 0,
            in_still: None,
            stopped: false,
            last_buttons: None,
            last_state: None,
            last_stripped_pgc: None,
            outputs: VecDeque::new(),
        }
    }

    pub fn take_outputs(&mut self) -> Vec<Output> {
        self.outputs.drain(..).collect()
    }

    /// Chunks the host should fetch next.
    pub fn wanted(&self) -> Vec<(Vobs, u32)> {
        let around = self.job.as_ref().map(|j| (j.vobs, j.first)).or_else(|| {
            let pci = self.nav.pci();
            Some((self.nav.vobs(), pci.nv_pck_lbn))
        });
        self.cache.wanted(around, 8)
    }

    fn field_ms(&self) -> f64 {
        if self.nav.video_attr().video_format == 1 {
            20.0
        } else {
            1001.0 / 60.0
        }
    }

    /// The (stream id, substream) of the audio stream to play.
    fn audio_stream(&self) -> Option<(u8, Option<u8>)> {
        let n = self.nav.audio_active_stream()?;
        let disc = self.nav.disc();
        let vts = disc.vts(self.nav.status().vtsn);
        let format = match self.nav.status().domain {
            Domain::Title => vts.and_then(|v| v.audio_attr.get(n as usize)).map(|a| a.format),
            Domain::Vtsm => vts.and_then(|v| v.menu_audio_attr.first()).map(|a| a.format),
            Domain::Vmgm | Domain::FirstPlay => disc.vmgi.menu_audio_attr.first().map(|a| a.format),
        };
        Codec::stream_for(format.unwrap_or(0), n)
    }

    fn emit_state(&mut self) {
        let pgcn = self.nav.pgcn();
        let attr = self.nav.video_attr();
        let (width, height) = attr.resolution();
        let vm = self.nav.status();
        let regs = self.nav.regs();
        let state = State {
            domain: vm.domain,
            vts: vm.vtsn,
            pgcn,
            pgn: vm.pgn,
            cell: vm.celln,
            button: regs.button(),
            width: width as u16,
            height: height as u16,
            widescreen: attr.is_widescreen(),
            sprm: regs.sprm,
            gprm: regs.gprm,
        };
        if self.last_state.as_ref() != Some(&state) {
            self.last_state = Some(state.clone());
            self.outputs.push_back(Output::State(state));
        }
    }

    fn buttons(hli: &Hli) -> Vec<ButtonInfo> {
        if hli.hli_ss == 0 {
            return vec![];
        }
        (1..=hli.btn_ns)
            .filter_map(|n| {
                let b = hli.button(n)?;
                Some(ButtonInfo {
                    selected: button_highlight(hli, n, HighlightMode::Selected),
                    activated: button_highlight(hli, n, HighlightMode::Activated),
                    x0: b.x_start,
                    y0: b.y_start,
                    x1: b.x_end,
                    y1: b.y_end,
                })
            })
            .collect()
    }

    /// Decode as far ahead of `presented_ms` as allowed.
    pub fn step(&mut self, presented_ms: f64) -> Step {
        self.presented_ms = presented_ms;
        loop {
            if let Some(job) = &self.job {
                let Some(data) = self.cache.range(job.vobs, job.first, job.last) else { return Step::NeedData };
                let tag = job.tag;
                self.job = None;
                self.decode_vobu(tag, &data);
                continue;
            }
            if self.stopped {
                return Step::Stopped;
            }
            if self.in_still.is_some() {
                return Step::Still;
            }
            if self.cursor_ms > presented_ms + self.config.lookahead_ms {
                return Step::Ahead;
            }
            self.nav.set_now_ms(self.cursor_ms.max(0.0) as u64);
            let ev = self.nav.next_event(&mut self.cache);
            match ev {
                Event::Nop => {}
                Event::Pending { .. } => return Step::NeedData,
                Event::Wait => self.nav.wait_skip(),
                Event::Stop => {
                    self.finish_video();
                    self.emit_state();
                    self.outputs.push_back(Output::Stopped);
                    self.stopped = true;
                }
                Event::Error(e) => {
                    self.outputs.push_back(Output::Error(e));
                    self.stopped = true;
                }
                Event::Still(seconds) => {
                    // The last picture of the cell is held in the decoder's
                    // reorder buffer: release it before holding.
                    self.finish_video();
                    self.emit_state();
                    self.outputs.push_back(Output::Still { epoch: self.epoch, at_ms: self.cursor_ms, seconds });
                    self.in_still = Some(self.epoch);
                }
                Event::CellChange { .. } => {
                    self.emit_state();
                    self.check_stripped();
                }
                Event::Nav { lbn } => self.start_vobu(lbn),
                Event::HopChannel
                | Event::Highlight { .. }
                | Event::VtsChange { .. }
                | Event::SpuClutChange
                | Event::SpuStreamChange
                | Event::AudioStreamChange => self.emit_state(),
            }
        }
    }

    fn check_stripped(&mut self) {
        let vobs = self.nav.vobs();
        if self.cache.size(vobs).is_some() {
            if self.nav.status().domain != Domain::Title {
                self.last_stripped_pgc = None;
            }
            return;
        }
        let pgc = Some((self.nav.status().domain, self.nav.status().vtsn, self.nav.pgcn()));
        let duration_ms =
            if pgc == self.last_stripped_pgc { self.config.stripped_ms / 10.0 } else { self.config.stripped_ms };
        self.last_stripped_pgc = pgc;
        self.outputs.push_back(Output::Placeholder {
            epoch: self.epoch,
            at_ms: self.cursor_ms,
            duration_ms,
            title: self.nav.regs().sprm[4],
            chapter: self.nav.regs().sprm[7],
        });
        self.cursor_ms += duration_ms;
    }

    fn start_vobu(&mut self, lbn: u32) {
        let (pci, dsi) = (self.nav.pci().clone(), self.nav.dsi().clone());
        let tag = self.next_tag;
        self.next_tag += 1;
        let start_ms = self.cursor_ms;
        self.vobus.insert(tag, Vobu { epoch: self.epoch, start_ms, s_ptm: pci.vobu_s_ptm, next_frame_ms: start_ms });
        // Keep only recent VOBUs (decoder output lags input by a few).
        self.vobus.retain(|t, _| *t + 64 > tag);
        self.cursor_ms += pci.vobu_e_ptm.wrapping_sub(pci.vobu_s_ptm) as f64 / 90.0;

        let buttons = Self::buttons(&pci.hli);
        if self.last_buttons.as_ref() != Some(&buttons) {
            self.last_buttons = Some(buttons.clone());
            self.outputs.push_back(Output::Buttons { epoch: self.epoch, at_ms: start_ms, buttons });
        }
        self.job = Some(Job { tag, vobs: self.nav.vobs(), first: lbn, last: lbn + dsi.vobu_ea });
    }

    fn decode_vobu(&mut self, tag: u64, data: &[u8]) {
        let audio_stream = self.audio_stream();
        if self.audio.as_ref().map(|a| (a.0, a.1)) != audio_stream {
            self.audio = audio_stream.and_then(|(id, sub)| {
                Codec::of(id, sub).map(|codec| (id, sub, AudioDecoder::new(codec)))
            });
        }
        let spu_stream = self.nav.subp_active_stream(0);
        let forced_only = self.nav.status().domain == Domain::Title && spu_stream.is_some_and(|s| s & 0x80 != 0);
        let spu_sub = spu_stream.map(|s| 0x20 | (s & 0x1f));
        let palette = self.nav.pgc().map(|p| p.palette).unwrap_or([0; 16]);

        let mut pictures = vec![];
        let mut chunks: Vec<AudioChunk> = vec![];
        for sector in data.chunks(2048) {
            for p in packets(sector) {
                if p.stream_id == 0xe0 {
                    pictures.extend(self.video.push(p.payload, tag));
                } else if Some((p.stream_id, p.substream)) == self.audio.as_ref().map(|a| (a.0, a.1)) {
                    if let Some((_, _, dec)) = self.audio.as_mut() {
                        dec.push(p.payload, p.pts, tag, &mut chunks);
                    }
                } else if p.stream_id == 0xbd && p.substream.is_some() && p.substream == spu_sub {
                    if let Some((unit, pts)) = self.spu.push(p.payload, p.pts) {
                        self.emit_spu(tag, &unit, pts, &palette, forced_only);
                    }
                }
            }
        }
        for (picture, tag) in pictures {
            self.emit_frame(picture, tag);
        }
        for chunk in chunks {
            let Some(v) = self.vobus.get(&chunk.tag) else { continue };
            if v.epoch != self.epoch {
                continue;
            }
            self.outputs.push_back(Output::Audio {
                epoch: v.epoch,
                at_ms: v.start_ms + rel_ms(chunk.pts, v.s_ptm),
                sample_rate: chunk.sample_rate,
                samples: chunk.samples,
            });
        }
    }

    fn emit_frame(&mut self, picture: Picture, tag: u64) {
        let field_ms = self.field_ms();
        let Some(v) = self.vobus.get_mut(&tag) else { return };
        let duration_ms = picture.fields as f64 * field_ms;
        let at_ms = v.next_frame_ms;
        v.next_frame_ms += duration_ms;
        if v.epoch == self.epoch {
            self.outputs.push_back(Output::Frame { epoch: v.epoch, at_ms, duration_ms, picture });
        }
    }

    fn finish_video(&mut self) {
        for (picture, tag) in self.video.flush() {
            self.emit_frame(picture, tag);
        }
    }

    fn emit_spu(&mut self, tag: u64, unit: &[u8], pts: Option<u32>, palette: &[u32; 16], forced_only: bool) {
        let Ok(spu) = Spu::parse(unit) else { return };
        if forced_only && !spu.forced {
            return;
        }
        let Some(v) = self.vobus.get(&tag) else { return };
        let base = v.start_ms + pts.map_or(0.0, |p| rel_ms(p, v.s_ptm));
        let at_ms = base + spu.start.unwrap_or(0) as f64 / 90.0;
        let until_ms = spu.stop.map(|s| base + s as f64 / 90.0);
        let mut palette_rgb = [[0u8; 3]; 16];
        for (i, &e) in palette.iter().enumerate() {
            palette_rgb[i] = ycrcb_to_rgb(e);
        }
        self.outputs.push_back(Output::Subpicture { epoch: v.epoch, at_ms, until_ms, spu, palette_rgb });
    }

    /// The host finished presenting a still.
    pub fn still_done(&mut self, epoch: u32) {
        if self.in_still == Some(epoch) {
            self.in_still = None;
            self.nav.still_skip();
        }
    }

    /// Apply user input. Returns whether it did anything.
    pub fn input(&mut self, input: Input) -> bool {
        let vm = self.nav.status();
        let before = (vm.hop_channel, vm.domain, vm.vtsn, vm.celln, vm.cell_restart);
        let ok = match input {
            Input::Up => self.nav.button_up(),
            Input::Down => self.nav.button_down(),
            Input::Left => self.nav.button_left(),
            Input::Right => self.nav.button_right(),
            Input::Activate => self.nav.button_activate(),
            Input::Select(n) => self.nav.button_select(n),
            Input::Menu(m) => self.nav.menu_call(m),
            Input::Pointer(x, y) => self.nav.mouse_select(x, y),
            Input::Click(x, y) => self.nav.mouse_select(x, y) && self.nav.button_activate(),
        };
        let vm = self.nav.status();
        if (vm.hop_channel, vm.domain, vm.vtsn, vm.celln, vm.cell_restart) != before || vm.stopped {
            // Playback moved: drop what was decoded ahead and continue from
            // the present. The video decoder keeps its state: a jump target
            // may be authored to follow the picture being shown.
            self.epoch += 1;
            self.outputs.push_back(Output::Flush { epoch: self.epoch });
            self.cursor_ms = self.presented_ms;
            self.job = None;
            self.in_still = None;
            self.last_buttons = None;
            self.spu = SpuAssembler::default();
        }
        self.emit_state();
        ok
    }
}
