//! Stockage des préférences de l'utilisateur.
//!
//! * Windows : base de registre, `HKEY_CURRENT_USER\SOFTWARE\FALLOUT\FalloutImageBrowser` (une valeur texte par élément :
//!   `fib_settings_v1` pour les réglages, `window` pour la fenêtre).
//! * Linux : `$XDG_CONFIG_HOME/Fallout/FalloutImageBrowser/` (à défaut `~/.config/...`), un fichier par élément.
//! * macOS : `~/Library/Application Support/Fallout/FalloutImageBrowser/`, un fichier par élément.
//!
//! `Store` implémente `eframe::Storage` : `eframe::get_value` / `set_value` servent à (dé)sérialiser les réglages.

use std::collections::HashMap;

/// Chemin de la clé de registre (Windows), sous `HKEY_CURRENT_USER`.
pub const REG_KEY: &str = r"SOFTWARE\FALLOUT\FalloutImageBrowser";

/// Préférences : chaque écriture est immédiatement envoyée au support de stockage.
pub struct Store {
    /// Windows : sous-clé du registre ; ailleurs : dossier (utile aux tests).
    place: String,
    cache: HashMap<String, String>,
}

impl Store {
    /// Emplacement normal de l'utilisateur.
    pub fn open() -> Store {
        Store::at(&default_place())
    }

    /// Emplacement choisi (tests).
    pub fn at(place: &str) -> Store {
        Store { place: place.to_string(), cache: HashMap::new() }
    }

    /// Lit une valeur texte.
    pub fn read(&self, name: &str) -> Option<String> {
        self.cache.get(name).cloned().or_else(|| imp::read(&self.place, name))
    }

    /// Écrit une valeur texte ; renvoie faux si le support de stockage n'est pas accessible. Rien n'est écrit si la valeur
    /// enregistrée est déjà identique (la sauvegarde périodique ne touche donc ni le registre ni le disque sans changement).
    pub fn write(&mut self, name: &str, value: &str) -> bool {
        self.cache.insert(name.to_string(), value.to_string());
        if imp::read(&self.place, name).as_deref() == Some(value) {
            return true;
        }
        imp::write(&self.place, name, value)
    }

    /// Supprime une valeur.
    pub fn remove(&mut self, name: &str) {
        self.cache.remove(name);
        imp::remove(&self.place, name);
    }

    /// Supprime tout ce qui a été enregistré à cet emplacement (tests, « réinitialiser »).
    pub fn clear(&mut self) {
        self.cache.clear();
        imp::clear(&self.place);
    }
}

impl eframe::Storage for Store {
    fn get_string(&self, key: &str) -> Option<String> {
        self.read(key)
    }
    fn set_string(&mut self, key: &str, value: String) {
        self.write(key, &value);
    }
    fn remove_string(&mut self, key: &str) {
        self.remove(key);
    }
    fn flush(&mut self) {}
}

#[cfg(windows)]
fn default_place() -> String {
    REG_KEY.to_string()
}

#[cfg(not(windows))]
fn default_place() -> String {
    let base = if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
    };
    base.map(|b| b.join("Fallout").join("FalloutImageBrowser")).unwrap_or_default().to_string_lossy().to_string()
}

// ----------------------------------------------------------------------------- Windows : registre

#[cfg(windows)]
mod imp {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegDeleteKeyValueW, RegGetValueW, RegSetValueExW,
    };

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    pub fn write(key: &str, name: &str, value: &str) -> bool {
        let k = wide(key);
        let n = wide(name);
        let v = wide(value);
        let mut h: HKEY = std::ptr::null_mut();
        // SAFETY : pointeurs valides vers des tampons terminés par zéro qui vivent pendant l'appel.
        unsafe {
            if RegCreateKeyExW(HKEY_CURRENT_USER, k.as_ptr(), 0, std::ptr::null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, std::ptr::null(), &mut h, std::ptr::null_mut()) != 0 {
                return false;
            }
            let r = RegSetValueExW(h, n.as_ptr(), 0, REG_SZ, v.as_ptr() as *const u8, (v.len() * 2) as u32);
            RegCloseKey(h);
            r == 0
        }
    }

    pub fn read(key: &str, name: &str) -> Option<String> {
        let k = wide(key);
        let n = wide(name);
        // SAFETY : tampons valides ; la taille est demandée d'abord, puis la valeur lue dans un tampon de cette taille.
        unsafe {
            let mut bytes = 0u32;
            if RegGetValueW(HKEY_CURRENT_USER, k.as_ptr(), n.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut bytes) != 0 || bytes < 2 {
                return None;
            }
            let mut buf = vec![0u16; (bytes as usize).div_ceil(2)];
            let mut bytes2 = (buf.len() * 2) as u32;
            if RegGetValueW(HKEY_CURRENT_USER, k.as_ptr(), n.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr() as *mut _, &mut bytes2) != 0 {
                return None;
            }
            let len = (bytes2 as usize / 2).saturating_sub(1);
            Some(String::from_utf16_lossy(&buf[..len.min(buf.len())]))
        }
    }

    pub fn remove(key: &str, name: &str) {
        let (k, n) = (wide(key), wide(name));
        // SAFETY : tampons valides terminés par zéro.
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), n.as_ptr()) };
    }

    pub fn clear(key: &str) {
        let k = wide(key);
        // SAFETY : tampon valide terminé par zéro.
        unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, k.as_ptr()) };
    }
}

// ----------------------------------------------------------------------------- Linux / macOS : fichiers

#[cfg(not(windows))]
mod imp {
    use std::path::Path;

    fn file(dir: &str, name: &str) -> Option<std::path::PathBuf> {
        // Le nom ne doit jamais sortir du dossier.
        (!dir.is_empty() && !name.contains(['/', '\\']) && !name.is_empty()).then(|| Path::new(dir).join(name))
    }

    pub fn write(dir: &str, name: &str, value: &str) -> bool {
        let Some(f) = file(dir, name) else { return false };
        std::fs::create_dir_all(dir).is_ok() && std::fs::write(f, value).is_ok()
    }

    pub fn read(dir: &str, name: &str) -> Option<String> {
        std::fs::read_to_string(file(dir, name)?).ok()
    }

    pub fn remove(dir: &str, name: &str) {
        if let Some(f) = file(dir, name) {
            let _ = std::fs::remove_file(f);
        }
    }

    pub fn clear(dir: &str) {
        if !dir.is_empty() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_roundtrip_with_newlines_and_accents() {
        #[cfg(windows)]
        let place = format!(r"SOFTWARE\FALLOUT\FibStoreTest_{}", std::process::id());
        #[cfg(not(windows))]
        let place = std::env::temp_dir().join(format!("fib_store_test_{}", std::process::id())).to_string_lossy().to_string();
        let mut s = Store::at(&place);
        let text = "a=1\nnom=éàü 日本\n(x: 2)";
        assert!(s.write("fib_settings_v1", text));
        // Une nouvelle instance relit depuis le support, pas depuis le cache.
        assert_eq!(Store::at(&place).read("fib_settings_v1").as_deref(), Some(text));
        assert_eq!(Store::at(&place).read("absent"), None);
        // Réécrire la même valeur réussit sans rien changer ; une valeur différente est bien enregistrée.
        assert!(s.write("fib_settings_v1", text));
        assert!(s.write("fib_settings_v1", "autre"));
        assert_eq!(Store::at(&place).read("fib_settings_v1").as_deref(), Some("autre"));
        s.clear();
        assert_eq!(Store::at(&place).read("fib_settings_v1"), None);
    }
}
