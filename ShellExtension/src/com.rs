//! Objets COM : le fournisseur de vignettes (`IThumbnailProvider`) et sa fabrique de classes.

use crate::thumbnail;
use std::cell::RefCell;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicI32, Ordering};
use windows::Win32::Foundation::{CLASS_E_NOAGGREGATION, E_FAIL, E_UNEXPECTED};
use windows::Win32::Graphics::Gdi::{CreateBitmap, HBITMAP};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl, IStream, STREAM_SEEK_SET};
use windows::Win32::UI::Shell::PropertiesSystem::{IInitializeWithStream, IInitializeWithStream_Impl};
use windows::Win32::UI::Shell::{IThumbnailProvider, IThumbnailProvider_Impl, WTSAT_ARGB, WTS_ALPHATYPE};
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

#[implement(IInitializeWithStream, IThumbnailProvider)]
pub struct Provider {
    data: RefCell<Option<Vec<u8>>>,
}

impl Provider {
    pub fn new() -> Self {
        LIVE.fetch_add(1, Ordering::SeqCst);
        Provider { data: RefCell::new(None) }
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
        *self.data.borrow_mut() = Some(read_all(stream)?);
        Ok(())
    }
}

impl IThumbnailProvider_Impl for Provider_Impl {
    fn GetThumbnail(&self, cx: u32, phbmp: *mut HBITMAP, pdwalpha: *mut WTS_ALPHATYPE) -> Result<()> {
        if phbmp.is_null() || pdwalpha.is_null() {
            return Err(E_FAIL.into());
        }
        let data = self.data.borrow();
        let data = data.as_ref().ok_or(E_UNEXPECTED)?;
        // Un décodeur qui panique ne doit jamais remonter jusque dans l'Explorateur.
        let thumb = catch_unwind(AssertUnwindSafe(|| thumbnail::render(data, cx))).map_err(|_| E_FAIL)?.map_err(|_| E_FAIL)?;
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
