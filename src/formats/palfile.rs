//! Fichiers de palette affichés en nuancier : PAL (Fallout, RIFF Microsoft, JASC), ACT, GPL, KPL, HEX. Lecture seule.

use super::*;
use crate::zipread::Zip;

const CELL: usize = 24;
const COLUMNS: usize = 16;

fn from_colors(colors: &[[u8; 3]]) -> Palette {
    let mut p = Palette::empty();
    for (i, c) in colors.iter().take(256).enumerate() {
        p.colors[i] = [c[0], c[1], c[2], 255];
        p.used[i] = true;
    }
    p
}

fn text(data: &[u8]) -> String {
    String::from_utf8_lossy(data).trim_start_matches('\u{feff}').to_string()
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let s = tag.find(&key)? + key.len();
    let e = tag[s..].find('"')?;
    Some(&tag[s..s + e])
}

/// (palette, nom du type, nom de la palette)
fn parse(data: &[u8], ext: &str) -> Result<(Palette, &'static str, Option<String>)> {
    // Archive Krita.
    if Zip::is_zip(data) {
        let z = Zip::open(data)?;
        let xml = text(&z.read("colorset.xml")?);
        let name = xml.find("<Colorset").and_then(|i| attr(&xml[i..xml[i..].find('>').map(|e| i + e)?], "name")).map(|s| s.to_string());
        let mut colors = Vec::new();
        let mut from = 0;
        while let Some(i) = xml[from..].find("<RGB").or_else(|| xml[from..].find("<sRGB")) {
            let s = from + i;
            let e = xml[s..].find('>').map(|e| s + e).unwrap_or(xml.len());
            let tag = &xml[s..e];
            let f = |n: &str| attr(tag, n).and_then(|v| v.parse::<f32>().ok());
            if let (Some(r), Some(g), Some(b)) = (f("r"), f("g"), f("b")) {
                let k = if r > 1.0 || g > 1.0 || b > 1.0 { 1.0 } else { 255.0 };
                colors.push([(r * k + 0.5) as u8, (g * k + 0.5) as u8, (b * k + 0.5) as u8]);
            }
            from = e;
        }
        if colors.is_empty() {
            bail!("KPL : aucune couleur RVB dans colorset.xml");
        }
        return Ok((from_colors(&colors), "Krita Palette (KPL)", name));
    }
    // RIFF PAL.
    if data.len() >= 24 && &data[..4] == b"RIFF" && &data[8..12] == b"PAL " {
        let n = u16::from_le_bytes([data[22], data[23]]) as usize;
        let body = data.get(24..24 + n * 4).ok_or_else(|| anyhow!("PAL RIFF tronqué"))?;
        let colors: Vec<[u8; 3]> = body.chunks_exact(4).map(|c| [c[0], c[1], c[2]]).collect();
        return Ok((from_colors(&colors), "Microsoft RIFF Palette (PAL)", None));
    }
    let head = text(&data[..data.len().min(64)]);
    // JASC-PAL, GPL.
    if head.starts_with("JASC-PAL") || head.starts_with("GIMP Palette") {
        let t = text(data);
        let mut name = None;
        let mut colors = Vec::new();
        let jasc = head.starts_with("JASC-PAL");
        for (i, line) in t.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(n) = line.strip_prefix("Name:") {
                name = Some(n.trim().to_string());
                continue;
            }
            if line.starts_with("Columns:") || (jasc && i < 3) || line.starts_with("GIMP Palette") {
                continue;
            }
            let nums: Vec<u32> = line.split_whitespace().take(3).filter_map(|w| w.parse().ok()).collect();
            if nums.len() == 3 {
                colors.push([nums[0].min(255) as u8, nums[1].min(255) as u8, nums[2].min(255) as u8]);
            }
        }
        if colors.is_empty() {
            bail!("palette texte sans couleur");
        }
        return Ok((from_colors(&colors), if jasc { "JASC Palette (PAL)" } else { "GIMP Palette (GPL)" }, name));
    }
    // HEX : une couleur « rrggbb » par ligne.
    if ext == "hex" || (ext != "act" && ext != "pal" && data.len() < 1 << 20) {
        let t = text(data);
        let colors: Vec<[u8; 3]> = t
            .lines()
            .map(|l| l.trim().trim_start_matches('#'))
            .filter(|l| !l.is_empty())
            .filter_map(|l| {
                if l.len() == 6 && l.chars().all(|c| c.is_ascii_hexdigit()) {
                    let v = u32::from_str_radix(l, 16).ok()?;
                    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
                } else {
                    None
                }
            })
            .collect();
        let lines = t.lines().filter(|l| !l.trim().is_empty()).count();
        if !colors.is_empty() && colors.len() == lines {
            return Ok((from_colors(&colors), "Hex Color Palette (HEX)", None));
        }
    }
    // ACT (772 octets avec en-queue) ou 768 octets : Fallout PAL (6 bits) ou ACT selon l'extension.
    if data.len() == 772 || (ext == "act" && data.len() >= 768) {
        let mut p = Palette::empty();
        let (count, transp) = if data.len() >= 772 {
            (u16::from_be_bytes([data[768], data[769]]) as usize, u16::from_be_bytes([data[770], data[771]]))
        } else {
            (256, 0xFFFF)
        };
        for i in 0..count.clamp(1, 256) {
            p.colors[i] = [data[i * 3], data[i * 3 + 1], data[i * 3 + 2], 255];
            p.used[i] = true;
        }
        if (transp as usize) < 256 {
            p.colors[transp as usize][3] = 0;
        }
        return Ok((p, "Adobe Color Table (ACT)", None));
    }
    if data.len() >= 768 {
        let p = crate::palette::parse_pal(data)?;
        return Ok((p, "Fallout Palette (PAL, 6-bit)", None));
    }
    bail!("fichier de palette non reconnu")
}

pub fn decode(data: &[u8], ext: &str) -> Result<Picture> {
    let (pal, kind, name) = parse(data, ext)?;
    let last = (0..256).rev().find(|&i| pal.used[i]).unwrap_or(0);
    let rows = (last + 1).div_ceil(COLUMNS);
    let (w, h) = (COLUMNS * CELL, rows * CELL);
    let mut rgba = vec![0u8; w * h * 4];
    for i in 0..=last {
        if !pal.used[i] {
            continue;
        }
        let (cx, cy) = ((i % COLUMNS) * CELL, (i / COLUMNS) * CELL);
        for y in 0..CELL {
            for x in 0..CELL {
                let o = ((cy + y) * w + cx + x) * 4;
                // Fine séparation entre les cases.
                let edge = x == CELL - 1 || y == CELL - 1;
                let c = pal.colors[i];
                rgba[o..o + 4].copy_from_slice(&if edge { [c[0] / 2, c[1] / 2, c[2] / 2, 255] } else { [c[0], c[1], c[2], 255] });
            }
        }
    }
    let mut pic = Picture::single(Format::Palette, w as u32, h as u32, Pixels::Rgba(rgba), None);
    pic.bpp = 24;
    let s = pic.section("sec.format");
    s.raw("Type", kind);
    s.add("p.palette_entries", pal.count_used());
    if let Some(n) = name {
        s.add("p.name", n);
    }
    pic.colors = Some(pal);
    Ok(pic)
}
