# Test files

Real files used by the integration tests in `tests/` (they are located through `CARGO_MANIFEST_DIR`, so `cargo test`
works from any machine). They are small samples kept only to check that the readers and converters handle real data.

| Folder | Content | Used by |
|---|---|---|
| `fallout/` | FRM (still image, 6-direction animation), RIX splash screen | round trips, GIF/ART/FRM conversions, palette export, CLI |
| `arcanum/` | two ART files | round trips |
| `psd/` | three PSD files and a PNG | PSD reader (compared with the PNG), PNG reader |
| `tactics/zar`, `tactics/spr`, `tactics/til` | Fallout Tactics ZAR, SPR and TIL files | Fallout Tactics readers, corruption tests |

The files come from the games' own data (Fallout, Arcanum, Fallout Tactics and mods) and from the author's work files; they
remain the property of their respective owners and are not covered by the licence of this project. Remove a file only
together with the tests that use it.
