//! Reading DVD-Video files from a disc image (ISO 9660) or a VIDEO_TS directory.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use menuseum_core::navigator::{Disc, NavRead, NavSource, Vobs};
use menuseum_core::ifo::{Vmgi, Vtsi, SECTOR};
use menuseum_core::nav::NavPack;

/// Where a file's bytes live: an extent of the image or a standalone file.
#[derive(Clone, Debug)]
enum Location {
    Extent { lba: u32 },
    Path(PathBuf),
}

pub struct DvdFiles {
    image: Option<File>,
    /// Upper-case names in VIDEO_TS.
    files: BTreeMap<String, (Location, u64)>,
}

fn rd_u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

impl DvdFiles {
    /// Open a disc image file, a directory containing VIDEO_TS, or VIDEO_TS itself.
    pub fn open(path: &Path) -> Result<Self, String> {
        if path.is_dir() {
            let dir = if path.join("VIDEO_TS").is_dir() { path.join("VIDEO_TS") } else { path.to_path_buf() };
            let mut files = BTreeMap::new();
            for e in std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))? {
                let e = e.map_err(|e| e.to_string())?;
                let size = e.metadata().map_err(|e| e.to_string())?.len();
                let name = e.file_name().to_string_lossy().to_uppercase();
                files.insert(name, (Location::Path(e.path()), size));
            }
            return Ok(DvdFiles { image: None, files });
        }
        let mut f = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut pvd = vec![0u8; SECTOR];
        f.seek(SeekFrom::Start(16 * SECTOR as u64)).map_err(|e| e.to_string())?;
        f.read_exact(&mut pvd).map_err(|e| e.to_string())?;
        if &pvd[1..6] != b"CD001" || pvd[0] != 1 {
            return Err(format!("{}: no ISO 9660 volume descriptor (UDF-only images are not supported yet)", path.display()));
        }
        let root = &pvd[156..190];
        let mut img = DvdFiles { image: Some(f), files: BTreeMap::new() };
        let entries = img.read_dir(rd_u32le(root, 2), rd_u32le(root, 10))?;
        let (lba, size) = entries
            .iter()
            .find(|(n, _, _, dir)| *dir && n == "VIDEO_TS")
            .map(|(_, l, s, _)| (*l, *s))
            .ok_or("no VIDEO_TS directory in image")?;
        for (name, lba, size, dir) in img.read_dir(lba, size)? {
            if !dir {
                img.files.insert(name, (Location::Extent { lba }, size as u64));
            }
        }
        Ok(img)
    }

    fn read_dir(&mut self, lba: u32, size: u32) -> Result<Vec<(String, u32, u32, bool)>, String> {
        let data = self.read_image(lba as u64 * SECTOR as u64, size as usize)?;
        let mut out = vec![];
        let mut pos = 0;
        while pos < data.len() {
            let n = data[pos] as usize;
            if n == 0 {
                pos = (pos / SECTOR + 1) * SECTOR;
                continue;
            }
            let rec = &data[pos..pos + n];
            let name = &rec[33..33 + rec[32] as usize];
            if name != [0] && name != [1] {
                let name = String::from_utf8_lossy(name);
                let name = name.split(';').next().unwrap_or("").trim_end_matches('.').to_uppercase();
                out.push((name, rd_u32le(rec, 2), rd_u32le(rec, 10), rec[25] & 2 != 0));
            }
            pos += n;
        }
        Ok(out)
    }

    fn read_image(&mut self, offset: u64, len: usize) -> Result<Vec<u8>, String> {
        let f = self.image.as_mut().ok_or("not an image")?;
        let mut buf = vec![0u8; len];
        f.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
        f.read_exact(&mut buf).map_err(|e| e.to_string())?;
        Ok(buf)
    }

    pub fn size(&self, name: &str) -> Option<u64> {
        self.files.get(name).map(|(_, s)| *s)
    }

    pub fn read_range(&mut self, name: &str, offset: u64, len: usize) -> Result<Vec<u8>, String> {
        let (loc, size) = self.files.get(name).cloned().ok_or_else(|| format!("{name}: not on disc"))?;
        if offset + len as u64 > size {
            return Err(format!("{name}: read of {len} at {offset} past end ({size})"));
        }
        match loc {
            Location::Extent { lba } => self.read_image(lba as u64 * SECTOR as u64 + offset, len),
            Location::Path(p) => {
                let mut f = File::open(&p).map_err(|e| format!("{}: {e}", p.display()))?;
                let mut buf = vec![0u8; len];
                f.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
                f.read_exact(&mut buf).map_err(|e| e.to_string())?;
                Ok(buf)
            }
        }
    }

    pub fn read(&mut self, name: &str) -> Result<Vec<u8>, String> {
        let size = self.size(name).ok_or_else(|| format!("{name}: not on disc"))?;
        self.read_range(name, 0, size as usize)
    }

    /// Read an IFO, falling back to its BUP.
    fn read_ifo(&mut self, stem: &str) -> Result<Vec<u8>, String> {
        self.read(&format!("{stem}.IFO")).or_else(|_| self.read(&format!("{stem}.BUP")))
    }

    pub fn load_disc(&mut self) -> Result<Disc, String> {
        let vmgi = Vmgi::parse(&self.read_ifo("VIDEO_TS")?).map_err(|e| e.to_string())?;
        let vts = (1..=vmgi.nr_of_title_sets)
            .map(|n| {
                let stem = format!("VTS_{n:02}_0");
                let parsed = self.read_ifo(&stem).and_then(|d| Vtsi::parse(&d).map_err(|e| e.to_string()));
                match parsed {
                    Ok(v) => Some(v),
                    Err(e) => {
                        eprintln!("warning: {stem}: {e}");
                        None
                    }
                }
            })
            .collect();
        Ok(Disc { vmgi, vts })
    }

    /// The files making up a VOB set, in order.
    pub fn vob_files(&self, vobs: Vobs) -> Vec<String> {
        match vobs {
            Vobs::Vmgm => vec!["VIDEO_TS.VOB".into()],
            Vobs::Menu(n) => vec![format!("VTS_{n:02}_0.VOB")],
            Vobs::Title(n) => (1..=9).map(|i| format!("VTS_{n:02}_{i}.VOB")).filter(|f| self.files.contains_key(f)).collect(),
        }
    }

    /// Read one sector of a VOB set; `lba` is relative to the set's start.
    pub fn read_vob_sector(&mut self, vobs: Vobs, lba: u32) -> Result<Vec<u8>, String> {
        let mut offset = lba as u64 * SECTOR as u64;
        for name in self.vob_files(vobs) {
            let size = self.size(&name).unwrap_or(0);
            if offset < size {
                return self.read_range(&name, offset, SECTOR);
            }
            offset -= size;
        }
        Err(format!("sector {lba} is past the end of {vobs:?}"))
    }
}

impl NavSource for DvdFiles {
    fn nav_pack(&mut self, vobs: Vobs, lba: u32) -> Result<NavRead, String> {
        if self.vob_files(vobs).is_empty() {
            return Ok(NavRead::Stripped); // e.g. a bundle: no title VOBs
        }
        let sector = self.read_vob_sector(vobs, lba)?;
        NavPack::parse(&sector).map_err(|e| e.to_string())?.ok_or_else(|| "not a NAV pack".to_string()).map(NavRead::Pack)
    }
}

/// Name of a VOB set as used on the command line and in bundle manifests.
pub fn vobs_id(v: Vobs) -> String {
    match v {
        Vobs::Vmgm => "vmgm".into(),
        Vobs::Menu(n) => format!("menu-{n}"),
        Vobs::Title(n) => format!("title-{n}"),
    }
}

pub fn parse_vobs(s: &str) -> Option<Vobs> {
    if s == "vmgm" {
        return Some(Vobs::Vmgm);
    }
    let (kind, n) = s.split_once('-')?;
    let n: u8 = n.parse().ok()?;
    match kind {
        "menu" => Some(Vobs::Menu(n)),
        "title" => Some(Vobs::Title(n)),
        _ => None,
    }
}
