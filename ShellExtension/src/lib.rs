//! Fournisseur de vignettes pour l'Explorateur Windows : affiche les images des fichiers Fallout `.FRM` et `.RIX`.
//!
//! La DLL expose les points d'entrée COM habituels (`DllGetClassObject`, `DllCanUnloadNow`) et ceux de `regsvr32`
//! (`DllRegisterServer`, `DllUnregisterServer`). Le décodage vient du projet principal (`../`).

pub mod thumbnail;

#[cfg(windows)]
mod com;
#[cfg(windows)]
pub mod register;

#[cfg(windows)]
mod exports {
    use super::{com, register};
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicIsize, Ordering};
    use windows::Win32::Foundation::{CLASS_E_CLASSNOTAVAILABLE, E_FAIL, E_INVALIDARG, HINSTANCE, S_FALSE, S_OK};
    use windows::Win32::System::Com::IClassFactory;
    use windows::core::{BOOL, GUID, HRESULT, Interface};

    /// Classe COM du fournisseur de vignettes.
    pub const CLSID_THUMBNAIL_PROVIDER: GUID = GUID::from_u128(0x7a3c9d52_4e1b_4f6a_9c2d_5b8e0f1a6d37);

    static MODULE: AtomicIsize = AtomicIsize::new(0);

    pub fn module_handle() -> isize {
        MODULE.load(Ordering::SeqCst)
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn DllMain(hinst: HINSTANCE, reason: u32, _reserved: *mut c_void) -> BOOL {
        const DLL_PROCESS_ATTACH: u32 = 1;
        if reason == DLL_PROCESS_ATTACH {
            MODULE.store(hinst.0 as isize, Ordering::SeqCst);
        }
        BOOL(1)
    }

    /// # Safety
    /// Appelée par COM avec des pointeurs valides (ou nuls, qui sont refusés).
    #[unsafe(no_mangle)]
    pub unsafe extern "system" fn DllGetClassObject(rclsid: *const GUID, riid: *const GUID, ppv: *mut *mut c_void) -> HRESULT {
        if rclsid.is_null() || riid.is_null() || ppv.is_null() {
            return E_INVALIDARG;
        }
        // SAFETY : pointeurs vérifiés ci-dessus.
        unsafe {
            *ppv = std::ptr::null_mut();
            if *rclsid != CLSID_THUMBNAIL_PROVIDER {
                return CLASS_E_CLASSNOTAVAILABLE;
            }
            let factory: IClassFactory = com::Factory.into();
            factory.query(riid, ppv)
        }
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn DllCanUnloadNow() -> HRESULT {
        if com::LIVE.load(Ordering::SeqCst) <= 0 { S_OK } else { S_FALSE }
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn DllRegisterServer() -> HRESULT {
        match register::install() {
            Ok(()) => S_OK,
            Err(_) => E_FAIL,
        }
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn DllUnregisterServer() -> HRESULT {
        match register::uninstall() {
            Ok(()) => S_OK,
            Err(_) => E_FAIL,
        }
    }
}

#[cfg(windows)]
pub use exports::*;
