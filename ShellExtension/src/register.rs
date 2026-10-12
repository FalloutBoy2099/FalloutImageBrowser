//! Enregistrement de la DLL : classe COM et association aux extensions `.frm` et `.rix`, dans `HKEY_CURRENT_USER`
//! (aucun droit administrateur). Les fonctions prennent la branche de départ en paramètre pour pouvoir être testées
//! ailleurs que dans `Software\Classes`.

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteKeyW, RegDeleteTreeW, RegGetValueW,
    RegOpenKeyExW, RegQueryInfoKeyW, RegSetValueExW,
};
use windows::core::PCWSTR;

/// Identifiant de la classe du fournisseur de vignettes.
pub const CLSID: &str = "{7A3C9D52-4E1B-4F6A-9C2D-5B8E0F1A6D37}";
/// Identifiant d'interface qui désigne un fournisseur de vignettes dans `ShellEx`.
pub const THUMBNAIL_HANDLER: &str = "{E357FCCD-A995-4576-B01F-234630154E96}";
/// Extensions prises en charge : Fallout (FRM, RIX), Arcanum (ART) et Fallout Tactics (ZAR, TIL, SPR).
pub const EXTENSIONS: [&str; 6] = ["frm", "rix", "art", "zar", "til", "spr"];

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Écrit une valeur texte (`name` = None : valeur par défaut) sous `HKEY_CURRENT_USER\key`.
fn set(key: &str, name: Option<&str>, value: &str) -> Result<(), String> {
    let k = wide(key);
    let mut h = HKEY::default();
    // SAFETY : pointeurs valides vers des tampons terminés par zéro qui vivent pendant l'appel.
    unsafe {
        let r = RegCreateKeyExW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr()), None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut h, None);
        if r != ERROR_SUCCESS {
            return Err(format!("registre : {key} (code {})", r.0));
        }
        let n = name.map(wide);
        let v = wide(value);
        let bytes = std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 2);
        let r = RegSetValueExW(h, n.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr())), None, REG_SZ, Some(bytes));
        let _ = RegCloseKey(h);
        if r != ERROR_SUCCESS {
            return Err(format!("registre : {key} (code {})", r.0));
        }
    }
    Ok(())
}

/// Valeur par défaut de `HKEY_CURRENT_USER\key`, si elle existe.
pub fn get_default(key: &str) -> Option<String> {
    let k = wide(key);
    let mut buf = vec![0u16; 1024];
    let mut bytes = (buf.len() * 2) as u32;
    // SAFETY : `buf` est assez grand pour `bytes` octets.
    let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr()), PCWSTR::null(), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut bytes)) };
    (r == ERROR_SUCCESS).then(|| String::from_utf16_lossy(&buf[..(bytes as usize / 2).saturating_sub(1)]))
}

/// La clé `HKEY_CURRENT_USER\key` existe-t-elle ?
pub fn key_exists(key: &str) -> bool {
    let k = wide(key);
    let mut h = HKEY::default();
    // SAFETY : pointeur valide vers un tampon terminé par zéro.
    unsafe {
        let r = RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr()), None, KEY_READ, &mut h);
        if r == ERROR_SUCCESS {
            let _ = RegCloseKey(h);
        }
        r == ERROR_SUCCESS
    }
}

/// Supprime la clé seulement si elle ne contient ni sous-clé ni valeur (on ne retire jamais le travail d'un autre programme).
fn delete_if_empty(key: &str) {
    let k = wide(key);
    let mut h = HKEY::default();
    // SAFETY : pointeurs valides ; la poignée est fermée avant la suppression.
    unsafe {
        if RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr()), None, KEY_READ, &mut h) != ERROR_SUCCESS {
            return;
        }
        let (mut subkeys, mut values) = (0u32, 0u32);
        let r = RegQueryInfoKeyW(h, None, None, None, Some(&mut subkeys), None, None, Some(&mut values), None, None, None, None);
        let _ = RegCloseKey(h);
        if r == ERROR_SUCCESS && subkeys == 0 && values == 0 {
            let _ = RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr()));
        }
    }
}

/// Supprime une branche (une clé absente n'est pas une erreur).
pub fn delete_tree(key: &str) -> Result<(), String> {
    let k = wide(key);
    // SAFETY : pointeur valide vers un tampon terminé par zéro.
    let r = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr())) };
    if r == ERROR_SUCCESS || r == ERROR_FILE_NOT_FOUND { Ok(()) } else { Err(format!("registre : {key} (code {})", r.0)) }
}

/// Déclare la classe COM (serveur en processus : `dll`) et associe les extensions, sous `HKEY_CURRENT_USER\<root>`.
pub fn register_at(root: &str, dll: &str) -> Result<(), String> {
    let class = format!(r"{root}\CLSID\{CLSID}");
    set(&class, None, "Fallout thumbnail provider")?;
    set(&format!(r"{class}\InprocServer32"), None, dll)?;
    set(&format!(r"{class}\InprocServer32"), Some("ThreadingModel"), "Apartment")?;
    for ext in EXTENSIONS {
        set(&format!(r"{root}\.{ext}\ShellEx\{THUMBNAIL_HANDLER}"), None, CLSID)?;
    }
    Ok(())
}

/// Retire ce que `register_at` a écrit (sans toucher à une association qui pointerait vers un autre fournisseur).
pub fn unregister_at(root: &str) -> Result<(), String> {
    for ext in EXTENSIONS {
        let key = format!(r"{root}\.{ext}\ShellEx\{THUMBNAIL_HANDLER}");
        if get_default(&key).is_some_and(|v| v.eq_ignore_ascii_case(CLSID)) {
            delete_tree(&key)?;
            // Les clés parentes créées par l'enregistrement disparaissent aussi si elles sont vides.
            delete_if_empty(&format!(r"{root}\.{ext}\ShellEx"));
            delete_if_empty(&format!(r"{root}\.{ext}"));
        }
    }
    delete_tree(&format!(r"{root}\CLSID\{CLSID}"))
}

/// Prévient l'Explorateur que les associations ont changé (les vignettes sont régénérées).
pub fn notify_shell() {
    use windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
    // SAFETY : aucun élément n'est transmis.
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}

/// Chemin de cette DLL.
fn dll_path() -> Result<String, String> {
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
    let mut buf = vec![0u16; 1024];
    // SAFETY : tampon valide ; le module est celui enregistré par DllMain.
    let n = unsafe { GetModuleFileNameW(Some(HMODULE(crate::module_handle() as *mut _)), &mut buf) } as usize;
    if n == 0 {
        return Err("chemin de la DLL introuvable".into());
    }
    Ok(String::from_utf16_lossy(&buf[..n]))
}

/// `regsvr32` : enregistre pour l'utilisateur courant.
pub fn install() -> Result<(), String> {
    register_at(r"Software\Classes", &dll_path()?)?;
    notify_shell();
    Ok(())
}

/// `regsvr32 /u`.
pub fn uninstall() -> Result<(), String> {
    unregister_at(r"Software\Classes")?;
    notify_shell();
    Ok(())
}
