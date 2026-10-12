//! Rendu des vignettes (toutes plates-formes) ; objet COM et enregistrement (Windows).
//! Les fichiers d'exemple sont ceux du projet principal (`../_test_files`).

use fallout_thumbnails::thumbnail;
use std::path::PathBuf;

/// Chemin absolu et sans `..` (l'API du shell refuse les chemins relatifs et le préfixe `\?\`).
fn sample_path(rel: &str) -> PathBuf {
    let p: PathBuf = [env!("CARGO_MANIFEST_DIR"), "..", "_test_files", rel].iter().collect();
    let c = std::fs::canonicalize(&p).unwrap_or(p);
    let s = c.to_string_lossy().to_string();
    PathBuf::from(s.strip_prefix(r"\\?\").unwrap_or(&s))
}

fn sample(rel: &str) -> Vec<u8> {
    let p = sample_path(rel);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fib_thumb_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn every_supported_family_has_a_thumbnail() {
    for rel in
        ["fallout/EXP.FRM", "fallout/SPLASH0.rix", "arcanum/MusicBoxMap.art", "tactics/zar/charcreate.zar", "tactics/spr/Mace Glove.spr", "tactics/til/Tree Pine Type 1 02 ii.TIL"]
    {
        let t = thumbnail::render_file(&sample_path(rel), 128).unwrap_or_else(|e| panic!("{rel}: {e}"));
        assert!(t.width > 0 && t.height > 0 && t.width <= 128 && t.height <= 128, "{rel}");
        assert!(t.bgra.chunks_exact(4).any(|p| p[3] > 0), "{rel}: image entièrement transparente");
    }
}

/// Un `.PAL` du même nom que le FRM remplace la palette Fallout (comme dans l'application).
#[test]
fn a_same_name_pal_file_is_used() {
    let dir = temp_dir("pal");
    let frm = dir.join("EXP.FRM");
    std::fs::copy(sample_path("fallout/EXP.FRM"), &frm).unwrap();
    let default = thumbnail::render_file(&frm, 64).unwrap();
    // Palette Fallout (768 octets, valeurs sur 6 bits) entièrement blanche.
    std::fs::write(dir.join("EXP.pal"), vec![63u8; 768]).unwrap();
    let white = thumbnail::render_file(&frm, 64).unwrap();
    assert_eq!((default.width, default.height), (white.width, white.height));
    let opaque = |t: &thumbnail::Thumb| t.bgra.chunks_exact(4).filter(|p| p[3] == 255).map(|p| (p[0], p[1], p[2])).collect::<Vec<_>>();
    // 63 sur 6 bits donne environ 252 sur 8 bits : « blanc » = les trois canaux au moins à 250.
    let is_white = |&(b, g, r): &(u8, u8, u8)| b >= 250 && g >= 250 && r >= 250;
    assert!(!opaque(&white).is_empty() && opaque(&white).iter().all(is_white), "tous les pixels opaques doivent être blancs");
    assert!(!opaque(&default).iter().all(is_white), "la palette Fallout n'est pas blanche");
    let _ = std::fs::remove_dir_all(&dir);
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
    use super::{sample_path, temp_dir};
    use fallout_thumbnails::{CLSID_THUMBNAIL_PROVIDER, DllCanUnloadNow, DllGetClassObject, register};
    use std::ffi::c_void;
    use windows::Win32::Foundation::{CLASS_E_CLASSNOTAVAILABLE, S_OK};
    use windows::Win32::Graphics::Gdi::{BITMAP, DeleteObject, GetObjectW, HBITMAP};
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, IClassFactory};
    use windows::Win32::UI::Shell::PropertiesSystem::IInitializeWithStream;
    use windows::Win32::UI::Shell::{IThumbnailProvider, SHCreateStreamOnFileEx, WTS_ALPHATYPE};
    use windows::core::{GUID, Interface, PCWSTR};

    fn factory() -> IClassFactory {
        let mut out: Option<IClassFactory> = None;
        let hr = unsafe { DllGetClassObject(&CLSID_THUMBNAIL_PROVIDER, &IClassFactory::IID, &mut out as *mut _ as *mut *mut c_void) };
        assert_eq!(hr, S_OK);
        out.expect("fabrique de classes")
    }

    /// Appelle l'objet comme le ferait l'Explorateur : flux sur le fichier -> Initialize -> GetThumbnail.
    fn thumbnail_via_com(path: &std::path::Path, size: u32) -> windows::core::Result<(i32, i32, i32)> {
        // Le shell exige un thread COM initialisé (le thread de test ne l'est pas).
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        let init: IInitializeWithStream = unsafe { factory().CreateInstance(None)? };
        let wide: Vec<u16> = path.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
        let stream = unsafe { SHCreateStreamOnFileEx(PCWSTR(wide.as_ptr()), 0, 0, false, None)? };
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
        let (w, h, alpha) = thumbnail_via_com(&sample_path("fallout/SPLASH0.rix"), 256).unwrap();
        assert_eq!((w, h), (256, 192));
        assert_eq!(alpha, 2, "WTSAT_ARGB");
        let (w, h, _) = thumbnail_via_com(&sample_path("fallout/HMJMPSAA.FRM"), 128).unwrap();
        assert!(w > 0 && h > 0 && w <= 128 && h <= 128);
        // Les autres familles de fichiers passent par le même objet.
        for rel in ["arcanum/MusicBoxMap.art", "tactics/zar/char_bar.zar", "tactics/spr/dopeknife.spr", "tactics/til/Log Type 2 02.TIL"] {
            let (w, h, _) = thumbnail_via_com(&sample_path(rel), 128).unwrap_or_else(|e| panic!("{rel}: {e}"));
            assert!(w > 0 && h > 0 && w <= 128 && h <= 128, "{rel}");
        }
        // Un fichier qui n'est pas une image : une erreur COM, pas un plantage.
        let dir = temp_dir("garbage");
        let bad = dir.join("bad.frm");
        std::fs::write(&bad, b"garbage garbage garbage").unwrap();
        assert!(thumbnail_via_com(&bad, 128).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn class_factory_rejects_other_classes() {
        let mut out: *mut c_void = std::ptr::null_mut();
        let other = GUID::from_u128(0x11112222_33334444_55556666_77778888);
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
        let other = register::get_default(&format!(r"{root}\.bmp\ShellEx\{}", register::THUMBNAIL_HANDLER));
        register::unregister_at(&root).unwrap();
        let after = register::get_default(&format!(r"{root}\.frm\ShellEx\{}", register::THUMBNAIL_HANDLER));
        let class_after = register::get_default(&format!(r"{root}\CLSID\{}\InprocServer32", register::CLSID));
        let parents_left = register::key_exists(&format!(r"{root}\.frm")) || register::key_exists(&format!(r"{root}\.rix"));
        register::delete_tree(&top).unwrap();
        result.unwrap();
        assert_eq!(clsid.as_deref(), Some(dll));
        assert_eq!(frm.as_deref(), Some(register::CLSID));
        assert_eq!(rix.as_deref(), Some(register::CLSID));
        assert_eq!(other, None, "seules les extensions Fallout, Arcanum et Fallout Tactics sont liées");
        assert_eq!((after, class_after), (None, None), "tout doit être retiré");
        assert!(!parents_left, "les clés vides créées par l'enregistrement doivent disparaître");
    }
}
