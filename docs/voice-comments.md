# Voice Comments

Touch a block, speak, and the words become a note attached to that block — arriving
at the agent as prose it can act on, not as a transcript it has to decode.

> Status: built, all five phases. The server half is `server/src/voice/` (`capture` is the
> socket, `clean` the contract, `asr`, `llm`, `tts`), the page half is the voice card in
> `element-map.js`/`eink.css`, the tablet half is `VoiceCapture.kt` and the tray in
> `MainActivity`. Where the build departs from the text below: the card is not in the
> element map's `QUERY` (a card inserted above a bound block would renumber every block
> under it); the cleanup rules are deterministic and only `shorter`, `make it a question`
> and a free instruction reach a model, through the guard; `longer` returns to the full
> deterministic clean rather than asking the model; the wire format is in `docs/api.md`
> "Voice". Setup is in [`setup.md`](setup.md).

> Vocabulary: this is speech **to** text. The other direction (text to speech) is the
> `play` control in Phase E; everything below is voice input. Calling the whole thing
> "TTS" will make the code searchable in the wrong place.

## Three stages, three owners

```
  tap ──► capture ──► clean ──► present
            │          │          │
         tablet     server      page
        (native)   (Ollama /   (WebView,
                    OpenAI)     once)
```

Each stage lives where the hardware is:

- **Capture** is native, because a partial transcript must grow without repainting the
  document. A native strip can be refreshed on its own (`EpdController.enableA2ForSpecificView`);
  a WebView cannot refresh a region, so streaming into the page would ghost the page under
  the reader's eyes.
- **Clean** is on the server, for the same reasons OCR is: Boox is AOSP without Google
  Play services, so `SpeechRecognizer` is not reliably there, and the server already owns
  the one model, the cache, and `EINK_OPENAI_API_KEY_FILE` discipline.
- **Present** is in the page, because the page owns the design.

## How good is the local recogniser?

Measured on amd-pc (Ryzen 9 7950X, 32 threads, no GPU), `whisper.cpp` 1.8.4, model loaded
once and 24 threads per request. Ten dictated comments written the way someone talks at a
tablet — filler, a self-correction, identifiers from this repository. Audio was synthesised
(piper `lessac-medium`), which is cleaner than a real room and a real mouth, so treat the
error rate as a floor.

| model | WER | ms per 4.6 s clip | realtime factor |
|---|---|---|---|
| tiny.en | 8.0% | 285 | 0.06 |
| base.en | 7.3% | 285 | 0.06 |
| **small.en** | **8.0%** | **847** | **0.18** |
| medium.en | 6.6% | 8 361 | 1.79 |
| large-v3-turbo q5 | 10.2% | 3 224 | 0.70 |

Two things follow, and only one of them is about the model.

**Accuracy saturates.** Everything from tiny to medium lands within 1.4 points of each
other, and the largest model was *worse* than base because it "corrected" more
aggressively (`EINK_TOKEN` became `INC_tokencheck`). A 30 s comment costs 5 s with
small.en and 54 s with medium. The model is not the bottleneck.

**The model is multilingual, and the language is detected.** `small.en` cannot be asked
for Polish, and the reader is not going to be asked which language they are about to think
in. The shipped model is `small` (multilingual) with `-l auto`; the same clips come back as
`polish` and `english` respectively, and `EINK_ASR_LANGUAGE=pl` pins it when detection
keeps flipping on short clips. Cost: about 1.5 s per clip whatever its length, because
whisper encodes a 30 s window either way — that is the floor on how fast a partial can
arrive, and the reason partials stop after 20 s of audio.

Measured on the Polish side, two dictated notes through `small` multilingual:
`Tak, ta kolumna cachą jest niepotrzebna, sam indyks wystarczy.` — the domain words are
wrong exactly as they are in English ("cache", "indeks"), which is what the vocabulary pass
is for; the sentence structure, punctuation and language were right.

**The errors are lexical, and the same at every size.** What breaks is identifiers:
`reanchor.rs` → "re-anchor.RS", `EINK_TOKEN` → "INK underscore token" or "EINC_ token",
`lasso_bbox` → "Lasso B-Box", `WebSocket` → "web socket". Prose is nearly perfect. So the
money is in the vocabulary pass, not in more parameters — see below.

Cloud is therefore not the default: local `small.en` at 0.18× realtime is fast enough for
per-phrase streaming and keeps a comment about an unfinished PR on the machine. Keep the
hosted path (`EINK_OPENAI_API_KEY_FILE`, same opt-in as OCR) for a noisy room or a speaker
the model keeps mangling, and record which provider answered in the payload.

### Setup on amd-pc

```bash
nix profile add nixpkgs#whisper-cpp
mkdir -p ~/.local/share/eink-bridge/models
curl -L -o ~/.local/share/eink-bridge/models/ggml-small.bin \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin
```

`~/.config/systemd/user/whisper-asr.service` — resident, loopback only, 16 threads so the
e-ink server still gets a core:

```ini
[Service]
ExecStart=%h/.nix-profile/bin/whisper-server \
    -m %h/.local/share/eink-bridge/models/ggml-small.bin \
    --host 127.0.0.1 --port 8090 -t 16 -l auto --suppress-nst --convert
Restart=on-failure
[Install]
WantedBy=default.target
```

`--convert` is what lets the tablet post a recorded file directly instead of the server
having to guess at a container. The provider then looks exactly like the OCR ones:
`EINK_ASR_URL` (default `http://127.0.0.1:8090`) with a hosted fallback behind
`EINK_ASR_OPENAI_ENABLED`.

## One vocabulary for notes

Voice does not invent an anchoring mechanism. It reuses the tap.

| gesture | anchor | mark on the block | badge |
|---|---|---|---|
| pen strokes near a block | proximity / lasso | left border, group colour | ✏ |
| tap a block | explicit | 4px outline + 8% tint | ⊕ |
| speak at a block | explicit (from the tap) | outline + left-ruled card below | number |

A voice note binds to the result of `__einkTapElement`. Nothing changes in the element map,
nothing changes on the Android anchoring path.

The card, in the page:

```html
<aside class="voice-block" data-note="1" data-severity="note">
  <span class="eink-link-badge">1</span>
  <span class="voice-state">spoken · 6s</span>
  <span class="voice-text">Drop the cache column; the index is enough.</span>
  <details class="voice-raw"><summary>heard</summary>
    so um drop the cache column the index is enough
  </details>
</aside>
```

`<aside>`, not `<p>`: a `<details>` inside a paragraph makes the parser close the
paragraph early, and the note would render as two unrelated blocks.

`.voice-block` takes the `blockquote` left rule in the group's colour — the same rule
`applyStrokeStyle` already draws — with `.eink-link-badge` reused unchanged for the
numeral. Paper white, Georgia 18px, no shadow, no radius, no transition: the house style.

**The numeral is load-bearing, not decoration.** Most Boox panels are greyscale, where
`#e74c3c` and `#2196f3` are the same grey. The number is the only thing that tells two
notes apart, and it is also the address voice edits use ("drop two"). Colour is the bonus
on a panel that can show it.

## The tray

A native strip docked above the toolbar while capturing. Paper white, `2px solid #111`
on top, Georgia, full width, at most a third of the screen.

```
┌──────────────────────────────────────────────────────────┐
│ ● listening 0:07            [Done] [Undo] [Discard]      │
│ so um drop the cache column and  the index               │
│ is enough                                                │
│ [Shorter] [As a list] [Blocker]                          │
└──────────────────────────────────────────────────────────┘
```

Three decisions in that sketch:

- **Words, not glyphs.** A microphone icon at 30px on a greyscale panel reads as a blob.
  `Speak` does not. Every control in the tray is a word.
- **No waveform.** A level meter is a full-row repaint at 30 Hz; on e-ink that is a grey
  smear and a slow page. Elapsed seconds, once a second, in a fixed-width field, is honest
  and costs one small region.
- **The text is capped at four lines and scrolls in place.** It never grows the page or the
  tray while you are speaking.

Why the tray streams at all when whisper is not a streaming model: partials come from
re-transcribing a growing buffer every ~500 ms. That means the last word appears roughly
0.7 s after it is said. The tray says `listening`, never `live`, because the difference is
something the reader will notice within one sentence.

## The cleanup contract

The transcript is an instruction to an agent. A messy one makes the agent guess; a
tidy one that says something the reader did not say makes the agent act on a fiction.
Seven rules, each a fixture:

1. **Nothing is added.** Every content word in the cleaned note appears in the raw.
   Not trusted — checked (below).
2. **Filler goes.** "um", "uh", "like", "you know", "I mean".
3. **A self-correction keeps the correction, drops what it replaced.** "the second one —
   no, the third" → "the third".
4. **An ambiguous correction keeps both, in order.** "…the second → or the third?" —
   guessing which side was meant is refused, for the same reason `reanchor.rs` refuses to
   place a note it is not sure of.
5. **Identifiers keep the document's spelling.** Tokens are matched against the rendered
   text and the element map, so "re-anchor dot rs" becomes `reanchor.rs` and "eink token"
   becomes `EINK_TOKEN`. The document is the vocabulary; no model call needed.

   The repair must be **span-local and narrow**, and the trial below is why: a pass that
   replaced phrases by fuzzy match turned "the submit path" into "the submit" and
   "EINK_TOKEN" into "EINK_TOKEN_Token", losing a word and doubling one. So the rule is:
   rewrite a span only when the spoken form resolves to a *term of art* in the document
   (an identifier, a file name, a protocol name) and the spoken form is not itself a
   plausible reading of the sentence. Prose is left alone even when a document term is
   one edit away — "cash column" stays wrong rather than being guessed into "cache
   column", and the agent reads a note that is odd but honest.
6. **Hedges stay.** "I think", "maybe", "not sure" are signal for an agent, not noise.
7. **No summarising, no translating, no reordering** — unless the speaker asked. A cleaned
   note that comes back at half length was summarised, which is a rule-7 violation.

### The guard

Before the cleaned text replaces the raw, a content-word check runs: the cleaned note's
content words must be a subset of the raw's, modulo identifier normalisation and stop
words. If the cleaner invented a word, **the raw wins** and the card says so.

Deliberately blunt. An LLM that turns "add a cache column" into "add a caching layer with
Redis" has told the agent to build something nobody asked for, and the reader will not be
re-reading a comment on a 10" panel to catch it.

## Editing by voice

A closed verb set, not an instruct-anything box. Bounded verbs dispatch deterministically,
can be tested, and cannot surprise the reader:

| said | effect |
|---|---|
| `shorter` / `longer` | re-clean with a length instruction |
| `drop two` / `drop the last one` | remove sentence *n* |
| `add …` | a new capture turn, appended |
| `turn that into a list` | sentence-split into `- ` items |
| `make it a question` | rewrite as a question |
| `mark it a blocker` | `severity: blocker`, card switches to the red rule |
| `undo` | pop the edit stack |
| `done` / `send` | accept into the session |

Three of the verbs — `shorter`, `make it a question`, a free instruction — need a model.
It is the hosted one when a key is configured, because the reader is standing at the tablet
waiting: 2.0 s through `gpt-4.1-mini` against 9.9 s through the house `qwen2.5vl:7b` on the
same Polish note, both still guarded against the raw transcript. The initial note never
touches a model: `clean::clean` is rules, and that is what makes the text the agent reads
predictable.

Anything outside the set is "clean with this instruction" — unbounded, so it previews with
`keep` / `undo` and is never applied silently. Only the closed verbs apply immediately.

The pen still has one job here and does it better: **a strike through a sentence deletes
it.** Unambiguous, no round trip, no words — and it already reads as deletion, because that
is what a `diff` block looks like. Voice adds, the pen strikes.

## Wire

`GET /api/sessions/{id}/voice` — a WebSocket of its own, not the observer socket. The
observer socket carries session lifecycle to the agent; putting a 20 ms audio cadence on
the channel that also delivers `submitted` couples two unrelated clocks.

Binary frames up: 16 kHz mono PCM, 20 ms chunks.
Text frames down:

```json
{"type":"partial","seq":41,"text":"so um drop the cache"}
{"type":"final","raw":"…","text":"…","seconds":6.2}
{"type":"edit","instruction":"shorter","text":"…","seq":42}
{"type":"error","message":"no speech recogniser configured"}
```

Bounds, in the shape the rest of the API uses: 5 minutes and 4 MB per capture, one open
capture per session (a second is `409`, like a second submit), a 1.5 s silence gate so
nobody has to press stop, and `seq` on every frame so a late partial cannot overwrite a
final.

## Payload

A voice note is an `AnnotationGroup` carrying the tap's anchor, so `eink-review result`
and the `/eink` skill need no change:

```rust
pub struct AnnotationGroup {
    pub anchor: Option<AnnotationAnchor>,
    pub strokes: Vec<Vec<[f64; 2]>>,
    pub recognized_text: Option<String>,   // the cleaned comment
    /// How these words got here. A pen stroke run through OCR and a spoken note are
    /// both `recognized_text`; an agent that needs to know which is reading this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<NoteSource>,        // "pen" | "tap" | "voice"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<VoiceNote>,          // only when source == voice
}

pub struct VoiceNote {
    /// Exactly what was heard. Never rewritten, never dropped.
    pub raw: String,
    /// The instructions applied to get from `raw` to `recognized_text`, in order.
    pub edits: Vec<VoiceEdit>,             // {instruction, before, after}
    /// Every identifier the vocabulary pass rewrote, with what it heard. An agent acting
    /// on `reanchor.rs` should be able to see that the reader said "re-anchor rs".
    pub repairs: Vec<Repair>,              // {heard, wrote}
    /// The worst segment confidence in the note, 0..1. Low means "the recogniser was
    /// unsure of some of this" — worth a screenshot request or a question before acting.
    pub confidence: f32,
    /// Which recogniser answered: {provider, model, language}.
    pub stt: SttProvenance,
    pub seconds: f32,
    /// Session audio route, for "what did they actually say".
    pub audio: Option<String>,
}
```

`recognized_text` stays the field the agent reads, so the formatter keeps working. `raw`
is there for the case the guard lets through and the sentence still reads wrong.

`source` is what the reader asked for and it earns its place: **a spoken "two" may be the
word, a written "2" may be the numeral**, and "the second example" said aloud is a
reference the agent should not read as a number. It also tells the agent whether a note
can be re-recognised — a pen note can be re-OCR'd from its strokes, a voice note cannot be
re-recognised from its text, only replayed from its audio.

## Seeing the page

A spoken note lands in a margin the reader is looking at and the agent is not. When a note
binds somewhere surprising, or the reader says "that's not where I meant", the agent can
ask the tablet what it is showing — `eink-review screenshot <id>`, which composites the
reader's ink over the page as it is scrolled and zoomed. That exists (server route, CLI
command, skill section, `docs/api.md` "Screenshots"), and it already has the two answers a
voice note will need: "no tablet is on this session" at once rather than after a timeout,
and "the tablet could not capture" with the reason. A low-confidence voice note should
trigger the same ask, because from the agent's side the two failures look identical.

## Phases

| | what | useful alone? |
|---|---|---|
| A | tap, record, server ASR, raw note in the payload | yes — messy words at the agent beat no words |
| B | tray, partials, silence gate, A2 refresh | yes |
| C | cleanup + guard + `heard:` toggle | yes |
| D | voice edits, closed verbs, undo stack | yes |
| E | `play` — read a note aloud (server TTS, one voice) | this is the actual TTS half |

## What can go wrong quietly

- **The note binds to the wrong block.** The tap is converted with a viewport that was
  stale after a scroll or at scale 1.333 — the same class of bug that landed ink 33% off
  the text. `__einkViewport` exists for this; a tap at 1.333 is a test.
- **A late partial overwrites the final.** `seq`, and drop partials older than the newest
  final.
- **The cleaner rewrites meaning.** The content-word guard, and `raw` on the card.
- **The tray eats the pen.** A stylus-down while the tray has focus must yield to the
  page, or the reader cannot strike a sentence.
- **Audio is kept forever.** Kept with the session and removed with it at retention, like
  images. A note without audio is still a complete note.

## Tests

| area | test |
|---|---|
| cleanup | `voice::clean` — the seven rules as fixtures |
| guard | an invented word falls back to raw; a reorder that keeps every word passes |
| contracts | voice WS handshake, partial→final, 409 on a second capture, the 4 MB bound |
| element map | `.voice-block` is in `QUERY`; `every_block_kind_can_be_bound_by_a_tap` |
| render | numeral, group colour on the left rule, `heard` collapsed, no clipping ancestor |
| android | `voiceTrayState(partial, final, edit)` as a pure function, no Android types |
| mutation | guard switched to always-accept → the invented-word test goes red |
