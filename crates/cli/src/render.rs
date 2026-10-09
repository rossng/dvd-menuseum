//! Offline rendering of the current menu state to an image: the VOBU's first
//! video frame, the subpicture and the button highlight.

use menuseum_core::demux::{packets, SpuAssembler};
use menuseum_engine::Engine;
use menuseum_core::spu::{button_highlight, composite, HighlightMode, Spu};
use menuseum_media::video::VideoDecoder;

use crate::source::DvdFiles;

pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
}

/// Sectors decoded before the cell, so a cell authored to follow its
/// predecessor on disc has its reference pictures.
const PREROLL: u32 = 512;

/// The first picture, in display order, of the current VOBU.
fn video_frame(files: &mut DvdFiles, engine: &Engine) -> Result<(usize, usize, Vec<u8>), String> {
    let cell = engine.pgc().and_then(|p| p.cell(engine.vm.celln)).ok_or("no current cell")?;
    let lbn = engine.pci().nv_pck_lbn;
    let end = lbn + engine.dsi().vobu_ea;
    let mut dec = VideoDecoder::default();
    let mut pictures = vec![];
    for s in cell.first_sector.saturating_sub(PREROLL)..=end {
        let sector = files.read_vob_sector(engine.vobs(), s)?;
        let tag = u64::from(s >= lbn);
        for p in packets(&sector) {
            if p.stream_id == 0xe0 {
                pictures.extend(dec.push(p.payload, tag));
            }
        }
    }
    pictures.extend(dec.flush());
    let picture = pictures
        .iter()
        .find(|(_, tag)| *tag == 1)
        .or(pictures.last())
        .map(|(p, _)| p)
        .ok_or("no picture decoded")?;
    let rgb = picture.to_rgba().chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    Ok((picture.width, picture.height, rgb))
}

/// The most recent subpicture unit of `stream` from the start of the current
/// cell up to the end of the current VOBU.
fn current_spu(files: &mut DvdFiles, engine: &Engine, stream: u8) -> Option<Spu> {
    let cell = engine.pgc()?.cell(engine.vm.celln)?;
    let end = engine.pci().nv_pck_lbn + engine.dsi().vobu_ea;
    let mut asm = SpuAssembler::default();
    let mut last = None;
    for s in cell.first_sector..=end.min(cell.last_sector) {
        let Ok(sector) = files.read_vob_sector(engine.vobs(), s) else { break };
        for p in packets(&sector) {
            if p.stream_id == 0xbd && p.substream == Some(0x20 | (stream & 0x1f)) {
                if let Some((unit, _pts)) = asm.push(p.payload, p.pts) {
                    match Spu::parse(&unit) {
                        Ok(spu) => last = Some(spu),
                        Err(e) => eprintln!("warning: bad SPU at sector {s}: {}", e.0),
                    }
                }
            }
        }
    }
    last
}

pub fn render(files: &mut DvdFiles, engine: &mut Engine) -> Result<Frame, String> {
    let (mut width, mut height, mut rgb) = match video_frame(files, engine) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("warning: {e}");
            (0, 0, vec![])
        }
    };
    if rgb.is_empty() || rgb.len() != width * height * 3 {
        let (w, h) = engine.video_attr().resolution();
        (width, height, rgb) = (w as usize, h as usize, vec![0; w as usize * h as usize * 3]);
    }
    // Widescreen menus are rendered as they would be on a 16:9 display.
    if let Some(stream) = engine.subp_active_stream(0) {
        if let Some(spu) = current_spu(files, engine, stream) {
            let hli = &engine.pci().hli;
            let highlight = if hli.hli_ss != 0 {
                button_highlight(hli, engine.vm.regs.button(), HighlightMode::Selected)
            } else {
                None
            };
            let palette = engine.pgc().map(|p| p.palette).unwrap_or([0; 16]);
            composite(&mut rgb, width, height, &spu, &palette, highlight.as_ref());
        }
    }
    Ok(Frame { width, height, rgb })
}
