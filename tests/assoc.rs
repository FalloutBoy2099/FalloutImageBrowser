//! Association de fichiers : définitions MIME / .desktop (toutes plates-formes) et clés de registre (Windows, dans une
//! branche de test : les associations réelles de l'utilisateur ne sont jamais modifiées).

use fallout_image_browser::assoc;
use std::path::Path;

#[test]
fn groups_and_labels() {
    assert_eq!(assoc::list(&assoc::FALLOUT), ".FRM, .RIX");
    assert_eq!(assoc::list(&assoc::TACTICS), ".ZAR, .TIL, .SPR");
    assert_eq!(assoc::list(&assoc::ARCANUM), ".ART");
    assert_eq!(assoc::all().len(), 6);
}

#[test]
fn mime_and_desktop_files_cover_every_extension() {
    let xml = assoc::mime_xml();
    let desktop = assoc::desktop_entry(Path::new("/opt/fib/fallout_image_browser"));
    for t in assoc::all() {
        assert!(xml.contains(&format!("type=\"{}\"", assoc::mime_type(t.ext))), "{}", t.ext);
        assert!(xml.contains(&format!("pattern=\"*.{}\"", t.ext)), "{}", t.ext);
        assert!(desktop.contains(&format!("{};", assoc::mime_type(t.ext))), "{}", t.ext);
    }
    assert!(desktop.contains("Exec=\"/opt/fib/fallout_image_browser\" %f"));
    assert!(desktop.starts_with("[Desktop Entry]\n"));
    // Un chemin avec espaces, `%`, guillemet ou `$` reste utilisable dans Exec.
    let odd = assoc::desktop_entry(Path::new("/opt/My 100%/a\"b$c"));
    assert!(odd.contains(r#"Exec="/opt/My 100%%/a\\"b\\$c" %f"#), "{odd}");
}

#[cfg(windows)]
#[test]
fn registry_keys_are_written_under_a_test_root() {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegDeleteTreeW, RegGetValueW};

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }
    fn read(key: &str) -> Option<String> {
        let k = wide(key);
        let mut buf = vec![0u16; 1024];
        let mut bytes = (buf.len() * 2) as u32;
        let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, k.as_ptr(), std::ptr::null(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr() as *mut _, &mut bytes) };
        (r == 0).then(|| String::from_utf16_lossy(&buf[..(bytes as usize / 2).saturating_sub(1)]))
    }

    let top = format!(r"Software\FibAssocTest_{}", std::process::id());
    let root = format!(r"{top}\Classes");
    let exe = Path::new(r"C:\Program Files\Fallout Image Browser\fallout_image_browser.exe");
    assoc::register_in(&root, exe, &assoc::FALLOUT).unwrap();
    let frm_prog = read(&format!(r"{root}\.frm"));
    let command = read(&format!(r"{root}\FalloutImageBrowser.FRM\shell\open\command"));
    let icon = read(&format!(r"{root}\FalloutImageBrowser.RIX\DefaultIcon"));
    let description = read(&format!(r"{root}\FalloutImageBrowser.RIX"));
    let untouched = read(&format!(r"{root}\.zar"));
    // Nettoyage avant les vérifications : rien ne doit rester dans le registre même si une assertion échoue.
    unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, wide(&top).as_ptr()) };
    assert_eq!(frm_prog.as_deref(), Some("FalloutImageBrowser.FRM"));
    assert_eq!(command.as_deref(), Some(r#""C:\Program Files\Fallout Image Browser\fallout_image_browser.exe" "%1""#));
    assert_eq!(icon.as_deref(), Some(r#""C:\Program Files\Fallout Image Browser\fallout_image_browser.exe",0"#));
    assert_eq!(description.as_deref(), Some("Fallout RIX image"));
    assert_eq!(untouched, None, "seules les extensions demandées sont enregistrées");
    assert_eq!(read(&format!(r"{root}\.frm")), None, "la branche de test doit être supprimée");
}
