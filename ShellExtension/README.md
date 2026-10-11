# Explorer thumbnails for Fallout files

A Windows **thumbnail provider** (COM DLL) that makes Explorer show the picture of `.FRM` and `.RIX` files, in the folder
views and in the preview pane's thumbnail. It is a separate Rust project (its own `Cargo.toml`, `Cargo.lock` and `target`
folder) that reuses the decoders of Fallout Image Browser through `fallout_image_browser = { path = ".." }`.

## Install / uninstall

For the current user only, no administrator rights:

```powershell
.\install.ps1        # builds the DLL, copies it to %LOCALAPPDATA%\FalloutImageBrowser\ShellExtension and registers it
.\uninstall.ps1      # unregisters it (add -RemoveFiles to delete the DLL too)
```

Or by hand: `regsvr32 fallout_thumbnails.dll` and `regsvr32 /u fallout_thumbnails.dll`. Registration writes the COM class
under `HKEY_CURRENT_USER\Software\Classes\CLSID` and links `.frm` and `.rix` through
`HKEY_CURRENT_USER\Software\Classes\.frm\ShellEx\{E357FCCD-A995-4576-B01F-234630154E96}` (and the same for `.rix`).
Unregistering removes exactly these keys, plus the parent keys that registration created if they are empty.

Thumbnails already cached by Windows are replaced when the files change or the cache is rebuilt; if an old icon stays,
restart Explorer or sign out and in. Explorer's thumbnail host keeps the DLL loaded for a while, so replacing the DLL
right after an uninstall may need a retry (the scripts do that).

## What it shows

* **RIX**: the whole picture. **FRM**: the first frame of the first direction, with the Fallout `default.pal` palette
  (index 0 is transparent). A same-name `.PAL` file next to the FRM is *not* used, because a thumbnail provider only
  receives the file's content, not its location.
* The picture is scaled down (box filter, premultiplied alpha) so that its larger side fits the requested size; small
  images are never enlarged.
* Files that cannot be decoded give no thumbnail (Explorer shows the usual icon): errors and even panics are caught
  before they reach Explorer.

## How it works

| File | Role |
|---|---|
| `src/thumbnail.rs` | decodes the bytes (FRM, RIX) with the main project's readers and builds the scaled BGRA bitmap data; no Windows code, tested everywhere |
| `src/com.rs` | the COM objects: `IInitializeWithStream` + `IThumbnailProvider`, and the class factory |
| `src/register.rs` | the registry keys (functions take the root key as a parameter so tests never touch real associations) |
| `src/lib.rs` | DLL entry points: `DllGetClassObject`, `DllCanUnloadNow`, `DllRegisterServer`, `DllUnregisterServer`, `DllMain` |

## Build and test

```powershell
cargo build --release          # target\release\fallout_thumbnails.dll (about 1.4 MB)
cargo test --release
```

The tests cover the rendering on the repository's sample files (`..\_test_files`), the COM object driven the way
Explorer drives it (stream → `Initialize` → `GetThumbnail` → bitmap), and the registration in a temporary registry branch.

The DLL must match Explorer's architecture: build with `--target aarch64-pc-windows-msvc` for an ARM64 Windows, which needs
the Visual Studio "C++ ARM64 build tools" component.
