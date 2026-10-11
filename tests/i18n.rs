use fallout_image_browser::i18n::{self, Lang};

#[test]
fn every_language_has_every_key() {
    let keys = i18n::reference_keys();
    assert!(keys.len() > 100);
    let mut missing = Vec::new();
    for l in Lang::ALL {
        for k in &keys {
            if !i18n::has_key(l, k) {
                missing.push(format!("{}: {k}", l.code()));
            }
        }
    }
    assert!(missing.is_empty(), "clés manquantes : {missing:?}");
}

#[test]
fn keys_used_in_sources_exist() {
    // Toutes les clés littérales du code source doivent exister dans le fichier anglais.
    let mut used = std::collections::BTreeSet::new();
    for entry in std::fs::read_dir("src").unwrap().flatten().chain(std::fs::read_dir("src/formats").unwrap().flatten()) {
        let p = entry.path();
        if p.extension().map(|e| e == "rs").unwrap_or(false) {
            let s = std::fs::read_to_string(&p).unwrap();
            let b = s.as_bytes();
            let mut i = 0;
            while i < b.len() {
                if b[i] == b'"' {
                    let start = i + 1;
                    let mut j = start;
                    while j < b.len() && b[j] != b'"' && b[j] != b'\n' {
                        j += 1;
                    }
                    let lit = &s[start..j.min(s.len())];
                    let ok = ["p.", "c.", "m.", "t.", "s.", "a.", "sec.", "about."].iter().any(|pre| lit.starts_with(pre))
                        && !lit.ends_with('.') && lit.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_');
                    if ok {
                        used.insert(lit.to_string());
                    }
                    i = j + 1;
                } else {
                    i += 1;
                }
            }
        }
    }
    let refs: std::collections::HashSet<_> = i18n::reference_keys().into_iter().collect();
    let missing: Vec<_> = used.iter().filter(|k| !refs.contains(k.as_str())).collect();
    assert!(missing.is_empty(), "clés utilisées mais absentes : {missing:?}");
}

#[test]
fn detect_returns_a_supported_language() {
    let l = Lang::detect();
    assert!(Lang::ALL.contains(&l));
}
