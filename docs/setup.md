# OCR and voice setup

## OCR

Handwriting recognition uses **Ollama** with `qwen2.5vl:7b` (vision model). Tesseract is not used.

### Prerequisites

1. Install Ollama — on NixOS add to `configuration.nix`:
   ```nix
   services.ollama = {
     enable = true;
     package = pkgs-unstable.ollama;
     environmentVariables = {
       OLLAMA_NUM_PARALLEL = "4";  # allow concurrent annotation OCR
     };
   };
   ```
   Then `sudo nixos-rebuild switch`.

2. Pull the model (one-time):
   ```bash
   ollama pull qwen2.5vl:7b
   ```

3. Verify it's running:
   ```bash
   curl http://localhost:11434/api/ps
   ```

### Configuration

| Env var | Default | Description |
|---|---|---|
| `EINK_OLLAMA_URL` | `http://localhost:11434` | Ollama base URL |
| `EINK_OLLAMA_MODEL` | `qwen2.5vl:7b` | Model name |
| `EINK_OCR_DEBUG` | unset | If set, saves PNG to `/tmp/eink-ocr-*.png` |
| `EINK_OCR_TIMEOUT_SECONDS` | `120` (max 300) | Bound on one local OCR attempt |
| `EINK_OCR_INTERACTIVE_TIMEOUT_SECONDS` | `20` (max 120) | The shorter bound for a call a reader is waiting on |

Both bounds fall back to their default on an unusable value; a `0` would abandon every
attempt before it was sent, which reads as "the model never answers".

### Timing

Each OCR call logs the provider that won, whether the cache answered, and wall time at
`INFO`; prompt-eval and token-eval timings are at `DEBUG`:
```
INFO OCR complete provider="ollama" cache_hit=false wall_ms=1823
```

Typical latency on CPU with `qwen2.5vl:7b`: 1–4 s per annotation group.

### OCR handwriting tests

```bash
cargo test --test ocr_handwriting_test -- --ignored
```

Tests use PNG fixtures cropped from a real Boox tablet screenshot (`tests/fixtures/ocr/`).
They require Ollama to be running with the vision model loaded.
A `tokio::sync::Mutex` serialises the calls so concurrent tests don't saturate the model.
Once `OLLAMA_NUM_PARALLEL=4` is active in the NixOS service, parallelism is handled by Ollama itself.

### Gotcha: coloured ink

The OCR engine renders strokes as **black on white** before sending to Ollama. The test
fixtures were converted to greyscale for the same reason — Ollama's vision model returns
empty output for brightly coloured (e.g. pink) handwriting.

## Voice

Spoken notes use **whisper.cpp** for speech to text and **piper** to read a note back.
Both run on this machine; the hosted recogniser is an opt-in fallback.

```bash
nix profile add nixpkgs#whisper-cpp nixpkgs#piper-tts
mkdir -p ~/.local/share/eink-bridge/models && cd ~/.local/share/eink-bridge/models
curl -LO https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin   # multilingual
curl -LO https://huggingface.co/rhasspy/piper-voices/resolve/main/pl/pl_PL/darkman/medium/pl_PL-darkman-medium.onnx
curl -LO https://huggingface.co/rhasspy/piper-voices/resolve/main/pl/pl_PL/darkman/medium/pl_PL-darkman-medium.onnx.json
curl -LO https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx
curl -LO https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx.json
systemctl --user enable --now whisper-asr   # ~/.config/systemd/user/whisper-asr.service, port 8090
```

| Env var | Default | Description |
|---|---|---|
| `EINK_ASR_URL` | `http://127.0.0.1:8090` | whisper-server; empty disables voice |
| `EINK_ASR_MODEL` | `small` | Label recorded in the note's provenance |
| `EINK_ASR_LANGUAGE` | `auto` | `auto` detects per capture and pins the first answer; `pl`, `en` or `pl,en` pins the first of the list |
| `EINK_ASR_TIMEOUT_SECONDS` | `30` (max 120) | Bound on one transcription |
| `EINK_ASR_OPENAI_ENABLED` | unset | `1` adds OpenAI `whisper-1` as the fallback (same key as OCR) |
| `EINK_VOICE_LLM_PROVIDER` | `openai` when a key is configured, else `ollama` | Which model rewrites a spoken edit |
| `EINK_VOICE_LLM_URL` / `EINK_VOICE_LLM_MODEL` | `https://api.openai.com/v1` + `gpt-4.1-mini`, or the OCR Ollama | The cleaner for edits, and its model |
| `EINK_VOICE_LLM_ENABLED` | unset | `0` turns the cleaner off; the deterministic verbs still work |
| `EINK_PIPER_VOICE` | unset | An `.onnx` voice; set it to enable `POST .../speech` |
| `EINK_PIPER_BIN` | `piper` | The binary, when it is not on the service's PATH |

The service gets the piper variables from
`~/.config/systemd/user/eink-serve.service.d/voice.conf`. The model is the **multilingual**
`small`, not `small.en`: the same tablet is used in Polish and English, and an English-only
model cannot be asked for anything else. Accuracy against English still saturates from tiny
to medium and the errors that remain are lexical, which the document vocabulary repairs
deterministically — see [`voice-comments.md`](voice-comments.md).

The cleaner that rewrites a spoken edit runs on the **hosted** model when a key is present,
because it sits on the reader's critical path: a rewrite took 2.0 s through `gpt-4.1-mini`
and 9.9 s through the house `qwen2.5vl:7b` on the same note. Neither is believed until
`clean::guard` has checked that it invented no word. The initial note never touches a model
at all — `clean::clean` is rules.

Partials are transcribed off the websocket's receive loop, one at a time, and stop after
20 s of audio; transcribing inline used to stop the audio being read for the length of the
call, which is what made the tray look frozen. Expect the first words about 2 s after the
reader starts talking: whisper encodes a 30 s window whatever the clip length, so ~1.5 s is
the floor per answer on this CPU.
