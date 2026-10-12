//! Objets COM : le fournisseur de vignettes (`IThumbnailProvider`) et sa fabrique de classes.
//!
//! L'Explorateur exécute les fournisseurs de vignettes dans un processus isolé, où seul `IInitializeWithStream` est pris en
//! charge (les autres interfaces d'initialisation exigeraient de désactiver cette isolation). Le fournisseur lit donc le
//! flux et en retire le nom du fichier (`IStream::Stat`) : l'extension distingue les formats sans signature (ART), et un
//! chemin complet permet de retrouver un `.PAL` du même nom.

use crate::thumbnail;
use std::cell::RefCell;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicI32, Ordering};
use windows::Win32::Foundation::{CLASS_E_NOAGGREGATION, E_FAIL, E_UNEXPECTED};
use windows::Win32::Graphics::Gdi::{CreateBitmap, HBITMAP};
use windows::Win32::System::Com::{CoTaskMemFree, IClassFactory, IClassFactory_Impl, IStream, STATFLAG_DEFAULT, STATSTG, STREAM_SEEK_SET};
use windows::Win32::UI::Shell::PropertiesSystem::{IInitializeWithStream, IInitializeWithStream_Impl};
use windows::Win32::UI::Shell::{IThumbnailProvider, IThumbnailProvider_Impl, WTS_ALPHATYPE, WTSAT_ARGB};
use windows::core::{BOOL, IUnknown, Interface, Ref, Result, implement};

/// Nombre d'objets et de verrous en vie : `DllCanUnloadNow` ne répond « oui » que s'il est nul.
pub static LIVE: AtomicI32 = AtomicI32::new(0);

/// Lit tout le flux (au plus `MAX_INPUT` octets).
fn read_all(stream: &IStream) -> Result<Vec<u8>> {
    // SAFETY : tampons valides pendant les appels ; le flux est fourni par l'Explorateur.
    unsafe {
        let _ = stream.Seek(0, STREAM_SEEK_SET, None);
        let mut data = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let mut read = 0u32;
            stream.Read(buf.as_mut_ptr() as *mut c_void, buf.len() as u32, Some(&mut read)).ok()?;
            if read == 0 {
                break;
            }
            data.extend_from_slice(&buf[..read as usize]);
            if data.len() > thumbnail::MAX_INPUT {
                return Err(E_FAIL.into());
            }
        }
        Ok(data)
    }
}

/// Nom du fichier tel que le flux le déclare (nom seul ou chemin complet, selon l'origine du flux), s'il en a un.
fn stream_name(stream: &IStream) -> Option<String> {
    // SAFETY : `STATSTG` est rempli par COM ; le nom alloué est copié puis libéré.
    unsafe {
        let mut stat = STATSTG::default();
        stream.Stat(&mut stat, STATFLAG_DEFAULT).ok()?;
        if stat.pwcsName.is_null() {
            return None;
        }
        let name = stat.pwcsName.to_string().ok();
        CoTaskMemFree(Some(stat.pwcsName.0 as *const c_void));
        name.filter(|n| !n.is_empty())
    }
}

/// Ce que le fournisseur a reçu.
struct Input {
    data: Vec<u8>,
    name: Option<String>,
}

#[implement(IInitializeWithStream, IThumbnailProvider)]
pub struct Provider {
    input: RefCell<Option<Input>>,
}

impl Provider {
    pub fn new() -> Self {
        LIVE.fetch_add(1, Ordering::SeqCst);
        Provider { input: RefCell::new(None) }
    }
}

impl Default for Provider {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Provider {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

impl IInitializeWithStream_Impl for Provider_Impl {
    fn Initialize(&self, pstream: Ref<'_, IStream>, _grfmode: u32) -> Result<()> {
        let stream = pstream.ok()?;
        let name = stream_name(stream);
        *self.input.borrow_mut() = Some(Input { data: read_all(stream)?, name });
        Ok(())
    }
}

impl IThumbnailProvider_Impl for Provider_Impl {
    fn GetThumbnail(&self, cx: u32, phbmp: *mut HBITMAP, pdwalpha: *mut WTS_ALPHATYPE) -> Result<()> {
        if phbmp.is_null() || pdwalpha.is_null() {
            return Err(E_FAIL.into());
        }
        let input = self.input.borrow();
        let input = input.as_ref().ok_or(E_UNEXPECTED)?;
        // Un décodeur qui panique ne doit jamais remonter jusque dans l'Explorateur.
        let thumb = catch_unwind(AssertUnwindSafe(|| thumbnail::render_named(&input.data, input.name.as_deref(), cx))).map_err(|_| E_FAIL)?.map_err(|_| E_FAIL)?;
        // SAFETY : `bgra` contient width × height pixels de 4 octets ; les pointeurs de sortie ont été vérifiés.
        unsafe {
            let hbmp = CreateBitmap(thumb.width as i32, thumb.height as i32, 1, 32, Some(thumb.bgra.as_ptr() as *const c_void));
            if hbmp.is_invalid() {
                return Err(E_FAIL.into());
            }
            *phbmp = hbmp;
            *pdwalpha = WTSAT_ARGB;
        }
        Ok(())
    }
}

#[implement(IClassFactory)]
pub struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(&self, punkouter: Ref<'_, IUnknown>, riid: *const windows::core::GUID, ppvobject: *mut *mut c_void) -> Result<()> {
        if punkouter.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let unknown: IUnknown = Provider::new().into();
        // SAFETY : `riid` et `ppvobject` sont fournis par l'appelant COM.
        unsafe { unknown.query(riid, ppvobject).ok() }
    }

    fn LockServer(&self, flock: BOOL) -> Result<()> {
        if flock.as_bool() {
            LIVE.fetch_add(1, Ordering::SeqCst);
        } else {
            LIVE.fetch_sub(1, Ordering::SeqCst);
        }
        Ok(())
    }
}
