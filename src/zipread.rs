//! Lecture minimale d'archives ZIP (entrées stockées ou « deflate »), pour les formats ORA et KRA.

use anyhow::{Result, anyhow, bail};
use std::io::Read;

/// Taille maximale d'une entrée décompressée (protège d'une archive piégée).
const MAX_ENTRY: u64 = 1 << 30;

struct Entry {
    name: String,
    method: u16,
    csize: usize,
    usize_: usize,
    header_offset: usize,
}

pub struct Zip<'a> {
    data: &'a [u8],
    entries: Vec<Entry>,
}

fn u16le(d: &[u8], o: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(d.get(o..o + 2).ok_or_else(|| anyhow!("zip tronqué"))?.try_into().unwrap()))
}
fn u32le(d: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(d.get(o..o + 4).ok_or_else(|| anyhow!("zip tronqué"))?.try_into().unwrap()))
}

impl<'a> Zip<'a> {
    pub fn is_zip(data: &[u8]) -> bool {
        data.len() > 4 && &data[..2] == b"PK" && (data[2] == 3 || data[2] == 5)
    }

    pub fn open(data: &'a [u8]) -> Result<Zip<'a>> {
        // Fin de répertoire central : on cherche la signature en partant de la fin.
        let min = data.len().saturating_sub(22 + 65535);
        let eocd = (min..=data.len().saturating_sub(22)).rev().find(|&i| data.get(i..i + 4) == Some(&[0x50, 0x4B, 0x05, 0x06])).ok_or_else(|| anyhow!("archive ZIP invalide"))?;
        let count = u16le(data, eocd + 10)? as usize;
        let mut p = u32le(data, eocd + 16)? as usize;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            if data.get(p..p + 4) != Some(&[0x50, 0x4B, 0x01, 0x02]) {
                bail!("répertoire central ZIP invalide");
            }
            let method = u16le(data, p + 10)?;
            let (csize, usize_) = (u32le(data, p + 20)?, u32le(data, p + 24)?);
            let (nl, el, cl) = (u16le(data, p + 28)? as usize, u16le(data, p + 30)? as usize, u16le(data, p + 32)? as usize);
            let header_offset = u32le(data, p + 42)? as usize;
            if csize == u32::MAX || usize_ == u32::MAX || header_offset == u32::MAX as usize {
                bail!("ZIP64 non pris en charge");
            }
            let name = String::from_utf8_lossy(data.get(p + 46..p + 46 + nl).ok_or_else(|| anyhow!("zip tronqué"))?).to_string();
            entries.push(Entry { name, method, csize: csize as usize, usize_: usize_ as usize, header_offset });
            p += 46 + nl + el + cl;
        }
        Ok(Zip { data, entries })
    }

    pub fn names(&self) -> Vec<&str> {
        self.entries.iter().map(|e| e.name.as_str()).collect()
    }

    pub fn has(&self, name: &str) -> bool {
        self.entries.iter().any(|e| e.name == name)
    }

    pub fn read(&self, name: &str) -> Result<Vec<u8>> {
        let e = self.entries.iter().find(|e| e.name == name).ok_or_else(|| anyhow!("{name} : absent de l'archive"))?;
        let h = e.header_offset;
        if self.data.get(h..h + 4) != Some(&[0x50, 0x4B, 0x03, 0x04]) {
            bail!("en-tête local ZIP invalide");
        }
        let start = h + 30 + u16le(self.data, h + 26)? as usize + u16le(self.data, h + 28)? as usize;
        let raw = self.data.get(start..start + e.csize).ok_or_else(|| anyhow!("zip tronqué"))?;
        match e.method {
            0 => Ok(raw.to_vec()),
            8 => {
                // La taille annoncée n'est pas fiable : capacité prudente et décompression bornée (archive piégée).
                let mut out = Vec::with_capacity(e.usize_.min(64 << 20));
                flate2::read::DeflateDecoder::new(raw).take(MAX_ENTRY).read_to_end(&mut out).map_err(|er| anyhow!("{name} : {er}"))?;
                Ok(out)
            }
            m => bail!("méthode de compression ZIP non prise en charge ({m})"),
        }
    }
}
