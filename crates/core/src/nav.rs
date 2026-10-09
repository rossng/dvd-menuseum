//! NAV pack parsing: the PCI (presentation control, incl. button highlight
//! information) and DSI (data search information) packets that start every VOBU.

use crate::bytes::Bytes;
use crate::ifo::{DvdTime, RawCmd, SECTOR};
use crate::Error;

/// Offset of the PCI payload inside a NAV pack sector (after pack header,
/// system header, PES header and the 0x00 substream id).
const PCI_START: usize = 0x2d;
/// Offset of the DSI payload (after the second PES header and the 0x01 substream id).
const DSI_START: usize = 0x407;

/// `next_vobu` value meaning "this is the last VOBU of the cell".
pub const SRI_END_OF_CELL: u32 = 0x3fff_ffff;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Button {
    /// Colour scheme 1..=3 (0 = none).
    pub coln: u8,
    pub x_start: u16,
    pub x_end: u16,
    pub y_start: u16,
    pub y_end: u16,
    /// 1 = activate as soon as selected.
    pub auto_action: u8,
    pub up: u8,
    pub down: u8,
    pub left: u8,
    pub right: u8,
    pub cmd: RawCmd,
}

impl Default for RawCmd {
    fn default() -> Self {
        RawCmd([0; 8])
    }
}

/// Highlight Information: the menu buttons valid for this VOBU.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hli {
    /// 0 no buttons, 1 new HLI, 2 same as previous, 3 same except commands.
    pub hli_ss: u8,
    pub hli_s_ptm: u32,
    pub hli_e_ptm: u32,
    pub btn_se_e_ptm: u32,
    /// Number of button groups (1..=3).
    pub btngr_ns: u8,
    /// Display type of each group's subpicture stream (bit 2 wide, 1 letterbox, 0 pan&scan).
    pub btngr_dsp_ty: [u8; 3],
    pub btn_ofn: u8,
    pub btn_ns: u8,
    pub nsl_btn_ns: u8,
    pub fosl_btnn: u8,
    pub foac_btnn: u8,
    /// `[colour scheme - 1][0 = selection, 1 = action]`; each word packs four
    /// 4-bit colour indices (high nibbles) and four 4-bit contrasts (low nibbles).
    pub btn_colit: [[u32; 2]; 3],
    /// All 36 button slots; group layout depends on `btngr_ns`.
    pub buttons: [Button; 36],
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pci {
    pub nv_pck_lbn: u32,
    pub vobu_cat: u16,
    pub vobu_uop_ctl: u32,
    pub vobu_s_ptm: u32,
    pub vobu_e_ptm: u32,
    pub vobu_se_e_ptm: u32,
    pub e_eltm: DvdTime,
    pub nsml_agl_dsta: [u32; 9],
    pub hli: Hli,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dsi {
    pub nv_pck_scr: u32,
    pub nv_pck_lbn: u32,
    /// Relative sector of the last pack in this VOBU.
    pub vobu_ea: u32,
    pub vobu_vob_idn: u16,
    pub vobu_c_idn: u8,
    /// Elapsed time in the cell at the start of this VOBU.
    pub c_eltm: DvdTime,
    pub sml_category: u16,
    pub ilvu_ea: u32,
    pub ilvu_sa: u32,
    pub sml_agl_address: [u32; 9],
    /// Relative offset to the next VOBU (top bits are flags), or [`SRI_END_OF_CELL`].
    pub next_vobu: u32,
    pub prev_vobu: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NavPack {
    pub pci: Pci,
    pub dsi: Dsi,
}

impl NavPack {
    /// Parse a 2048-byte NAV pack sector. Returns `Ok(None)` if the sector is
    /// not a NAV pack.
    pub fn parse(sector: &[u8]) -> Result<Option<NavPack>, Error> {
        if sector.len() < SECTOR || !is_nav_pack(sector) {
            return Ok(None);
        }
        Ok(Some(NavPack {
            pci: Pci::parse(Bytes::new(&sector[PCI_START..], "PCI"))?,
            dsi: Dsi::parse(Bytes::new(&sector[DSI_START..], "DSI"))?,
        }))
    }
}

/// MPEG-2 pack header, system header, then private_stream_2 with substream 0
/// (PCI) and a second private_stream_2 with substream 1 (DSI).
pub fn is_nav_pack(s: &[u8]) -> bool {
    s.len() >= SECTOR
        && s[0..4] == [0, 0, 1, 0xba]
        && s[14..18] == [0, 0, 1, 0xbb]
        && s[38..42] == [0, 0, 1, 0xbf]
        && s[44] == 0x00
        && s[0x400..0x404] == [0, 0, 1, 0xbf]
        && s[0x406] == 0x01
}

impl Pci {
    fn parse(b: Bytes) -> Result<Self, Error> {
        let mut pci = Pci {
            nv_pck_lbn: b.u32(0)?,
            vobu_cat: b.u16(4)?,
            vobu_uop_ctl: b.u32(8)?,
            vobu_s_ptm: b.u32(12)?,
            vobu_e_ptm: b.u32(16)?,
            vobu_se_e_ptm: b.u32(20)?,
            e_eltm: DvdTime { hour: b.u8(24)?, minute: b.u8(25)?, second: b.u8(26)?, frame: b.u8(27)? },
            ..Default::default()
        };
        for i in 0..9 {
            pci.nsml_agl_dsta[i] = b.u32(60 + 4 * i)?;
        }
        pci.hli = Hli::parse(b.from(96)?)?;
        Ok(pci)
    }
}

impl Default for Hli {
    fn default() -> Self {
        Hli {
            hli_ss: 0,
            hli_s_ptm: 0,
            hli_e_ptm: 0,
            btn_se_e_ptm: 0,
            btngr_ns: 0,
            btngr_dsp_ty: [0; 3],
            btn_ofn: 0,
            btn_ns: 0,
            nsl_btn_ns: 0,
            fosl_btnn: 0,
            foac_btnn: 0,
            btn_colit: [[0; 2]; 3],
            buttons: [Button::default(); 36],
        }
    }
}

impl Hli {
    fn parse(b: Bytes) -> Result<Self, Error> {
        let btn_md = b.u16(14)?;
        let mut hli = Hli {
            hli_ss: (b.u16(0)? & 3) as u8,
            hli_s_ptm: b.u32(2)?,
            hli_e_ptm: b.u32(6)?,
            btn_se_e_ptm: b.u32(10)?,
            btngr_ns: ((btn_md >> 12) & 3) as u8,
            btngr_dsp_ty: [((btn_md >> 8) & 7) as u8, ((btn_md >> 4) & 7) as u8, (btn_md & 7) as u8],
            btn_ofn: b.u8(16)?,
            btn_ns: b.u8(17)? & 0x3f,
            nsl_btn_ns: b.u8(18)? & 0x3f,
            fosl_btnn: b.u8(20)? & 0x3f,
            foac_btnn: b.u8(21)? & 0x3f,
            ..Default::default()
        };
        for scheme in 0..3 {
            for mode in 0..2 {
                hli.btn_colit[scheme][mode] = b.u32(22 + 8 * scheme + 4 * mode)?;
            }
        }
        for i in 0..36 {
            let o = 46 + 18 * i;
            let w = |k| b.u8(o + k);
            let (b0, b1, b2, b3, b4, b5) = (w(0)?, w(1)?, w(2)?, w(3)?, w(4)?, w(5)?);
            hli.buttons[i] = Button {
                coln: b0 >> 6,
                x_start: ((b0 as u16 & 0x3f) << 4) | (b1 as u16 >> 4),
                x_end: ((b1 as u16 & 0x03) << 8) | b2 as u16,
                auto_action: b3 >> 6,
                y_start: ((b3 as u16 & 0x3f) << 4) | (b4 as u16 >> 4),
                y_end: ((b4 as u16 & 0x03) << 8) | b5 as u16,
                up: w(6)? & 0x3f,
                down: w(7)? & 0x3f,
                left: w(8)? & 0x3f,
                right: w(9)? & 0x3f,
                cmd: RawCmd(b.array(o + 10)?),
            };
        }
        Ok(hli)
    }

    /// Whether this HLI defines any buttons.
    pub fn has_buttons(&self) -> bool {
        self.hli_ss != 0 && self.btn_ns > 0
    }

    /// 1-based button lookup (within the first group, as libdvdnav does).
    pub fn button(&self, n: u8) -> Option<&Button> {
        if n == 0 || n > self.btn_ns {
            return None;
        }
        self.buttons.get(n as usize - 1)
    }
}

impl Dsi {
    fn parse(b: Bytes) -> Result<Self, Error> {
        let mut dsi = Dsi {
            nv_pck_scr: b.u32(0)?,
            nv_pck_lbn: b.u32(4)?,
            vobu_ea: b.u32(8)?,
            vobu_vob_idn: b.u16(24)?,
            vobu_c_idn: b.u8(27)?,
            c_eltm: DvdTime { hour: b.u8(28)?, minute: b.u8(29)?, second: b.u8(30)?, frame: b.u8(31)? },
            sml_category: b.u16(32)?,
            ilvu_ea: b.u32(34)?,
            ilvu_sa: b.u32(38)?,
            next_vobu: b.u32(314)?,
            prev_vobu: b.u32(318)?,
            ..Default::default()
        };
        for i in 0..9 {
            dsi.sml_agl_address[i] = b.u32(180 + 6 * i)?;
        }
        Ok(dsi)
    }
}
