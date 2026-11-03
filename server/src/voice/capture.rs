//! One spoken note, from the first audio frame to the card.
//!
//! The tablet streams PCM on a WebSocket of its own (the observer socket carries session
//! lifecycle; a 20 ms audio cadence does not belong on the channel that delivers
//! `submitted`). The server re-transcribes the growing buffer for partials, closes the
//! capture on its own after a silence, cleans the transcript behind the guard, keeps
//! the audio with the session, and then takes the reader's edits until `done`.
use super::clean::{self, Verb};
use super::{BYTES_PER_SECOND, SAMPLE_RATE, wav};
use crate::api::{Repair, SttProvenance, VoiceEdit, VoiceNote};
use crate::app::AppState;
use crate::session::SessionStatus;
use axum::{
    Json,
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{info, warn};

/// The bounds of one capture, in the shape the rest of the API uses.
pub const MAX_SECONDS: usize = 300;
pub const MAX_BYTES: usize = 4 * 1024 * 1024;
/// How long the reader has to be quiet before the note is taken as finished.
const SILENCE_GATE_BYTES: usize = BYTES_PER_SECOND * 3 / 2;
/// Loud audio that has to accumulate before quiet counts as "stopped talking", so a
/// pen click or the tap that started the capture does not end it.
const MIN_SPEECH_BYTES: usize = BYTES_PER_SECOND / 5;
/// How much new audio justifies another partial. One transcription runs at a time, so
/// this is a floor, not a cadence: on this machine a partial takes about a second, and the
/// tray updates as fast as that allows.
const PARTIAL_EVERY_BYTES: usize = BYTES_PER_SECOND / 2;
/// The shortest clip worth transcribing at all, measured rather than guessed. Two Polish
/// sentences cut into slices and asked three times each:
///
///   1 s   russian, russian, russian   /  english, english, english
///   2 s   polish,  polish,  polish    /  polish,  polish,  polish
///   3 s   polish,  polish,  polish    /  polish,  polish,  polish
///
/// A one-second window does not carry enough speech to place the language, and the tray
/// would show a Polish speaker's own words back in the wrong alphabet. Two seconds does,
/// on both clips, every time.
const PARTIAL_FIRST_BYTES: usize = BYTES_PER_SECOND * 2;
/// Past this, a re-transcription of the whole buffer costs more than the text it adds, so
/// the tray stops updating and the final answers for everything. Measured on the house
/// machine: the multilingual model spends about 1.5 s on any clip (the encoder always
/// reads a 30 s window), so a partial is worth doing while the buffer is short, and stops
/// being worth doing once the decode behind it starts to show.
const PARTIAL_CEILING_BYTES: usize = BYTES_PER_SECOND * 20;
/// How far above the room a chunk has to be to count as speech. A ratio, not a level,
/// because a room is not a constant: a study and a kitchen with an extractor fan differ by
/// an order of magnitude, and the fixed 700 this replaced called the kitchen a
/// conversation — a capture then ran for 70 seconds with nobody in the room talking,
/// because every chunk of room tone counted as the reader speaking.
const SPEECH_MARGIN: f64 = 2.5;
/// The quietest level that can still be speech, for a room quiet enough that the ratio
/// is meaningless.
const SPEECH_FLOOR_MIN: f64 = 150.0;
/// The floor is the quietest 20 ms block of the last two seconds. Speech has gaps — between
/// words, between sentences — and a fan does not: that is the whole difference between a
/// reader and a room, and it needs no magic level.
const FLOOR_BLOCK_BYTES: usize = 640;
const FLOOR_BLOCKS: usize = 100;
/// How long a capture may run without hearing anything before it closes itself. The old
/// gate could not end an empty capture at all: it needed speech first, and in a noisy room
/// it had already decided the noise was speech.
const NO_SPEECH_BYTES: usize = BYTES_PER_SECOND * 15;
/// A tablet that sends nothing at all for this long has lost its microphone or its
/// network; the capture is abandoned rather than held open.
const IDLE: Duration = Duration::from_secs(30);

/// The silence gate as a pure decision over the bytes, so it can be tested with a tone.
///
/// It learns the room from the first chunk and keeps following it downwards, so the same
/// rule works in a silent study and next to a fan. Speech is what stands clearly above
/// that floor, not what exceeds a number somebody picked at a desk.
#[derive(Debug)]
pub struct Gate {
    loud_bytes: usize,
    quiet_bytes: usize,
    /// The last two seconds of 20 ms block levels, so the floor can be the quietest of
    /// them rather than a number somebody guessed.
    recent: std::collections::VecDeque<f64>,
}

impl Default for Gate {
    fn default() -> Self {
        Self {
            loud_bytes: 0,
            quiet_bytes: 0,
            recent: std::collections::VecDeque::with_capacity(FLOOR_BLOCKS),
        }
    }
}

impl Gate {
    pub fn heard_speech(&self) -> bool {
        self.loud_bytes >= MIN_SPEECH_BYTES
    }

    /// The room, as the quietest block of the last two seconds.
    fn floor(&self) -> f64 {
        self.recent
            .iter()
            .cloned()
            .fold(f64::INFINITY, f64::min)
            .min(1000.0)
    }

    /// The level a chunk has to beat to be speech.
    fn threshold(&self) -> f64 {
        (self.floor() * SPEECH_MARGIN).max(SPEECH_FLOOR_MIN)
    }

    /// Feed one chunk; true when the reader has been quiet long enough after speaking.
    pub fn feed(&mut self, chunk: &[u8]) -> bool {
        for block in chunk.chunks(FLOOR_BLOCK_BYTES) {
            if self.recent.len() == FLOOR_BLOCKS {
                self.recent.pop_front();
            }
            self.recent.push_back(rms(block));
        }
        let level = rms(chunk);
        if level > self.threshold() {
            self.loud_bytes += chunk.len();
            self.quiet_bytes = 0;
        } else {
            self.quiet_bytes += chunk.len();
        }
        self.heard_speech() && self.quiet_bytes >= SILENCE_GATE_BYTES
    }
}

fn rms(chunk: &[u8]) -> f64 {
    let samples = chunk.len() / 2;
    if samples == 0 {
        return 0.0;
    }
    let sum: f64 = chunk
        .chunks_exact(2)
        .map(|s| {
            let v = i16::from_le_bytes([s[0], s[1]]) as f64;
            v * v
        })
        .sum();
    (sum / samples as f64).sqrt()
}

/// One capture per session: a second is a 409, like a second submit.
pub struct CaptureSlot {
    captures: Arc<Mutex<HashSet<String>>>,
    id: String,
}

impl CaptureSlot {
    fn claim(state: &AppState, id: &str) -> Option<Self> {
        let captures = state.captures.clone();
        let claimed = captures.lock().unwrap().insert(id.to_string());
        claimed.then(|| Self {
            captures,
            id: id.to_string(),
        })
    }
}

impl Drop for CaptureSlot {
    fn drop(&mut self) {
        self.captures.lock().unwrap().remove(&self.id);
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ClientFrame {
    Stop,
    Edit {
        #[serde(default)]
        instruction: String,
    },
    Undo,
    Done,
    Discard,
}

/// The note between `final` and `done`: the text as it stands, how it got there, and
/// enough to take each step back.
struct Note {
    raw: String,
    text: String,
    /// The deterministic clean, which "longer" returns to.
    base: String,
    history: Vec<(String, String)>,
    edits: Vec<VoiceEdit>,
    repairs: Vec<Repair>,
    confidence: f32,
    stt: SttProvenance,
    seconds: f32,
    audio: Option<String>,
    severity: String,
    guarded: bool,
}

impl Note {
    fn push(&mut self, instruction: &str, after: String, severity: Option<&str>) {
        self.history
            .push((self.text.clone(), self.severity.clone()));
        self.edits.push(VoiceEdit {
            instruction: instruction.to_string(),
            before: self.text.clone(),
            after: after.clone(),
        });
        self.text = after;
        if let Some(severity) = severity {
            self.severity = severity.to_string();
        }
    }

    fn undo(&mut self) -> bool {
        match self.history.pop() {
            Some((text, severity)) => {
                self.edits.pop();
                self.text = text;
                self.severity = severity;
                true
            }
            None => false,
        }
    }

    fn payload(&self) -> VoiceNote {
        VoiceNote {
            raw: self.raw.clone(),
            edits: self.edits.clone(),
            repairs: self.repairs.clone(),
            confidence: self.confidence,
            stt: self.stt.clone(),
            seconds: self.seconds,
            audio: self.audio.clone(),
            severity: self.severity.clone(),
            guarded: self.guarded,
        }
    }
}

enum Applied {
    Changed,
    Done,
    Refused(String),
}

pub async fn voice_ws(
    ws: WebSocketUpgrade,
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    let Some(voice) = state.voice.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "no speech recogniser configured",
        )
            .into_response();
    };
    let (status, content, dir) = {
        let mgr = state.sessions.read().await;
        match mgr.get(&id) {
            Some(session) => (
                session.status.clone(),
                session.content.clone(),
                session.voice_dir(),
            ),
            None => return StatusCode::NOT_FOUND.into_response(),
        }
    };
    if status != SessionStatus::Active {
        return (StatusCode::CONFLICT, "the session is over").into_response();
    }
    let Some(slot) = CaptureSlot::claim(&state, &id) else {
        return (
            StatusCode::CONFLICT,
            "a capture is already open on this session",
        )
            .into_response();
    };
    ws.on_upgrade(move |socket| handle(socket, id, voice, content, dir, slot))
}

struct Frames {
    socket: WebSocket,
    seq: u64,
}

impl Frames {
    async fn send(&mut self, mut frame: Value) -> Result<(), ()> {
        frame["seq"] = json!(self.seq);
        self.seq += 1;
        self.socket
            .send(Message::Text(frame.to_string().into()))
            .await
            .map_err(|_| ())
    }
}

async fn handle(
    socket: WebSocket,
    id: String,
    voice: Arc<super::Voice>,
    content: String,
    dir: PathBuf,
    _slot: CaptureSlot,
) {
    let capture = uuid::Uuid::new_v4().as_simple().to_string();
    info!(session_id = %id, capture, "voice capture opened");
    let vocabulary = clean::vocabulary(&content);
    let mut frames = Frames { socket, seq: 0 };
    if frames
        .send(json!({"type":"ready","capture":capture,"stt":voice.asr.provenance()}))
        .await
        .is_err()
    {
        return;
    }
    let mut pcm: Vec<u8> = Vec::new();
    let mut gate = Gate::default();
    let mut partial_at = 0usize;
    let mut note: Option<Note> = None;
    let audio_route = format!("/api/sessions/{id}/voice/{capture}.wav");
    // The language this capture is in. Auto-detect on the first answer, then pin it: a
    // short clip between two sentences can flip the whole transcript's language, and a
    // note that changes language halfway through is worse than a wrong guess at the top.
    let configured: Option<String> = voice.asr.pinned().map(str::to_string);
    let mut partial_language: Option<String> = configured.clone();
    // Partial transcripts are transcribed off this loop. Transcribing on it stopped the
    // audio being read for the whole call, so the tablet's chunks queued behind a reply
    // about words the reader had already moved past -- at two seconds of speech the
    // partials were permanently behind and the tray looked frozen.
    let (partial_tx, mut partial_rx) = mpsc::channel::<(String, SttProvenance)>(2);
    let mut partial_in_flight = false;

    let outcome = loop {
        let message = tokio::select! {
            message = tokio::time::timeout(IDLE, frames.socket.recv()) => match message {
                Ok(Some(Ok(message))) => message,
                Ok(Some(Err(error))) => {
                    warn!(session_id = %id, %error, "voice socket error");
                    break "abandoned";
                }
                Ok(None) => break "abandoned",
                Err(_) => {
                    let _ = frames
                        .send(
                            json!({"type":"error","message":"no word from the tablet for 30 seconds"}),
                        )
                        .await;
                    break "abandoned";
                }
            },
            spoken = partial_rx.recv() => {
                partial_in_flight = false;
                // A partial that was still in flight when the final landed describes the
                // audio *before* it. Sending it now would replace the finished note with an
                // older reading of the same words.
                if note.is_some() {
                    continue;
                }
                if let Some((text, stt)) = spoken {
                    // Partials keep the language of the first one that answered, so a
                    // note does not change alphabet halfway through. The final re-detects
                    // from the whole clip, so a wrong guess here costs a partial, not the
                    // note.
                    if partial_language.is_none()
                        && stt.language.len() <= 5
                        && stt.language != "unknown"
                    {
                        partial_language = Some(stt.language.clone());
                    }
                    if !text.is_empty()
                        && frames
                            .send(json!({"type":"partial","text":text,"stt":stt}))
                            .await
                            .is_err()
                    {
                        break "abandoned";
                    }
                }
                continue;
            }
        };
        match message {
            Message::Binary(chunk) if note.is_none() => {
                let at_bound = pcm.len() + chunk.len() > MAX_BYTES
                    || (pcm.len() + chunk.len()) / BYTES_PER_SECOND >= MAX_SECONDS;
                if !at_bound {
                    pcm.extend_from_slice(&chunk);
                }
                let quiet = gate.feed(&chunk);
                if quiet || at_bound {
                    match finalise(
                        &voice,
                        &pcm,
                        &gate,
                        &vocabulary,
                        &dir,
                        &capture,
                        &audio_route,
                        configured.as_deref(),
                    )
                    .await
                    {
                        Ok(finished) => {
                            if frames.send(final_frame(&finished)).await.is_err() {
                                break "abandoned";
                            }
                            note = Some(finished);
                        }
                        Err(message) => {
                            let _ = frames.send(json!({"type":"error","message":message})).await;
                            break "failed";
                        }
                    }
                } else if !gate.heard_speech() && pcm.len() >= NO_SPEECH_BYTES {
                    let _ = frames
                        .send(json!({"type":"error","message":"nothing was heard"}))
                        .await;
                    break "failed";
                } else if gate.heard_speech()
                    && !partial_in_flight
                    && pcm.len() >= PARTIAL_FIRST_BYTES
                    && pcm.len() - partial_at >= PARTIAL_EVERY_BYTES
                    && pcm.len() <= PARTIAL_CEILING_BYTES
                {
                    partial_at = pcm.len();
                    partial_in_flight = true;
                    let audio = wav(&pcm, SAMPLE_RATE);
                    let asr = Arc::clone(&voice);
                    let tx = partial_tx.clone();
                    let spoken = partial_language.clone();
                    tokio::spawn(async move {
                        match asr.asr.transcribe(audio, spoken.as_deref()).await {
                            Ok(transcript) => {
                                let _ = tx.send((transcript.text, transcript.stt)).await;
                            }
                            Err(error) => {
                                warn!(error = %error, "partial transcription failed");
                                let _ = tx.send((String::new(), SttProvenance::default())).await;
                            }
                        }
                    });
                }
            }
            Message::Binary(_) => {}
            Message::Text(text) => {
                let frame = match serde_json::from_str::<ClientFrame>(&text) {
                    Ok(frame) => frame,
                    Err(_) => {
                        warn!(session_id = %id, "unreadable voice frame ignored");
                        continue;
                    }
                };
                match (frame, note.as_mut()) {
                    (ClientFrame::Stop, None) => {
                        match finalise(
                            &voice,
                            &pcm,
                            &gate,
                            &vocabulary,
                            &dir,
                            &capture,
                            &audio_route,
                            configured.as_deref(),
                        )
                        .await
                        {
                            Ok(finished) => {
                                if frames.send(final_frame(&finished)).await.is_err() {
                                    break "abandoned";
                                }
                                note = Some(finished);
                            }
                            Err(message) => {
                                let _ =
                                    frames.send(json!({"type":"error","message":message})).await;
                                break "failed";
                            }
                        }
                    }
                    (ClientFrame::Stop, Some(_)) => {}
                    (ClientFrame::Edit { instruction }, Some(note)) => {
                        let (applied, message) = match apply(&voice, note, &instruction).await {
                            Applied::Changed => (true, None),
                            Applied::Refused(message) => (false, Some(message)),
                            Applied::Done => {
                                let _ = frames.send(done_frame(note)).await;
                                break "done";
                            }
                        };
                        let mut frame = json!({"type":"edit","instruction":instruction,"applied":applied,
                            "text":note.text,"severity":note.severity,"edits":note.edits.len()});
                        if let Some(message) = message {
                            frame["message"] = json!(message);
                        }
                        if frames.send(frame).await.is_err() {
                            break "abandoned";
                        }
                    }
                    (ClientFrame::Undo, Some(note)) => {
                        let applied = note.undo();
                        let mut frame = json!({"type":"edit","instruction":"undo","applied":applied,
                            "text":note.text,"severity":note.severity,"edits":note.edits.len()});
                        if !applied {
                            frame["message"] = json!("nothing to undo");
                        }
                        if frames.send(frame).await.is_err() {
                            break "abandoned";
                        }
                    }
                    (ClientFrame::Done, Some(note)) => {
                        let _ = frames.send(done_frame(note)).await;
                        break "done";
                    }
                    (ClientFrame::Discard, _) => {
                        let _ = tokio::fs::remove_file(dir.join(format!("{capture}.wav"))).await;
                        break "discarded";
                    }
                    (_, None) => {
                        if frames
                            .send(json!({"type":"error","message":"nothing has been heard yet"}))
                            .await
                            .is_err()
                        {
                            break "abandoned";
                        }
                    }
                }
            }
            Message::Close(_) => break "abandoned",
            _ => {}
        }
    };
    let _ = frames.socket.send(Message::Close(None)).await;
    crate::metrics::VOICE_CAPTURES
        .with_label_values(&[outcome])
        .inc();
    info!(session_id = %id, capture, outcome, seconds = pcm.len() / BYTES_PER_SECOND, "voice capture closed");
}

fn final_frame(note: &Note) -> Value {
    json!({"type":"final","raw":note.raw,"text":note.text,"seconds":note.seconds,
        "confidence":note.confidence,"repairs":note.repairs,"guarded":note.guarded,
        "audio":note.audio,"severity":note.severity,"stt":note.stt})
}

fn done_frame(note: &Note) -> Value {
    json!({"type":"done","text":note.text,"severity":note.severity,"note":note.payload()})
}

#[allow(clippy::too_many_arguments)]
async fn finalise(
    voice: &super::Voice,
    pcm: &[u8],
    gate: &Gate,
    vocabulary: &[String],
    dir: &std::path::Path,
    capture: &str,
    audio_route: &str,
    language: Option<&str>,
) -> Result<Note, String> {
    if !gate.heard_speech() {
        return Err("nothing was heard".into());
    }
    let transcript = voice
        .asr
        .transcribe(wav(pcm, SAMPLE_RATE), language)
        .await
        .map_err(|e| format!("the recogniser failed: {e}"))?;
    let raw = transcript.text;
    if raw.is_empty() {
        return Err("nothing was heard".into());
    }
    let cleaned = clean::clean(&raw, vocabulary);
    let guarded = !clean::guard(&raw, &cleaned.text, &cleaned.repairs);
    crate::metrics::VOICE_GUARD
        .with_label_values(&[if guarded { "refused" } else { "passed" }])
        .inc();
    let text = if guarded {
        raw.clone()
    } else {
        cleaned.text.clone()
    };
    let audio = match tokio::fs::create_dir_all(dir).await {
        Ok(()) => match tokio::fs::write(dir.join(format!("{capture}.wav")), wav(pcm, SAMPLE_RATE))
            .await
        {
            Ok(()) => Some(audio_route.to_string()),
            Err(error) => {
                warn!(%error, "could not keep the audio; the note is still complete");
                None
            }
        },
        Err(error) => {
            warn!(%error, "could not keep the audio; the note is still complete");
            None
        }
    };
    Ok(Note {
        base: text.clone(),
        raw,
        text,
        history: Vec::new(),
        edits: Vec::new(),
        repairs: if guarded { Vec::new() } else { cleaned.repairs },
        confidence: transcript.confidence,
        stt: transcript.stt,
        seconds: pcm.len() as f32 / BYTES_PER_SECOND as f32,
        audio,
        severity: "note".into(),
        guarded,
    })
}

/// The closed verbs apply at once; anything else goes to the cleaner and through the
/// guard, and is refused rather than applied when it says something new.
async fn apply(voice: &super::Voice, note: &mut Note, instruction: &str) -> Applied {
    match clean::parse_verb(instruction) {
        Verb::Undo => {
            if note.undo() {
                Applied::Changed
            } else {
                Applied::Refused("nothing to undo".into())
            }
        }
        Verb::Done => Applied::Done,
        Verb::Drop(n) => match clean::drop_sentence(&note.text, n) {
            Some(after) => {
                note.push(instruction, after, None);
                Applied::Changed
            }
            None => Applied::Refused(format!("there is no sentence {n}")),
        },
        Verb::DropLast => {
            let count = clean::sentences(&note.text).len();
            match clean::drop_sentence(&note.text, count) {
                Some(after) if count > 0 => {
                    note.push(instruction, after, None);
                    Applied::Changed
                }
                _ => Applied::Refused("there is nothing to drop".into()),
            }
        }
        Verb::List => {
            let after = clean::as_list(&note.text);
            if after == note.text {
                return Applied::Refused("it is already a list".into());
            }
            note.push(instruction, after, None);
            Applied::Changed
        }
        Verb::Blocker => {
            if note.severity == "blocker" {
                return Applied::Refused("it is already a blocker".into());
            }
            let text = note.text.clone();
            note.push(instruction, text, Some("blocker"));
            Applied::Changed
        }
        Verb::Longer => {
            if note.text == note.base {
                return Applied::Refused("this is already the whole note".into());
            }
            let base = note.base.clone();
            note.push(instruction, base, None);
            Applied::Changed
        }
        verb @ (Verb::Shorter | Verb::Question | Verb::Free(_)) => {
            let Some(cleaner) = &voice.cleaner else {
                return Applied::Refused(
                    "no cleaner is configured for that; the closed verbs still work".into(),
                );
            };
            let ask = match &verb {
                Verb::Shorter => "Make it shorter.".to_string(),
                Verb::Question => "Rephrase it as a question.".to_string(),
                Verb::Free(text) => text.clone(),
                _ => unreachable!(),
            };
            match cleaner.rewrite(&note.text, &ask).await {
                Ok(candidate) if candidate == note.text => {
                    Applied::Refused("the cleaner left it as it was".into())
                }
                Ok(candidate) => {
                    if clean::guard(&note.raw, &candidate, &note.repairs) {
                        crate::metrics::VOICE_GUARD
                            .with_label_values(&["passed"])
                            .inc();
                        note.push(instruction, candidate, None);
                        Applied::Changed
                    } else {
                        crate::metrics::VOICE_GUARD
                            .with_label_values(&["refused"])
                            .inc();
                        Applied::Refused(
                            "the cleaner said something you did not; kept your words".into(),
                        )
                    }
                }
                Err(message) => Applied::Refused(message),
            }
        }
    }
}

/// A capture's audio: `<32 hex>.wav` under the session, and nothing else is a name.
pub async fn get_audio(
    State(state): State<AppState>,
    Path((id, name)): Path<(String, String)>,
) -> Response {
    let Some(capture) = name.strip_suffix(".wav") else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !is_capture_id(capture) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let dir = {
        let mgr = state.sessions.read().await;
        match mgr.get(&id) {
            Some(session) => session.voice_dir(),
            None => return StatusCode::NOT_FOUND.into_response(),
        }
    };
    match tokio::fs::read(dir.join(format!("{capture}.wav"))).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "audio/wav"),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

fn is_capture_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

#[derive(Deserialize)]
pub struct SpeechRequest {
    #[serde(default)]
    pub text: String,
}

/// Read a note aloud. Nothing is stored: the tablet plays what comes back.
pub async fn speech(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<SpeechRequest>,
) -> Response {
    if state.sessions.read().await.get(&id).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(tts) = state.voice.as_ref().and_then(|voice| voice.tts.as_ref()) else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no voice configured").into_response();
    };
    if body.text.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "nothing to say").into_response();
    }
    if body.text.chars().count() > super::tts::MAX_TEXT_CHARS {
        return (
            StatusCode::BAD_REQUEST,
            format!(
                "the text is longer than {} characters",
                super::tts::MAX_TEXT_CHARS
            ),
        )
            .into_response();
    }
    match tts.speak(&body.text).await {
        Ok(wav) => (
            [
                (header::CONTENT_TYPE, "audio/wav"),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            wav,
        )
            .into_response(),
        Err(error) => {
            warn!(session_id = %id, error, "speech failed");
            (StatusCode::INTERNAL_SERVER_ERROR, error).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Speech, as the gate sees it: loud, and never loud for long. A constant tone is a
    /// machine, and the gate is right to refuse it as speech — the earlier fixture was a
    /// fan wearing a reader's clothes.
    fn tone(seconds: f64) -> Vec<u8> {
        let n = (seconds * SAMPLE_RATE as f64) as usize;
        let period = (SAMPLE_RATE as f64 * 0.4) as usize; // 300 ms of voice, 100 ms of gap
        (0..n)
            .flat_map(|i| {
                let v = if i % period < period * 3 / 4 {
                    (3000.0 * (i as f64 * 0.1).sin()) as i16
                } else {
                    0
                };
                v.to_le_bytes()
            })
            .collect()
    }

    fn room(seconds: f64) -> Vec<u8> {
        let n = (seconds * SAMPLE_RATE as f64) as usize;
        (0..n)
            .flat_map(|i| ((i % 7) as i16 * 20).to_le_bytes())
            .collect()
    }

    #[test]
    fn the_gate_closes_after_speech_then_silence_and_not_before() {
        let mut gate = Gate::default();
        assert!(
            !gate.feed(&room(2.0)),
            "quiet before any speech is a reader thinking"
        );
        assert!(!gate.heard_speech());
        assert!(!gate.feed(&tone(0.02)), "a click is not speech");
        assert!(!gate.heard_speech());
        assert!(!gate.feed(&tone(1.0)));
        assert!(gate.heard_speech());
        assert!(!gate.feed(&room(1.0)), "a pause inside a sentence");
        assert!(!gate.feed(&tone(0.5)), "speech resets the quiet");
        assert!(!gate.feed(&room(1.0)));
        assert!(
            gate.feed(&room(0.5)),
            "1.5 s of quiet after speech ends the note"
        );
    }

    /// A room with the extractor fan on: loud enough that the old fixed threshold heard
    /// it as a person talking, and ended nothing.
    fn noisy_room(seconds: f64) -> Vec<u8> {
        let n = (seconds * SAMPLE_RATE as f64) as usize;
        (0..n)
            .flat_map(|i| {
                let v = (900.0 * (i as f64 * 0.37).sin()) as i16;
                v.to_le_bytes()
            })
            .collect()
    }

    #[test]
    fn room_noise_is_not_a_reader_talking() {
        let mut gate = Gate::default();
        for _ in 0..30 {
            assert!(!gate.feed(&noisy_room(0.5)), "a fan is not a sentence");
        }
        assert!(
            !gate.heard_speech(),
            "15 seconds of kitchen must not count as speech, or the capture never ends"
        );
    }

    #[test]
    fn a_reader_above_a_noisy_room_is_still_heard() {
        let mut gate = Gate::default();
        let _ = gate.feed(&noisy_room(0.5));
        assert!(!gate.heard_speech());
        // The same tone the other test uses, which is well above the fan.
        assert!(!gate.feed(&tone(0.3)));
        assert!(gate.heard_speech(), "speech above the room has to be heard");
        assert!(
            gate.feed(&noisy_room(2.0)),
            "and the fan that follows it ends the note"
        );
    }

    #[test]
    fn a_capture_name_is_32_lowercase_hex_and_nothing_else() {
        assert!(is_capture_id(&"a".repeat(32)));
        assert!(!is_capture_id(&"A".repeat(32)));
        assert!(!is_capture_id("../escape"));
        assert!(!is_capture_id(&"a".repeat(31)));
    }

    #[test]
    fn undo_restores_the_text_and_the_severity_together() {
        let mut note = Note {
            raw: "drop it".into(),
            text: "Drop it.".into(),
            base: "Drop it.".into(),
            history: Vec::new(),
            edits: Vec::new(),
            repairs: Vec::new(),
            confidence: 0.9,
            stt: SttProvenance::default(),
            seconds: 1.0,
            audio: None,
            severity: "note".into(),
            guarded: false,
        };
        note.push("mark it a blocker", "Drop it.".into(), Some("blocker"));
        note.push("as a list", "- Drop it.".into(), None);
        assert_eq!(note.edits.len(), 2);
        assert!(note.undo());
        assert_eq!(note.text, "Drop it.");
        assert_eq!(note.severity, "blocker");
        assert!(note.undo());
        assert_eq!(note.severity, "note");
        assert!(!note.undo());
        assert!(note.edits.is_empty());
    }
}
