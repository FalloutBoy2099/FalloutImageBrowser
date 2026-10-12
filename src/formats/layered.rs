//! OpenRaster (.ora) et Krita (.kra) : archives ZIP, lecture de l'image fusionnée (sans calques).

use super::*;
use crate::zipread::Zip;

/// Décode un PNG en image RGBA.
fn png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png).map_err(|e| anyhow!("PNG : {e}"))?;
    let rgba = img.to_rgba8();
    check_dims(rgba.width() as u64, rgba.height() as u64)?;
    Ok((rgba.width(), rgba.height(), rgba.into_raw()))
}

/// Valeur d'un attribut XML : `attr(tag, "src")`.
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("{name}=\"");
    let mut from = 0;
    while let Some(i) = tag[from..].find(&key) {
        let at = from + i;
        // L'attribut doit commencer après un espace (évite « x » dans « max »).
        if at == 0 || tag.as_bytes()[at - 1].is_ascii_whitespace() {
            let s = at + key.len();
            let e = tag[s..].find('"')?;
            return Some(&tag[s..s + e]);
        }
        from = at + key.len();
    }
    None
}

fn tags<'a>(xml: &'a str, name: &str) -> Vec<&'a str> {
    let open = format!("<{name}");
    let mut v = Vec::new();
    let mut from = 0;
    while let Some(i) = xml[from..].find(&open) {
        let s = from + i;
        let after = xml.as_bytes().get(s + open.len()).copied().unwrap_or(b' ');
        let e = xml[s..].find('>').map(|e| s + e).unwrap_or(xml.len());
        if after.is_ascii_whitespace() || after == b'>' || after == b'/' {
            v.push(&xml[s..e]);
        }
        from = e.min(xml.len());
        if from >= xml.len() {
            break;
        }
    }
    v
}

pub fn decode_ora(data: &[u8]) -> Result<Picture> {
    let zip = Zip::open(data)?;
    let stack = zip.read("stack.xml").ok().map(|b| String::from_utf8_lossy(&b).to_string()).unwrap_or_default();
    let layers = tags(&stack, "layer");
    let (w, h, rgba, source) = if let Ok(b) = zip.read("mergedimage.png") {
        let (w, h, v) = png(&b)?;
        (w, h, v, "mergedimage.png")
    } else if let Some((w, h, v)) = compose_ora(&zip, &stack, &layers) {
        (w, h, v, "stack.xml")
    } else if let Ok(b) = zip.read("Thumbnails/thumbnail.png") {
        let (w, h, v) = png(&b)?;
        (w, h, v, "Thumbnails/thumbnail.png")
    } else {
        bail!("ORA : aucune image fusionnée, calque ou miniature exploitable");
    };
    let mut pic = Picture::single(Format::Ora, w, h, Pixels::Rgba(rgba), None);
    pic.bpp = 32;
    let s = pic.section("sec.format");
    s.raw("Source", source);
    s.add("p.layers", layers.len());
    if let Ok(m) = zip.read("mimetype") {
        s.raw("Mimetype", String::from_utf8_lossy(&m).trim().to_string());
    }
    if let Some(img) = tags(&stack, "image").first() {
        if let Some(v) = attr(img, "version") {
            s.add("p.version", v);
        }
        if let (Some(xr), Some(yr)) = (attr(img, "xres"), attr(img, "yres")) {
            s.add("p.dpi", format!("{xr} × {yr}"));
        }
    }
    Ok(pic)
}

/// Reconstitue l'image en superposant les calques visibles (mode Normal) quand il n'y a pas d'image fusionnée.
fn compose_ora(zip: &Zip, stack: &str, layers: &[&str]) -> Option<(u32, u32, Vec<u8>)> {
    let img = *tags(stack, "image").first()?;
    let (w, h): (usize, usize) = (attr(img, "w")?.parse().ok()?, attr(img, "h")?.parse().ok()?);
    check_dims(w as u64, h as u64).ok()?;
    let mut canvas = vec![0u8; w * h * 4];
    // Le premier calque de la pile est le plus haut : on dessine du dernier au premier.
    for l in layers.iter().rev() {
        if attr(l, "visibility") == Some("hidden") {
            continue;
        }
        let Some(src) = attr(l, "src") else { continue };
        let Ok(bytes) = zip.read(src) else { continue };
        let Ok((lw, lh, px)) = png(&bytes) else { continue };
        let (ox, oy): (i64, i64) = (attr(l, "x").and_then(|v| v.parse().ok()).unwrap_or(0), attr(l, "y").and_then(|v| v.parse().ok()).unwrap_or(0));
        let opacity: f32 = attr(l, "opacity").and_then(|v| v.parse().ok()).unwrap_or(1.0);
        blend_over(&mut canvas, w, h, &px, lw as usize, lh as usize, ox, oy, opacity);
    }
    Some((w as u32, h as u32, canvas))
}

/// Superposition « Normal » d'une image RGBA (alpha non prémultiplié) sur le canevas.
#[allow(clippy::too_many_arguments)]
pub fn blend_over(canvas: &mut [u8], cw: usize, ch: usize, src: &[u8], sw: usize, sh: usize, ox: i64, oy: i64, opacity: f32) {
    for y in 0..sh {
        let dy = y as i64 + oy;
        if dy < 0 || dy >= ch as i64 {
            continue;
        }
        for x in 0..sw {
            let dx = x as i64 + ox;
            if dx < 0 || dx >= cw as i64 {
                continue;
            }
            let s = &src[(y * sw + x) * 4..(y * sw + x) * 4 + 4];
            let sa = s[3] as f32 / 255.0 * opacity;
            if sa <= 0.0 {
                continue;
            }
            let d = &mut canvas[(dy as usize * cw + dx as usize) * 4..(dy as usize * cw + dx as usize) * 4 + 4];
            let da = d[3] as f32 / 255.0;
            let oa = sa + da * (1.0 - sa);
            for c in 0..3 {
                d[c] = ((s[c] as f32 * sa + d[c] as f32 * da * (1.0 - sa)) / oa + 0.5) as u8;
            }
            d[3] = (oa * 255.0 + 0.5) as u8;
        }
    }
}

pub fn decode_kra(data: &[u8]) -> Result<Picture> {
    let zip = Zip::open(data)?;
    let (source, bytes) = if let Ok(b) = zip.read("mergedimage.png") {
        ("mergedimage.png", b)
    } else if let Ok(b) = zip.read("preview.png") {
        ("preview.png (miniature)", b)
    } else {
        bail!("KRA : ni mergedimage.png ni preview.png dans l'archive");
    };
    let (w, h, rgba) = png(&bytes)?;
    let mut pic = Picture::single(Format::Kra, w, h, Pixels::Rgba(rgba), None);
    pic.bpp = 32;
    let doc = zip.read("maindoc.xml").ok().map(|b| String::from_utf8_lossy(&b).to_string()).unwrap_or_default();
    let s = pic.section("sec.format");
    s.raw("Source", source);
    if let Some(img) = tags(&doc, "IMAGE").first() {
        if let Some(v) = attr(img, "name") {
            s.add("p.name", v);
        }
        if let (Some(a), Some(b)) = (attr(img, "width"), attr(img, "height")) {
            s.add("p.dimensions", format!("{a} × {b}"));
        }
        if let Some(cs) = attr(img, "colorspacename") {
            s.raw("Color space", cs);
        }
        if let (Some(x), Some(y)) = (attr(img, "x-res"), attr(img, "y-res")) {
            s.add(
                "p.dpi",
                format!("{} × {}", x.parse::<f64>().map(|v| format!("{v:.0}")).unwrap_or(x.to_string()), y.parse::<f64>().map(|v| format!("{v:.0}")).unwrap_or(y.to_string())),
            );
        }
    }
    s.add("p.layers", tags(&doc, "layer").len());
    if let Ok(m) = zip.read("mimetype") {
        s.raw("Mimetype", String::from_utf8_lossy(&m).trim().to_string());
    }
    Ok(pic)
}
