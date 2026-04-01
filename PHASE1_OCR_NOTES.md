# Phase 1: Server-Side OCR -- Implementation Notes

## What was done

Added server-side OCR that auto-recognizes handwritten annotations after submit, populating `recognized_text` on each `AnnotationGroup`.

### Files changed

| File | Change |
|------|--------|
| `flake.nix` | Added tesseract, leptonica, libclang/bindgen deps; `TESSDATA_PREFIX` env; `makeWrapper` for runtime tessdata; `LD_LIBRARY_PATH` for dev shell |
| `server/Cargo.toml` | Added `leptess = "0.14"`, `image = "0.25"` |
| `server/src/ocr.rs` | **New** -- `OcrEngine` wrapping leptess, stroke-to-PNG renderer, `ocr_annotation_groups()` |
| `server/src/lib.rs` | Added `pub mod ocr` |
| `server/src/app.rs` | `OcrEngine` in `AppState`; spawns async OCR after submit; graceful fallback if tesseract unavailable |
| `server/src/session.rs` | Added `SessionManager::update_annotations()` for post-OCR update |

### How it works

1. On submit, after session transitions to `Submitted`, a `tokio::spawn` task runs OCR
2. For each `AnnotationGroup` missing `recognized_text`:
   - Strokes (`Vec<Vec<[f64; 2]>>`) are rendered to a cropped PNG (white bg, black strokes)
   - Tesseract OCRs the PNG
   - Result stored in `recognized_text`
3. Session is persisted again with the OCR results
4. OCR is best-effort: if tesseract is unavailable, server starts without it (logs warning)

### Testing

```bash
nix develop --command bash -c "cd server && cargo test --lib ocr"   # 5 OCR tests
nix develop --command bash -c "cd server && cargo test"              # full suite
nix build .#eink-bridge                                              # nix package
```

All pass. The nix-built binary wraps each executable with `TESSDATA_PREFIX` pointing at the nix-store tesseract data.

### What's next (Phase 2+)

See the plan at `.claude/plans/zany-juggling-twilight.md`. Next: WebSocket endpoint + iterative state machine, then CLI `--interactive` mode, then `/eink` skill update with iterative loop.

### Known limitations

- OCR runs after the long-poll notify, so the CLI may get the result before OCR completes on the first poll. The OCR'd text will be available on subsequent reads of `/api/sessions/{id}/result`.
- Handwriting OCR quality depends on stroke clarity. Pre-processing (thresholding, dilation) could improve results later.
- The `image` crate pulls in a lot of optional codecs. Could slim this down with feature flags if binary size matters.
