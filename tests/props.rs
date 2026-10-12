//! Copie des informations d'une image : texte, JSON et XML.

use fallout_image_browser::model::*;
use fallout_image_browser::props::{PropSections, props_json, props_text, props_xml};

fn sample() -> PropSections {
    vec![
        (
            "Fichier".to_string(),
            vec![("Nom".to_string(), "a \"b\" <c> & 'd'.frm".to_string()), ("Dossier".to_string(), "C:\\x\\y".to_string()), ("Nom".to_string(), "doublon".to_string())],
        ),
        ("Fichier".to_string(), vec![("Ligne".to_string(), "tab\there\nnewline \u{1}".to_string())]),
        ("Palette".to_string(), vec![("0".to_string(), "#FF0000".to_string())]),
        ("Vide".to_string(), vec![]),
    ]
}

#[test]
fn json_is_valid_and_keeps_every_value() {
    let v: serde_json::Value = serde_json::from_str(&props_json(&sample())).expect("JSON valide");
    let o = v.as_object().unwrap();
    assert_eq!(o.len(), 4, "les sections de même nom reçoivent un suffixe");
    assert_eq!(o["Fichier"]["Nom"], "a \"b\" <c> & 'd'.frm");
    assert_eq!(o["Fichier"]["Nom (2)"], "doublon");
    assert_eq!(o["Fichier"]["Dossier"], "C:\\x\\y");
    assert_eq!(o["Fichier (2)"]["Ligne"], "tab\there\nnewline \u{1}");
    assert_eq!(o["Palette"]["0"], "#FF0000");
    assert!(o["Vide"].as_object().unwrap().is_empty());
}

#[test]
fn xml_is_well_formed_and_escaped() {
    let xml = props_xml(&sample());
    let doc = roxmltree::Document::parse(&xml).unwrap_or_else(|e| panic!("XML invalide : {e}\n{xml}"));
    let root = doc.root_element();
    assert_eq!(root.tag_name().name(), "properties");
    let sections: Vec<_> = root.children().filter(|n| n.is_element()).collect();
    assert_eq!(sections.len(), 4);
    assert_eq!(sections[0].attribute("name"), Some("Fichier"));
    let first = sections[0].children().find(|n| n.is_element()).unwrap();
    assert_eq!(first.attribute("name"), Some("Nom"));
    assert_eq!(first.text(), Some("a \"b\" <c> & 'd'.frm"));
    // Le caractère de contrôle (interdit en XML) est supprimé, la tabulation et le retour à la ligne sont conservés.
    let line = sections[1].children().find(|n| n.is_element()).unwrap();
    assert_eq!(line.text(), Some("tab\there\nnewline "));
    assert_eq!(sections[3].children().filter(|n| n.is_element()).count(), 0);
}

#[test]
fn text_lists_sections_and_rows() {
    let t = props_text(&sample());
    assert!(t.starts_with("[Fichier]\nNom: a \"b\" <c> & 'd'.frm\n"));
    assert!(t.contains("\n[Palette]\n0: #FF0000\n"));
}

#[test]
fn a_real_picture_reaches_the_three_formats() {
    let pic = Picture::single(Format::Bmp, 2, 1, Pixels::Rgba(vec![0, 0, 0, 255, 255, 255, 255, 255]), None);
    let sections = fallout_image_browser::props::collect_props(&pic, 0, 0);
    serde_json::from_str::<serde_json::Value>(&props_json(&sections)).unwrap();
    roxmltree::Document::parse(&props_xml(&sections)).unwrap();
    assert!(!props_text(&sections).is_empty() || sections.is_empty());
}
