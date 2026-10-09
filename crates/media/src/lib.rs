//! Media for the DVD Menuseum: decoding the VOBUs the navigation engine
//! selects (MPEG-2 video, AC-3 / DTS / LPCM / MPEG audio, subpictures) and
//! the real-time [`player::Player`] that ties engine and decoders together.

pub mod audio;
pub mod player;
pub mod video;
