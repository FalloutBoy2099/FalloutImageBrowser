//! Informations d'une image (panneau « Propriétés ») mises en forme pour le presse-papiers : texte, JSON et XML.

use crate::i18n::tr;
use crate::model::*;

pub type PropSections = Vec<(String, Vec<(String, String)>)>;

/// Toutes les propriétés affichées dans le panneau (sections, image courante, palette), textes traduits.
pub fn collect_props(pic: &Picture, dir: usize, frame: usize) -> PropSections {
    let mut out: PropSections = Vec::new();
    for sec in &pic.sections {
        let rows = sec.items.iter().map(|p| (tr(&p.key), if p.value.starts_with("p.") || p.value.starts_with("c.") { tr(&p.value) } else { p.value.clone() })).collect();
        out.push((tr(&sec.title), rows));
    }
    if pic.frames.len() > 1 {
        let f = &pic.frames[pic.frame_index(dir, frame)];
        let mut rows = Vec::new();
        if pic.directions > 1 {
            rows.push((tr("p.direction"), format!("{} / {}", dir + 1, pic.directions)));
        }
        rows.push((tr("a.frame"), format!("{} / {}", frame + 1, pic.dir_len(dir))));
        rows.push((tr("p.dimensions"), format!("{} × {}", f.width, f.height)));
        rows.push((tr("p.offset"), format!("{}, {}", f.pos.0, f.pos.1)));
        if let Some(d) = f.delay_ms {
            rows.push((tr("p.delay"), format!("{d} ms")));
        }
        out.push((tr("sec.current"), rows));
    }
    if let Some(pal) = pic.palette.as_ref().or(pic.colors.as_ref()) {
        let rows = (0..256).filter(|&i| pal.used[i]).map(|i| (i.to_string(), format!("#{:02X}{:02X}{:02X}", pal.colors[i][0], pal.colors[i][1], pal.colors[i][2]))).collect();
        out.push((tr("sec.palette"), rows));
    }
    out
}

pub fn props_text(sections: &PropSections) -> String {
    let mut s = String::new();
    for (title, rows) in sections {
        if !s.is_empty() {
            s.push('\n');
        }
        s += &format!("[{title}]\n");
        for (k, v) in rows {
            s += &format!("{k}: {v}\n");
        }
    }
    s
}

fn json_str(v: &str) -> String {
    let mut o = String::with_capacity(v.len() + 2);
    o.push('"');
    for ch in v.chars() {
        match ch {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// Clé unique dans un objet JSON : un doublon reçoit le suffixe « (2) », « (3) »…
fn unique_key(used: &mut Vec<String>, k: &str) -> String {
    let mut name = k.to_string();
    let mut n = 2;
    while used.contains(&name) {
        name = format!("{k} ({n})");
        n += 1;
    }
    used.push(name.clone());
    name
}

pub fn props_json(sections: &PropSections) -> String {
    let mut titles: Vec<String> = Vec::new();
    let mut o = String::from("{\n");
    for (si, (title, rows)) in sections.iter().enumerate() {
        o += &format!("  {}: {{\n", json_str(&unique_key(&mut titles, title)));
        let mut keys: Vec<String> = Vec::new();
        for (ri, (k, v)) in rows.iter().enumerate() {
            o += &format!("    {}: {}{}\n", json_str(&unique_key(&mut keys, k)), json_str(v), if ri + 1 < rows.len() { "," } else { "" });
        }
        o += if si + 1 < sections.len() { "  },\n" } else { "  }\n" };
    }
    o.push('}');
    o
}

/// Texte pour un contenu XML : `&`, `<`, `>`, guillemets et apostrophes protégés, caractères de contrôle interdits supprimés.
fn xml_escape(v: &str) -> String {
    let mut o = String::with_capacity(v.len());
    for ch in v.chars() {
        match ch {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            '\t' | '\n' | '\r' => o.push(ch),
            c if (c as u32) < 0x20 => {}
            c => o.push(c),
        }
    }
    o
}

/// XML : `<properties>` contenant une `<section name="…">` par section, et une `<property name="…">valeur</property>` par ligne.
pub fn props_xml(sections: &PropSections) -> String {
    let mut o = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<properties>\n");
    for (title, rows) in sections {
        o += &format!("  <section name=\"{}\">\n", xml_escape(title));
        for (k, v) in rows {
            o += &format!("    <property name=\"{}\">{}</property>\n", xml_escape(k), xml_escape(v));
        }
        o += "  </section>\n";
    }
    o + "</properties>\n"
}
