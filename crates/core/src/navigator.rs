//! The interface between a navigation engine and its host: the disc, the
//! events a navigator produces, the inputs it accepts and [`Navigator`] itself.
//!
//! The engine that implements it lives in its own crate (`menuseum-engine`),
//! so that the player and decoders can be built against this interface alone.

use crate::ifo::{MenuId, Pgc, VideoAttr, Vmgi, Vtsi};
use crate::nav::{Dsi, NavPack, Pci};
use crate::vm::Registers;

/// Numbering matches libdvdnav's `DVDDomain_t`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Domain {
    FirstPlay = 1,
    Title = 2,
    Vmgm = 4,
    Vtsm = 8,
}

/// A parsed disc: the VMGI and every VTSI.
#[derive(Clone, Debug, Default)]
pub struct Disc {
    pub vmgi: Vmgi,
    /// Index `n - 1` holds VTS `n`; `None` if its IFO was missing or unreadable.
    pub vts: Vec<Option<Vtsi>>,
}

impl Disc {
    pub fn vts(&self, vtsn: u8) -> Option<&Vtsi> {
        self.vts.get((vtsn as usize).checked_sub(1)?)?.as_ref()
    }
}

/// Which VOB set a sector address refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Vobs {
    /// `VIDEO_TS.VOB` (first play and VMGM).
    Vmgm,
    /// `VTS_nn_0.VOB`.
    Menu(u8),
    /// `VTS_nn_1.VOB` .. `VTS_nn_9.VOB`, concatenated.
    Title(u8),
}

/// Result of asking a [`NavSource`] for a NAV pack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavRead {
    Pack(NavPack),
    /// The VOB set's data was stripped (e.g. a title in a menu bundle): the
    /// engine treats the cell as having ended.
    Stripped,
    /// Not available yet (still being fetched): the engine returns
    /// [`Event::Pending`] without changing state, so the call can be retried.
    Pending,
}

/// Supplies NAV packs; `lba` is relative to the start of the VOB set.
pub trait NavSource {
    fn nav_pack(&mut self, vobs: Vobs, lba: u32) -> Result<NavRead, String>;
}

/// Events returned by [`Navigator::next_event`], matching libdvdnav's (minus `BLOCK_OK`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Nop,
    Stop,
    HopChannel,
    Highlight { button: u8 },
    /// The host should finish presenting what it has, then call [`Navigator::wait_skip`].
    Wait,
    VtsChange { old_domain: Option<Domain>, old_vts: u8, new_domain: Domain, new_vts: u8 },
    CellChange { celln: u8, pgn: u8 },
    SpuClutChange,
    SpuStreamChange,
    AudioStreamChange,
    /// Still frame; seconds, 0xff = infinite. Repeats until [`Navigator::still_skip`].
    Still(u8),
    /// A NAV pack was read; see [`Navigator::pci`] / [`Navigator::dsi`].
    Nav { lbn: u32 },
    /// The NAV pack at `lba` is not available yet; call again once it is.
    Pending { vobs: Vobs, lba: u32 },
    Error(String),
}

/// Menus the user (not the disc) can ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuCall {
    Escape,
    Title,
    Root,
    Subpicture,
    Audio,
    Angle,
    Part,
}

impl MenuCall {
    pub fn id(self) -> u8 {
        match self {
            MenuCall::Escape | MenuCall::Root => MenuId::Root as u8,
            MenuCall::Title => MenuId::Title as u8,
            MenuCall::Subpicture => MenuId::Subpicture as u8,
            MenuCall::Audio => MenuId::Audio as u8,
            MenuCall::Angle => MenuId::Angle as u8,
            MenuCall::Part => MenuId::Part as u8,
        }
    }
}

/// Where playback is: the parts of the VM state a host needs to follow it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub domain: Domain,
    /// 0 = no VTS opened yet.
    pub vtsn: u8,
    pub pgn: u8,
    pub celln: u8,
    pub cell_restart: u32,
    pub hop_channel: u32,
    pub stopped: bool,
}

/// A DVD navigator, driven one event at a time by the host.
pub trait Navigator {
    fn disc(&self) -> &Disc;
    fn status(&self) -> Status;
    fn regs(&self) -> &Registers;
    /// Set the clock for counter-mode GPRMs, in milliseconds.
    fn set_now_ms(&mut self, now_ms: u64);
    /// Advance playback by one step.
    fn next_event(&mut self, src: &mut dyn NavSource) -> Event;
    /// PCI of the last NAV pack read.
    fn pci(&self) -> &Pci;
    /// DSI of the last NAV pack read.
    fn dsi(&self) -> &Dsi;
    /// The PGC currently playing.
    fn pgc(&self) -> Option<&Pgc>;
    /// Current PGC number in the current domain (0 for the first-play PGC).
    fn pgcn(&mut self) -> u16;
    /// The VOB set the current domain plays from.
    fn vobs(&self) -> Vobs;
    /// Video attributes of the current domain.
    fn video_attr(&self) -> VideoAttr;
    /// Physical audio stream to play.
    fn audio_active_stream(&self) -> Option<u8>;
    /// Subpicture stream to decode for `mode` (0 widescreen, 1 letterbox,
    /// 2 pan&scan); bit 7 set means only forced subpictures should be shown.
    fn subp_active_stream(&mut self, mode: u8) -> Option<u8>;
    fn still_skip(&mut self);
    fn wait_skip(&mut self);
    fn button_select(&mut self, n: u8) -> bool;
    fn button_activate(&mut self) -> bool;
    fn button_up(&mut self) -> bool;
    fn button_down(&mut self) -> bool;
    fn button_left(&mut self) -> bool;
    fn button_right(&mut self) -> bool;
    fn mouse_select(&mut self, x: u16, y: u16) -> bool;
    fn menu_call(&mut self, menu: MenuCall) -> bool;
}
