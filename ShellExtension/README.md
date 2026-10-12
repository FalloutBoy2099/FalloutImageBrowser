# Explorer thumbnails for Fallout files

A Windows **thumbnail provider** (COM DLL) that makes Explorer show the picture of Fallout `.FRM` and `.RIX`, Arcanum `.ART`
and Fallout Tactics `.ZAR`, `.TIL` and `.SPR` files, in the folder views and in the preview pane's thumbnail. It is a separate
Rust project (its own `Cargo.toml`, `Cargo.lock` and `target` folder) that reuses the decoders of Fallout Image Browser through
`fallout_image_browser = { path = ".." }`.

## Install / uninstall

For the current user only, no administrator rights:

```powershell
.\install.ps1        # builds the DLL, copies it to %LOCALAPPDATA%\FalloutImageBrowser\ShellExtension and registers it
.\uninstall.ps1      # unregisters it (add -RemoveFiles to delete the DLL too)
```

The release packages contain the DLL next to the scripts, so nothing has to be built.

Or by hand: `regsvr32 fallout_thumbnails.dll` and `regsvr32 /u fallout_thumbnails.dll`. Registration writes the COM class
under `HKEY_CURRENT_USER\Software\Classes\CLSID` and links the six extensions through
`HKEY_CURRENT_USER\Software\Classes\.<ext>\ShellEx\{E357FCCD-A995-4576-B01F-234630154E96}`. Unregistering removes exactly
these keys, plus the parent keys that registration created if they are empty.

Two things to know about Windows:

* Explorer runs thumbnail providers in separate `dllhost.exe` processes that stay alive for a while. One that was started
  before an update keeps the **old** DLL loaded (and locks the file), so after replacing the DLL by hand, thumbnails keep
  coming from the old version. The scripts close those processes (Windows starts a new one when needed); if you work by hand,
  end the `dllhost.exe` that has `fallout_thumbnails.dll` loaded, or sign out and in.
* Windows caches thumbnails: an image that already has a cached thumbnail (or a cached failure) keeps it until the file
  changes or the cache is rebuilt.

## What it shows

* The first frame of the first direction (FRM, ART, SPR), the whole picture (RIX, ZAR) or the tile (TIL), scaled down (box
  filter, premultiplied alpha) so that its larger side fits the requested size; small images are never enlarged. Transparent
  pixels stay transparent.
* Palette: the Fallout `default.pal`. A same-name `.PAL` next to the file is used **only when the provider is given the
  file's full path**. Explorer's isolated thumbnail host hands over just a stream (the only mode it supports; the other
  initialisation interfaces would require disabling the isolation), whose name has no folder, so in Explorer FRM thumbnails
  use the Fallout palette.
* The format is chosen from the file name the stream reports (ART has no signature), otherwise from the content.
* Files that cannot be decoded give no thumbnail (Explorer shows the usual icon): errors and even panics are caught
  before they reach Explorer.

## How it works

| File | Role |
|---|---|
| `src/thumbnail.rs` | decodes the bytes with the main project's readers and builds the scaled BGRA bitmap data; no Windows code, tested everywhere |
| `src/com.rs` | the COM objects: `IInitializeWithStream` (reads the stream and its name through `IStream::Stat`) + `IThumbnailProvider`, and the class factory |
| `src/register.rs` | the registry keys (functions take the root key as a parameter so tests never touch real associations) |
| `src/lib.rs` | DLL entry points: `DllGetClassObject`, `DllCanUnloadNow`, `DllRegisterServer`, `DllUnregisterServer`, `DllMain` |

## Build and test

```powershell
cargo build --release          # target\release\fallout_thumbnails.dll (about 1.5 MB)
cargo test --release
```

The tests cover the rendering of every supported family on the repository's sample files (`..\_test_files`), the same-name
palette, the COM object driven the way Explorer drives it (file stream → `Initialize` → `GetThumbnail` → bitmap), and the
registration in a temporary registry branch.

The DLL must match Explorer's architecture: build with `--target aarch64-pc-windows-msvc` for an ARM64 Windows, which needs
the Visual Studio "C++ ARM64 build tools" component (the GitHub workflow builds both).
