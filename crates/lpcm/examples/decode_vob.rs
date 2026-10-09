//! Decode the first LPCM track of a VOB file to interleaved s32le.
//!
//! usage: decode_vob <in.vob> <out.raw>
use menuseum_core::demux::packets;
use oxideav_core::{Decoder, Frame, Packet, TimeBase};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let vob = std::fs::read(&a[1]).unwrap();
    let mut dec = menuseum_lpcm::LpcmDecoder::new();
    let mut out = vec![];
    for sector in vob.chunks(2048) {
        for p in packets(sector) {
            if p.stream_id == 0xbd && p.substream == Some(0xa0) && p.payload.len() > 3 {
                let pkt = Packet::new(0, TimeBase::new(1, 90_000), p.payload[3..].to_vec());
                dec.send_packet(&pkt).unwrap();
                while let Ok(Frame::Audio(f)) = dec.receive_frame() {
                    out.extend_from_slice(&f.data[0]);
                }
            }
        }
    }
    eprintln!("{:?}, {} sample frames", dec.header(), out.len() / 4 / dec.header().map_or(1, |h| h.channels as usize));
    std::fs::write(&a[2], out).unwrap();
}
