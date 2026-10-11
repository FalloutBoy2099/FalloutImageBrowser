//! Rendu des vignettes (toutes plates-formes) ; objet COM et enregistrement (Windows).
//! Les fichiers d'exemple sont ceux du projet principal (`../_test_files`).

use fallout_thumbnails::thumbnail;
use std::path::PathBuf;

fn sample(rel: &str) -> Vec<u8> {
    let p: PathBuf = [env!("CARGO_MANIFEST_DIR"), "..", "_test_files", rel].iter().collect();
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

#[test]
fn frm_and_rix_thumbnails() {
    // RIX 640 × 480 réduit à 128 : même rapport largeur / hauteur.
    let t = thumbnail::render(&sample("fallout/SPLASH0.rix"), 128).unwrap();
    assert_eq!((t.width, t.height), (128, 96));
    assert_eq!(t.bgra.len(), 128 * 96 * 4);
    assert!(t.bgra.chunks_exact(4).any(|p| p[3] == 255 && (p[0] | p[1] | p[2]) != 0), "l'image ne doit pas être vide");
    // Une petite image n'est pas agrandie ; les couleurs sont prémultipliées par l'alpha.
    for name in ["fallout/EXP.FRM", "fallout/HEX.frm", "fallout/HMJMPSAA.FRM"] {
        let t = thumbnail::render(&sample(name), 256).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(t.width <= 256 && t.height <= 256 && t.width > 0 && t.height > 0, "{name}");
        assert!(t.bgra.chunks_exact(4).all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]), "{name}: couleurs non prémultipliées");
        assert!(t.bgra.chunks_exact(4).any(|p| p[3] > 0), "{name}: image entièrement transparente");
    }
    let small = thumbnail::render(&sample("fallout/HEX.frm"), 4096).unwrap();
    let again = thumbnail::render(&sample("fallout/HEX.frm"), 8192).unwrap();
    assert_eq!((small.width, small.height), (again.width, again.height), "pas d'agrandissement");
}

#[test]
fn bad_input_is_an_error_not_a_crash() {
    assert!(thumbnail::render(b"", 128).is_err());
    assert!(thumbnail::render(b"not an image at all", 128).is_err());
    // Fichiers tronqués ou abîmés : jamais de panique.
    for name in ["fallout/SPLASH0.rix", "fallout/HMJMPSAA.FRM", "fallout/EXP.FRM"] {
        let data = sample(name);
        for cut in [0, 1, 8, 31, 100, data.len() / 2, data.len() - 1] {
            let _ = thumbnail::render(&data[..cut.min(data.len())], 64);
        }
        let mut seed = 0x1234_5678_9abc_def0u64;
        for _ in 0..200 {
            let mut d = data.clone();
            for _ in 0..4 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let i = (seed as usize) % d.len();
                d[i] = (seed >> 32) as u8;
            }
            let _ = thumbnail::render(&d, 64);
        }
    }
}

#[cfg(windows)]
mod windows_only {
    use super::sample;
    use fallout_thumbnails::{CLSID_THUMBNAIL_PROVIDER, DllCanUnloadNow, DllGetClassObject, register};
    use std::ffi::c_void;
    use windows::Win32::Foundation::{CLASS_E_CLASSNOTAVAILABLE, S_OK};
    use windows::Win32::Graphics::Gdi::{BITMAP, DeleteObject, GetObjectW, HBITMAP};
    use windows::Win32::System::Com::IClassFactory;
    use windows::Win32::UI::Shell::PropertiesSystem::IInitializeWithStream;
    use windows::Win32::UI::Shell::{IThumbnailProvider, SHCreateMemStream, WTS_ALPHATYPE};
    use windows::core::{GUID, Interface};

    fn factory() -> IClassFactory {
        let mut out: Option<IClassFactory> = None;
        let hr = unsafe { DllGetClassObject(&CLSID_THUMBNAIL_PROVIDER, &IClassFactory::IID, &mut out as *mut _ as *mut *mut c_void) };
        assert_eq!(hr, S_OK);
        out.expect("fabrique de classes")
    }

    /// Appelle l'objet comme le ferait l'Explorateur : flux -> Initialize -> GetThumbnail.
    fn thumbnail_via_com(data: &[u8], size: u32) -> windows::core::Result<(i32, i32, i32)> {
        let init: IInitializeWithStream = unsafe { factory().CreateInstance(None)? };
        let stream = unsafe { SHCreateMemStream(Some(data)) }.expect("flux mémoire");
        unsafe { init.Initialize(&stream, 0)? };
        let provider: IThumbnailProvider = init.cast()?;
        let (mut hbmp, mut alpha) = (HBITMAP::default(), WTS_ALPHATYPE::default());
        unsafe { provider.GetThumbnail(size, &mut hbmp, &mut alpha)? };
        let mut bm = BITMAP::default();
        unsafe {
            GetObjectW(hbmp.into(), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut c_void));
            let _ = DeleteObject(hbmp.into());
        }
        Ok((bm.bmWidth, bm.bmHeight, alpha.0))
    }

    #[test]
    fn com_object_returns_a_bitmap() {
        let (w, h, alpha) = thumbnail_via_com(&sample("fallout/SPLASH0.rix"), 256).unwrap();
        assert_eq!((w, h), (256, 192));
        assert_eq!(alpha, 2, "WTSAT_ARGB");
        let (w, h, _) = thumbnail_via_com(&sample("fallout/HMJMPSAA.FRM"), 128).unwrap();
        assert!(w > 0 && h > 0 && w <= 128 && h <= 128);
        // Un fichier qui n'est pas une image : une erreur COM, pas un plantage.
        assert!(thumbnail_via_com(b"garbage garbage garbage", 128).is_err());
    }

    #[test]
    fn class_factory_rejects_other_classes() {
        let mut out: *mut c_void = std::ptr::null_mut();
        let other = GUID::from_u128(0x1111_2222_3333_4444_5555_666677778888);
        let hr = unsafe { DllGetClassObject(&other, &IClassFactory::IID, &mut out) };
        assert_eq!(hr, CLASS_E_CLASSNOTAVAILABLE);
        assert!(out.is_null());
        // Rien n'est en vie : la DLL peut être déchargée.
        assert_eq!(DllCanUnloadNow(), S_OK);
    }

    #[test]
    fn registration_writes_and_removes_the_keys() {
        // Branche de test : les vraies associations de l'utilisateur ne sont pas touchées.
        let root = format!(r"Software\FibThumbTest_{}\Classes", std::process::id());
        let top = root.trim_end_matches(r"\Classes").to_string();
        let dll = r"C:\Tools\ShellExtension\fallout_thumbnails.dll";
        let result = register::register_at(&root, dll);
        let clsid = register::get_default(&format!(r"{root}\CLSID\{}\InprocServer32", register::CLSID));
        let frm = register::get_default(&format!(r"{root}\.frm\ShellEx\{}", register::THUMBNAIL_HANDLER));
        let rix = register::get_default(&format!(r"{root}\.rix\ShellEx\{}", register::THUMBNAIL_HANDLER));
        let other = register::get_default(&format!(r"{root}\.art\ShellEx\{}", register::THUMBNAIL_HANDLER));
        register::unregister_at(&root).unwrap();
        let after = register::get_default(&format!(r"{root}\.frm\ShellEx\{}", register::THUMBNAIL_HANDLER));
        let class_after = register::get_default(&format!(r"{root}\CLSID\{}\InprocServer32", register::CLSID));
        let parents_left = register::key_exists(&format!(r"{root}\.frm")) || register::key_exists(&format!(r"{root}\.rix"));
        register::delete_tree(&top).unwrap();
        result.unwrap();
        assert_eq!(clsid.as_deref(), Some(dll));
        assert_eq!(frm.as_deref(), Some(register::CLSID));
        assert_eq!(rix.as_deref(), Some(register::CLSID));
        assert_eq!(other, None, "seules les extensions .frm et .rix sont liées");
        assert_eq!((after, class_after), (None, None), "tout doit être retiré");
        assert!(!parents_left, "les clés vides créées par l'enregistrement doivent disparaître");
    }
}
