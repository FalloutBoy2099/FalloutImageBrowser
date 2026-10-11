//! RIX (écrans Fallout / ColoRIX) : palette 6 bits + pixels 8 bits linéaires.
//! https://fallout.wiki/wiki/RIX_File_Format

use super::*;

pub fn decode(data: &[u8]) -> Result<Picture> {
    let mut r = Rd::new(data);
    if r.bytes(4)? != b"RIX3" {
        bail!("signature RIX3 absente");
    }
    let w = r.u16le()? as usize;
    let h = r.u16le()? as usize;
    let ptype = r.u8()?;
    let stype = r.u8()?;
    check_dims(w as u64, h as u64)?;
    if stype & 0x0F != 0 {
        bail!("RIX compressé non pris en charge (type de stockage {stype:#04X})");
    }
    let pal = r.bytes(768)?;
    let six = pal.iter().all(|&v| v <= 63);
    let mut palette = Palette::empty();
    for i in 0..256 {
        for k in 0..3 {
            palette.colors[i][k] = if six { pal[i * 3 + k] << 2 } else { pal[i * 3 + k] };
        }
        palette.colors[i][3] = 255;
        palette.used[i] = true;
    }
    let px = r.bytes(w * h)?.to_vec();
    let mut pic = Picture::single(Format::Rix, w as u32, h as u32, Pixels::Indexed(px), Some(palette));
    pic.bpp = 8;
    let s = pic.section("sec.format");
    s.raw("Signature", "RIX3");
    s.raw("Palette type", format!("{ptype:#04X}"));
    s.raw("Storage type", format!("{stype:#04X}"));
    s.add("p.compression", "p.none");
    Ok(pic)
}

pub fn encode(p: &Prepared) -> Result<Vec<u8>> {
    let f = p.first();
    let (OutData::Indexed(idx), Some(pal)) = (&f.data, &p.palette) else {
        bail!("RIX : image indexée requise");
    };
    if f.width > 65535 || f.height > 65535 {
        bail!("image trop grande pour le format RIX");
    }
    let mut o: Vec<u8> = Vec::with_capacity(778 + idx.len());
    o.extend_from_slice(b"RIX3");
    put_u16le(&mut o, f.width as u16);
    put_u16le(&mut o, f.height as u16);
    o.push(0xAF);
    o.push(0);
    for i in 0..256 {
        let c = if pal.used[i] { pal.colors[i] } else { [0, 0, 0, 255] };
        o.extend_from_slice(&[c[0] >> 2, c[1] >> 2, c[2] >> 2]);
    }
    o.extend_from_slice(idx);
    Ok(o)
}
