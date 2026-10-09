//! IFO parsing: the Video Manager (`VIDEO_TS.IFO`) and Video Title Set
//! (`VTS_nn_0.IFO`) information files.
//!
//! Layouts follow the mpucoder DVD-Video pages and libdvdread's `ifo_types.h`.
//! Only the tables the navigator needs are parsed; everything is owned so the
//! raw IFO bytes can be dropped afterwards.

use crate::bytes::Bytes;
use crate::Error;

pub const SECTOR: usize = 2048;

/// BCD playback time; the top two bits of `frame` carry the frame rate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DvdTime {
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub frame: u8,
}

impl DvdTime {
    fn parse(b: [u8; 4]) -> Self {
        DvdTime { hour: b[0], minute: b[1], second: b[2], frame: b[3] }
    }

    fn bcd(v: u8) -> u32 {
        (v >> 4) as u32 * 10 + (v & 0x0f) as u32
    }

    /// Frames per second encoded in the top bits of the frame byte (25, 30 or 0 if illegal).
    pub fn fps(&self) -> u32 {
        match self.frame >> 6 {
            1 => 25,
            3 => 30,
            _ => 0,
        }
    }

    pub fn whole_seconds(&self) -> u32 {
        Self::bcd(self.hour) * 3600 + Self::bcd(self.minute) * 60 + Self::bcd(self.second)
    }

    pub fn millis(&self) -> u64 {
        let frames = Self::bcd(self.frame & 0x3f) as u64;
        let frame_ms = match self.fps() {
            0 => 0,
            fps => frames * 1000 / fps as u64,
        };
        self.whole_seconds() as u64 * 1000 + frame_ms
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VideoAttr {
    pub mpeg_version: u8,
    /// 0 = NTSC, 1 = PAL.
    pub video_format: u8,
    /// 0 = 4:3, 3 = 16:9.
    pub display_aspect_ratio: u8,
    /// Bit 1: pan&scan disallowed, bit 0: letterbox disallowed.
    pub permitted_df: u8,
    pub picture_size: u8,
    pub letterboxed: bool,
    pub film_mode: bool,
}

impl VideoAttr {
    fn parse(b: [u8; 2]) -> Self {
        VideoAttr {
            mpeg_version: b[0] >> 6,
            video_format: (b[0] >> 4) & 3,
            display_aspect_ratio: (b[0] >> 2) & 3,
            permitted_df: b[0] & 3,
            picture_size: (b[1] >> 2) & 3,
            letterboxed: b[1] & 2 != 0,
            film_mode: b[1] & 1 != 0,
        }
    }

    pub fn is_widescreen(&self) -> bool {
        self.display_aspect_ratio == 3
    }

    /// Frame size in pixels as (width, height).
    pub fn resolution(&self) -> (u32, u32) {
        let height = if self.video_format == 1 { 576 } else { 480 };
        let width = match self.picture_size {
            0 => 720,
            1 => 704,
            2 => 352,
            _ => 352,
        };
        let height = if self.picture_size == 3 { height / 2 } else { height };
        (width, height)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AudioAttr {
    /// 0 AC-3, 2 MPEG-1, 3 MPEG-2 ext, 4 LPCM, 6 DTS.
    pub format: u8,
    pub lang_type: u8,
    pub lang_code: u16,
    pub channels: u8,
    pub code_extension: u8,
}

impl AudioAttr {
    fn parse(b: [u8; 8]) -> Self {
        AudioAttr {
            format: b[0] >> 5,
            lang_type: (b[0] >> 2) & 3,
            lang_code: u16::from_be_bytes([b[2], b[3]]),
            channels: (b[1] & 7) + 1,
            code_extension: b[5],
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SubpAttr {
    pub code_mode: u8,
    pub lang_type: u8,
    pub lang_code: u16,
    pub code_extension: u8,
}

impl SubpAttr {
    fn parse(b: [u8; 6]) -> Self {
        SubpAttr {
            code_mode: b[0] >> 5,
            lang_type: b[0] & 3,
            lang_code: u16::from_be_bytes([b[2], b[3]]),
            code_extension: b[5],
        }
    }
}

/// One 8-byte navigation command.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RawCmd(pub [u8; 8]);

impl RawCmd {
    pub fn as_u64(&self) -> u64 {
        u64::from_be_bytes(self.0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellPlayback {
    /// 0 not in block, 1 first cell, 2 in block, 3 last cell.
    pub block_mode: u8,
    /// 0 none, 1 angle block.
    pub block_type: u8,
    pub seamless_play: bool,
    pub interleaved: bool,
    pub stc_discontinuity: bool,
    pub seamless_angle: bool,
    /// Still after every VOBU.
    pub vobu_still: bool,
    pub restricted: bool,
    pub cell_type: u8,
    /// Seconds; 0xff = infinite.
    pub still_time: u8,
    /// 1-based index into the PGC's cell commands; 0 = none.
    pub cell_cmd_nr: u8,
    pub playback_time: DvdTime,
    pub first_sector: u32,
    pub first_ilvu_end_sector: u32,
    pub last_vobu_start_sector: u32,
    pub last_sector: u32,
}

impl CellPlayback {
    fn parse(b: Bytes, off: usize) -> Result<Self, Error> {
        let c0 = b.u8(off)?;
        let c1 = b.u8(off + 1)?;
        Ok(CellPlayback {
            block_mode: c0 >> 6,
            block_type: (c0 >> 4) & 3,
            seamless_play: c0 & 8 != 0,
            interleaved: c0 & 4 != 0,
            stc_discontinuity: c0 & 2 != 0,
            seamless_angle: c0 & 1 != 0,
            vobu_still: c1 & 0x40 != 0,
            restricted: c1 & 0x20 != 0,
            cell_type: c1 & 0x1f,
            still_time: b.u8(off + 2)?,
            cell_cmd_nr: b.u8(off + 3)?,
            playback_time: DvdTime::parse(b.array(off + 4)?),
            first_sector: b.u32(off + 8)?,
            first_ilvu_end_sector: b.u32(off + 12)?,
            last_vobu_start_sector: b.u32(off + 16)?,
            last_sector: b.u32(off + 20)?,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellPosition {
    pub vob_id: u16,
    pub cell_id: u8,
}

/// Program Chain.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pgc {
    pub nr_of_programs: u8,
    pub nr_of_cells: u8,
    pub playback_time: DvdTime,
    pub prohibited_uops: u32,
    pub audio_control: [u16; 8],
    pub subp_control: [u32; 32],
    pub next_pgc_nr: u16,
    pub prev_pgc_nr: u16,
    pub goup_pgc_nr: u16,
    pub pg_playback_mode: u8,
    /// Seconds; 0xff = infinite.
    pub still_time: u8,
    /// 16 colours, each 0x00YYCrCb.
    pub palette: [u32; 16],
    pub pre_cmds: Vec<RawCmd>,
    pub post_cmds: Vec<RawCmd>,
    pub cell_cmds: Vec<RawCmd>,
    /// Entry cell (1-based) of each program.
    pub program_map: Vec<u8>,
    pub cells: Vec<CellPlayback>,
    pub cell_positions: Vec<CellPosition>,
}

impl Pgc {
    pub(crate) fn parse(b: Bytes) -> Result<Self, Error> {
        let b = b.with_what("PGC");
        let mut pgc = Pgc {
            nr_of_programs: b.u8(2)?,
            nr_of_cells: b.u8(3)?,
            playback_time: DvdTime::parse(b.array(4)?),
            prohibited_uops: b.u32(8)?,
            next_pgc_nr: b.u16(156)?,
            prev_pgc_nr: b.u16(158)?,
            goup_pgc_nr: b.u16(160)?,
            pg_playback_mode: b.u8(162)?,
            still_time: b.u8(163)?,
            ..Default::default()
        };
        for i in 0..8 {
            pgc.audio_control[i] = b.u16(12 + 2 * i)?;
        }
        for i in 0..32 {
            pgc.subp_control[i] = b.u32(28 + 4 * i)?;
        }
        for i in 0..16 {
            pgc.palette[i] = b.u32(164 + 4 * i)?;
        }
        let cmd_off = b.u16(228)? as usize;
        let map_off = b.u16(230)? as usize;
        let playback_off = b.u16(232)? as usize;
        let position_off = b.u16(234)? as usize;

        if cmd_off != 0 {
            let t = b.from(cmd_off)?.with_what("PGC command table");
            let (pre, post, cell) = (t.u16(0)? as usize, t.u16(2)? as usize, t.u16(4)? as usize);
            // 128 pre + 128 post + 255 cell is the spec maximum; anything bigger is junk.
            if pre + post + cell <= 128 + 128 + 255 {
                let cmd = |i: usize| -> Result<RawCmd, Error> { Ok(RawCmd(t.array(8 + 8 * i)?)) };
                pgc.pre_cmds = (0..pre).map(cmd).collect::<Result<_, _>>()?;
                pgc.post_cmds = (pre..pre + post).map(cmd).collect::<Result<_, _>>()?;
                pgc.cell_cmds = (pre + post..pre + post + cell).map(cmd).collect::<Result<_, _>>()?;
            }
        }
        if map_off != 0 && pgc.nr_of_programs > 0 {
            pgc.program_map = b.slice(map_off, pgc.nr_of_programs as usize)?.to_vec();
        } else {
            pgc.nr_of_programs = 0;
        }
        if playback_off != 0 && pgc.nr_of_cells > 0 {
            pgc.cells = (0..pgc.nr_of_cells as usize)
                .map(|i| CellPlayback::parse(b, playback_off + 24 * i))
                .collect::<Result<_, _>>()?;
        } else {
            pgc.nr_of_cells = 0;
        }
        if position_off != 0 && pgc.nr_of_cells > 0 {
            pgc.cell_positions = (0..pgc.nr_of_cells as usize)
                .map(|i| -> Result<_, Error> {
                    let o = position_off + 4 * i;
                    Ok(CellPosition { vob_id: b.u16(o)?, cell_id: b.u8(o + 3)? })
                })
                .collect::<Result<_, _>>()?;
        }
        Ok(pgc)
    }

    pub fn cell(&self, cell_n: u8) -> Option<&CellPlayback> {
        self.cells.get((cell_n as usize).checked_sub(1)?)
    }
}

/// Menu type stored in the low nibble of a menu PGC's entry id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MenuId {
    /// Title menu (VMGM only).
    Title = 2,
    /// Root menu (VTSM).
    Root = 3,
    Subpicture = 4,
    Audio = 5,
    Angle = 6,
    /// Part-of-title / chapter menu.
    Part = 7,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PgciSrp {
    /// Bit 7: entry PGC. For menus the low nibble is a [`MenuId`];
    /// for titles the low 7 bits are the VTS title number.
    pub entry_id: u8,
    pub block_mode: u8,
    pub block_type: u8,
    pub ptl_id_mask: u16,
    pub pgc: Pgc,
}

/// PGC Information Table: the PGCs of one domain (or one menu language unit).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pgcit {
    pub srps: Vec<PgciSrp>,
}

impl Pgcit {
    fn parse(b: Bytes) -> Result<Self, Error> {
        let b = b.with_what("PGCIT");
        let n = b.u16(0)? as usize;
        let srps = (0..n)
            .map(|i| -> Result<_, Error> {
                let o = 8 + 8 * i;
                let blk = b.u8(o + 1)?;
                Ok(PgciSrp {
                    entry_id: b.u8(o)?,
                    block_mode: blk >> 6,
                    block_type: (blk >> 4) & 3,
                    ptl_id_mask: b.u16(o + 2)?,
                    pgc: Pgc::parse(b.from(b.u32(o + 4)? as usize)?)?,
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(Pgcit { srps })
    }

    /// 1-based PGC lookup.
    pub fn pgc(&self, pgcn: u16) -> Option<&Pgc> {
        Some(&self.srps.get((pgcn as usize).checked_sub(1)?)?.pgc)
    }

    /// PGC number of the entry PGC for `menu`.
    pub fn menu_pgcn(&self, menu: u8) -> Option<u16> {
        let id = menu | 0x80;
        self.srps.iter().position(|s| s.entry_id == id).map(|i| i as u16 + 1)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PgciLu {
    pub lang_code: u16,
    pub lang_extension: u8,
    /// Bit flags of the menus present in this language unit.
    pub exists: u8,
    pub pgcit: Pgcit,
}

/// Menu PGCI Unit Table: one PGCIT per menu language.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PgciUt {
    pub lus: Vec<PgciLu>,
}

impl PgciUt {
    fn parse(b: Bytes) -> Result<Self, Error> {
        let b = b.with_what("PGCI_UT");
        let n = b.u16(0)? as usize;
        let lus = (0..n)
            .map(|i| -> Result<_, Error> {
                let o = 8 + 8 * i;
                Ok(PgciLu {
                    lang_code: b.u16(o)?,
                    lang_extension: b.u8(o + 2)?,
                    exists: b.u8(o + 3)?,
                    pgcit: Pgcit::parse(b.from(b.u32(o + 4)? as usize)?)?,
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(PgciUt { lus })
    }

    /// The language unit for `lang`, falling back to the first unit as libdvdnav does.
    pub fn for_lang(&self, lang: u16) -> Option<&Pgcit> {
        self.lus.iter().find(|lu| lu.lang_code == lang).or(self.lus.first()).map(|lu| &lu.pgcit)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TitleInfo {
    pub playback_type: u8,
    pub nr_of_angles: u8,
    pub nr_of_ptts: u16,
    pub parental_id: u16,
    pub title_set_nr: u8,
    pub vts_ttn: u8,
    pub title_set_sector: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ptt {
    pub pgcn: u16,
    pub pgn: u16,
}

/// Video Manager Information (`VIDEO_TS.IFO`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Vmgi {
    pub vmg_category: u32,
    pub nr_of_title_sets: u16,
    pub provider_identifier: String,
    /// Start sector of the VMGM menu VOBs, relative to the IFO.
    pub vmgm_vobs: u32,
    pub first_play_pgc: Option<Pgc>,
    pub titles: Vec<TitleInfo>,
    pub menu_pgci_ut: Option<PgciUt>,
    pub menu_video_attr: VideoAttr,
    pub menu_audio_attr: Vec<AudioAttr>,
    pub menu_subp_attr: Vec<SubpAttr>,
    pub menu_vobu_admap: Vec<u32>,
}

/// Video Title Set Information (`VTS_nn_0.IFO`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Vtsi {
    pub vts_category: u32,
    pub vtsm_vobs: u32,
    pub vtstt_vobs: u32,
    /// Chapters of each VTS title.
    pub ptts: Vec<Vec<Ptt>>,
    pub pgcit: Pgcit,
    pub menu_pgci_ut: Option<PgciUt>,
    pub menu_video_attr: VideoAttr,
    pub menu_audio_attr: Vec<AudioAttr>,
    pub menu_subp_attr: Vec<SubpAttr>,
    pub video_attr: VideoAttr,
    pub audio_attr: Vec<AudioAttr>,
    pub subp_attr: Vec<SubpAttr>,
    pub menu_vobu_admap: Vec<u32>,
    pub title_vobu_admap: Vec<u32>,
}

fn sector_table(b: Bytes, sector: u32) -> Result<Option<Bytes>, Error> {
    if sector == 0 {
        return Ok(None);
    }
    b.from(sector as usize * SECTOR).map(Some)
}

fn parse_vobu_admap(t: Bytes) -> Result<Vec<u32>, Error> {
    let t = t.with_what("VOBU_ADMAP");
    let last = t.u32(0)? as usize;
    let n = last.saturating_sub(3) / 4;
    // Tolerate tables whose last_byte overshoots the file.
    let n = n.min(t.len().saturating_sub(4) / 4);
    (0..n).map(|i| t.u32(4 + 4 * i)).collect()
}

fn parse_attrs(b: Bytes, count_off: usize, attr_off: usize, max: usize) -> Result<(u8, usize), Error> {
    let n = b.u8(count_off)? as usize;
    Ok((n.min(max) as u8, attr_off))
}

fn audio_attrs(b: Bytes, count_off: usize, max: usize) -> Result<Vec<AudioAttr>, Error> {
    let (n, off) = parse_attrs(b, count_off, count_off + 1, max)?;
    (0..n as usize).map(|i| Ok(AudioAttr::parse(b.array(off + 8 * i)?))).collect()
}

fn subp_attrs(b: Bytes, count_off: usize, max: usize) -> Result<Vec<SubpAttr>, Error> {
    let (n, off) = parse_attrs(b, count_off, count_off + 1, max)?;
    (0..n as usize).map(|i| Ok(SubpAttr::parse(b.array(off + 6 * i)?))).collect()
}

impl Vmgi {
    pub fn parse(data: &[u8]) -> Result<Self, Error> {
        let b = Bytes::new(data, "VMGI_MAT");
        if b.slice(0, 12)? != b"DVDVIDEO-VMG" {
            return Err(Error::BadMagic("VIDEO_TS.IFO"));
        }
        let fp_off = b.u32(132)?;
        let tt_srpt = b.u32(196)?;
        let pgci_ut = b.u32(200)?;
        let admap = b.u32(220)?;
        let mut v = Vmgi {
            vmg_category: b.u32(34)?,
            nr_of_title_sets: b.u16(62)?,
            provider_identifier: String::from_utf8_lossy(b.slice(64, 32)?)
                .trim_end_matches(['\0', ' '])
                .to_string(),
            vmgm_vobs: b.u32(192)?,
            menu_video_attr: VideoAttr::parse(b.array(256)?),
            menu_audio_attr: audio_attrs(b, 259, 1)?,
            menu_subp_attr: subp_attrs(b, 341, 1)?,
            ..Default::default()
        };
        if fp_off != 0 {
            v.first_play_pgc = Some(Pgc::parse(b.from(fp_off as usize)?)?);
        }
        if let Some(t) = sector_table(b, tt_srpt)? {
            let t = t.with_what("TT_SRPT");
            let n = t.u16(0)? as usize;
            v.titles = (0..n)
                .map(|i| -> Result<_, Error> {
                    let o = 8 + 12 * i;
                    Ok(TitleInfo {
                        playback_type: t.u8(o)?,
                        nr_of_angles: t.u8(o + 1)?,
                        nr_of_ptts: t.u16(o + 2)?,
                        parental_id: t.u16(o + 4)?,
                        title_set_nr: t.u8(o + 6)?,
                        vts_ttn: t.u8(o + 7)?,
                        title_set_sector: t.u32(o + 8)?,
                    })
                })
                .collect::<Result<_, _>>()?;
        }
        if let Some(t) = sector_table(b, pgci_ut)? {
            v.menu_pgci_ut = Some(PgciUt::parse(t)?);
        }
        if let Some(t) = sector_table(b, admap)? {
            v.menu_vobu_admap = parse_vobu_admap(t)?;
        }
        Ok(v)
    }

    pub fn title(&self, ttn: u16) -> Option<&TitleInfo> {
        self.titles.get((ttn as usize).checked_sub(1)?)
    }

    /// Global title number of VTS title `vts_ttn` in title set `vtsn`; 0 if none.
    pub fn ttn_of(&self, vtsn: u8, vts_ttn: u8) -> u16 {
        self.titles
            .iter()
            .position(|t| t.title_set_nr == vtsn && t.vts_ttn == vts_ttn)
            .map_or(0, |i| i as u16 + 1)
    }
}

impl Vtsi {
    pub fn parse(data: &[u8]) -> Result<Self, Error> {
        let b = Bytes::new(data, "VTSI_MAT");
        if b.slice(0, 12)? != b"DVDVIDEO-VTS" {
            return Err(Error::BadMagic("VTS_nn_0.IFO"));
        }
        let ptt_srpt = b.u32(200)?;
        let pgcit = b.u32(204)?;
        let pgci_ut = b.u32(208)?;
        let menu_admap = b.u32(220)?;
        let title_admap = b.u32(228)?;
        let mut v = Vtsi {
            vts_category: b.u32(34)?,
            vtsm_vobs: b.u32(192)?,
            vtstt_vobs: b.u32(196)?,
            menu_video_attr: VideoAttr::parse(b.array(256)?),
            menu_audio_attr: audio_attrs(b, 259, 1)?,
            menu_subp_attr: subp_attrs(b, 341, 1)?,
            video_attr: VideoAttr::parse(b.array(512)?),
            audio_attr: audio_attrs(b, 515, 8)?,
            subp_attr: subp_attrs(b, 597, 32)?,
            ..Default::default()
        };
        if let Some(t) = sector_table(b, ptt_srpt)? {
            v.ptts = parse_ptt_srpt(t)?;
        }
        if let Some(t) = sector_table(b, pgcit)? {
            v.pgcit = Pgcit::parse(t)?;
        }
        if let Some(t) = sector_table(b, pgci_ut)? {
            v.menu_pgci_ut = Some(PgciUt::parse(t)?);
        }
        if let Some(t) = sector_table(b, menu_admap)? {
            v.menu_vobu_admap = parse_vobu_admap(t)?;
        }
        if let Some(t) = sector_table(b, title_admap)? {
            v.title_vobu_admap = parse_vobu_admap(t)?;
        }
        Ok(v)
    }

    pub fn ptt(&self, vts_ttn: u8, part: u16) -> Option<Ptt> {
        let title = self.ptts.get((vts_ttn as usize).checked_sub(1)?)?;
        title.get((part as usize).checked_sub(1)?).copied()
    }
}

fn parse_ptt_srpt(t: Bytes) -> Result<Vec<Vec<Ptt>>, Error> {
    let t = t.with_what("VTS_PTT_SRPT");
    let n = t.u16(0)? as usize;
    let end = (t.u32(4)? as usize + 1).min(t.len());
    let offsets: Vec<usize> = (0..n).map(|i| Ok(t.u32(8 + 4 * i)? as usize)).collect::<Result<_, Error>>()?;
    offsets
        .iter()
        .enumerate()
        .map(|(i, &start)| {
            let stop = offsets.get(i + 1).copied().unwrap_or(end).max(start);
            (0..(stop - start) / 4)
                .map(|j| Ok(Ptt { pgcn: t.u16(start + 4 * j)?, pgn: t.u16(start + 4 * j + 2)? }))
                .collect()
        })
        .collect()
}
