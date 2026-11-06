//! A spoken note, end to end against a recogniser that answers what the test tells it.
//!
//! What has to hold: the capture closes itself after a silence and on `stop`, the
//! transcript comes back cleaned with the document's spelling and the audio kept, a
//! cleaner that invents a word is refused and the reader's words win, the closed verbs
//! edit and undo, one capture per session, the bounds close a capture rather than let it
//! grow, nothing heard is an error rather than an empty note, and a capture name cannot
//! read outside the session.

mod common;

use axum::{Router, extract::Multipart, routing::post};
use eink_bridge::app::AppState;
use eink_bridge::voice::{SAMPLE_RATE, Voice, asr::Asr, llm::Cleaner, tts::Tts};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message as WsMessage;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The recogniser and the cleaner, scripted: `hears` is what every transcription says,
/// `rewrites` is what the cleaner answers.
struct Fake {
    url: String,
    hears: Arc<Mutex<String>>,
    rewrites: Arc<Mutex<String>>,
    transcriptions: Arc<Mutex<Vec<usize>>>,
}

async fn fake() -> Fake {
    let hears = Arc::new(Mutex::new(String::new()));
    let rewrites = Arc::new(Mutex::new(String::new()));
    let transcriptions = Arc::new(Mutex::new(Vec::new()));
    let (h, r, t) = (hears.clone(), rewrites.clone(), transcriptions.clone());
    let app =
        Router::new()
            .route(
                "/inference",
                post(move |mut parts: Multipart| {
                    let h = h.clone();
                    let t = t.clone();
                    async move {
                        let mut wav_len = 0;
                        while let Some(field) = parts.next_field().await.unwrap() {
                            if field.name() == Some("file") {
                                wav_len = field.bytes().await.unwrap().len();
                            }
                        }
                        t.lock().unwrap().push(wav_len);
                        let text = h.lock().unwrap().clone();
                        axum::Json(json!({"text": format!(" {text}\n"), "language": "english",
                        "segments": [{"avg_logprob": -0.2}, {"avg_logprob": -0.5}]}))
                    }
                }),
            )
            .route(
                "/api/generate",
                post(move || {
                    let r = r.clone();
                    async move {
                        axum::Json(json!({"response": r.lock().unwrap().clone(), "done": true}))
                    }
                }),
            )
            .layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Fake {
        url,
        hears,
        rewrites,
        transcriptions,
    }
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/voice")
        .join(name)
}

async fn start_server(fake: Option<&Fake>) -> (String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut state = AppState::with_config(dir.path().to_path_buf(), 5);
    state.voice = fake.map(|fake| {
        Voice::new(
            Asr::local(&fake.url, Duration::from_secs(5)),
            Some(Cleaner::new(
                fake.url.clone(),
                "m".into(),
                Duration::from_secs(5),
            )),
            Some(Tts::new(fixture("fake-piper.sh"), fixture("voice.onnx"))),
        )
    });
    let (url, _) = common::serve(state).await;
    (url, dir)
}

async fn create_session(url: &str) -> String {
    reqwest::Client::new()
        .post(format!("{url}/api/sessions"))
        .body(
            "# Relocation\n\n`reanchor.rs` decides where a note lands. Set EINK_TOKEN on the LAN.",
        )
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn connect(url: &str, id: &str) -> Result<Socket, u16> {
    let ws_url = format!(
        "{}/api/sessions/{id}/voice",
        url.replacen("http://", "ws://", 1)
    );
    match tokio_tungstenite::connect_async(&ws_url).await {
        Ok((socket, _)) => Ok(socket),
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            Err(response.status().as_u16())
        }
        Err(other) => panic!("unexpected connect error: {other}"),
    }
}

async fn next_frame(socket: &mut Socket) -> Value {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match socket.next().await {
                Some(Ok(WsMessage::Text(text))) => break serde_json::from_str(&text).unwrap(),
                Some(Ok(WsMessage::Close(_))) | None => break json!({"type":"closed"}),
                Some(Ok(_)) => continue,
                Some(Err(e)) => panic!("socket error: {e}"),
            }
        }
    })
    .await
    .expect("a frame within ten seconds")
}

async fn frame_of_type(socket: &mut Socket, wanted: &str) -> Value {
    loop {
        let frame = next_frame(socket).await;
        if frame["type"] == wanted {
            return frame;
        }
        assert_ne!(frame["type"], "error", "unexpected error frame: {frame}");
        assert_ne!(
            frame["type"], "closed",
            "the socket closed before a {wanted} frame"
        );
    }
}

/// Speech as the gate has to hear it: loud, with gaps. A constant tone is a machine, and
/// the gate treats it as one — the room floor is the quietest block of the last two
/// seconds, so anything that never goes quiet is the room, not the reader.
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

fn silence(seconds: f64) -> Vec<u8> {
    vec![0; (seconds * SAMPLE_RATE as f64) as usize * 2]
}

/// Audio in the tablet's 20 ms frames.
async fn speak(socket: &mut Socket, pcm: &[u8]) {
    for chunk in pcm.chunks(640) {
        socket
            .send(WsMessage::Binary(chunk.to_vec()))
            .await
            .unwrap();
    }
}

/// Silence, in the tablet's frames, until the server decides nobody is there.
///
/// The amount matters. A capture with no speech ends itself at fifteen seconds, so this
/// streams exactly fifteen and then listens: one more second and the tail of the burst is
/// written after the server has already hung up, which is a `BrokenPipe` racing the close
/// (it did on CI, while passing locally where the extra second fit in the socket buffer).
/// A write that does fail is still not an assertion about the server — the test is asking
/// what it *said* before it left.
async fn silence_until_the_capture_ends_itself(socket: &mut Socket) -> Value {
    const SECONDS: f64 = 15.0;
    for chunk in silence(SECONDS).chunks(640) {
        if socket
            .send(WsMessage::Binary(chunk.to_vec()))
            .await
            .is_err()
        {
            break;
        }
    }
    next_frame(socket).await
}

async fn say(socket: &mut Socket, frame: Value) {
    socket
        .send(WsMessage::Text(frame.to_string()))
        .await
        .unwrap();
}

/// Two seconds of speech, then the quiet that ends the note.
async fn speak_and_pause(socket: &mut Socket) -> Value {
    speak(socket, &tone(2.0)).await;
    speak(socket, &silence(1.6)).await;
    frame_of_type(socket, "final").await
}

#[tokio::test]
async fn a_spoken_note_comes_back_cleaned_with_the_documents_spelling_and_its_audio_kept() {
    let fake = fake().await;
    *fake.hears.lock().unwrap() = "so um re-anchor dot rs should refuse this".into();
    let (url, _dir) = start_server(Some(&fake)).await;
    let id = create_session(&url).await;
    let mut socket = connect(&url, &id).await.unwrap();

    let ready = next_frame(&mut socket).await;
    assert_eq!(ready["type"], "ready");
    assert_eq!(ready["seq"], 0);
    let capture = ready["capture"].as_str().unwrap().to_string();
    assert_eq!(capture.len(), 32);
    assert_eq!(ready["stt"]["provider"], "whisper");

    speak(&mut socket, &tone(2.0)).await;
    let partial = frame_of_type(&mut socket, "partial").await;
    assert_eq!(
        partial["text"], "so um re-anchor dot rs should refuse this",
        "a partial is what was heard, uncleaned"
    );

    speak(&mut socket, &silence(1.6)).await;
    let final_ = frame_of_type(&mut socket, "final").await;
    assert_eq!(final_["raw"], "so um re-anchor dot rs should refuse this");
    assert_eq!(final_["text"], "reanchor.rs should refuse this.");
    assert_eq!(final_["guarded"], false);
    assert_eq!(
        final_["repairs"],
        json!([{"heard":"re-anchor dot rs","wrote":"reanchor.rs"}])
    );
    assert!(
        (final_["confidence"].as_f64().unwrap() - (-0.5f64).exp()).abs() < 1e-5,
        "the worst segment"
    );
    // 2 s of speech plus the 1.5 s silence gate, plus the final gap of the last speech
    // burst — the fixture now has gaps, which is what makes it speech rather than a fan.
    let seconds = final_["seconds"].as_f64().unwrap();
    assert!(
        (3.3..=3.6).contains(&seconds),
        "{seconds} s of audio were kept"
    );
    assert!(final_["seq"].as_u64().unwrap() > partial["seq"].as_u64().unwrap());
    let audio = final_["audio"].as_str().unwrap().to_string();
    assert_eq!(audio, format!("/api/sessions/{id}/voice/{capture}.wav"));

    let wav = reqwest::get(format!("{url}{audio}")).await.unwrap();
    assert_eq!(wav.status(), 200);
    assert_eq!(wav.headers()["content-type"], "audio/wav");
    assert_eq!(wav.headers()["cache-control"], "no-store");
    let bytes = wav.bytes().await.unwrap();
    assert_eq!(&bytes[..4], b"RIFF");
    // The audio kept is exactly the audio the note was made from: the gate closed at
    // 1.5 s of quiet, and whatever arrived after that is not part of the note.
    let expected = 44 + (seconds * 32_000.0) as i64;
    assert!(
        (bytes.len() as i64 - expected).abs() <= 64,
        "{} bytes of wav for {seconds} s",
        bytes.len()
    );

    say(&mut socket, json!({"type":"done"})).await;
    let done = frame_of_type(&mut socket, "done").await;
    assert_eq!(done["text"], "reanchor.rs should refuse this.");
    assert_eq!(done["note"]["raw"], final_["raw"]);
    assert_eq!(done["note"]["severity"], "note");
    assert_eq!(done["note"]["audio"], audio);
    assert_eq!(done["note"]["stt"]["provider"], "whisper");
    assert_eq!(next_frame(&mut socket).await["type"], "closed");

    // The session is free again once the capture is over.
    drop(socket);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(connect(&url, &id).await.is_ok());
}

#[tokio::test]
async fn the_closed_verbs_edit_the_note_and_undo_takes_them_back() {
    let fake = fake().await;
    *fake.hears.lock().unwrap() = "drop the cache column. the index is enough".into();
    let (url, _dir) = start_server(Some(&fake)).await;
    let id = create_session(&url).await;
    let mut socket = connect(&url, &id).await.unwrap();
    next_frame(&mut socket).await;
    let final_ = speak_and_pause(&mut socket).await;
    assert_eq!(
        final_["text"],
        "Drop the cache column. The index is enough."
    );

    say(
        &mut socket,
        json!({"type":"edit","instruction":"turn that into a list"}),
    )
    .await;
    let edit = frame_of_type(&mut socket, "edit").await;
    assert_eq!(edit["applied"], true);
    assert_eq!(
        edit["text"],
        "- Drop the cache column.\n- The index is enough."
    );
    assert_eq!(edit["edits"], 1);

    say(
        &mut socket,
        json!({"type":"edit","instruction":"mark it a blocker"}),
    )
    .await;
    let edit = frame_of_type(&mut socket, "edit").await;
    assert_eq!(edit["severity"], "blocker");
    assert_eq!(edit["edits"], 2);

    say(&mut socket, json!({"type":"edit","instruction":"drop two"})).await;
    let edit = frame_of_type(&mut socket, "edit").await;
    assert_eq!(edit["text"], "- Drop the cache column.");

    say(&mut socket, json!({"type":"edit","instruction":"drop two"})).await;
    let edit = frame_of_type(&mut socket, "edit").await;
    assert_eq!(edit["applied"], false);
    assert_eq!(edit["message"], "there is no sentence 2");
    assert_eq!(
        edit["text"], "- Drop the cache column.",
        "a refused edit changes nothing"
    );

    say(&mut socket, json!({"type":"undo"})).await;
    let edit = frame_of_type(&mut socket, "edit").await;
    assert_eq!(edit["applied"], true);
    assert_eq!(
        edit["text"],
        "- Drop the cache column.\n- The index is enough."
    );
    assert_eq!(edit["severity"], "blocker");

    say(&mut socket, json!({"type":"edit","instruction":"undo"})).await;
    let edit = frame_of_type(&mut socket, "edit").await;
    assert_eq!(edit["severity"], "note", "undo is a verb too");

    say(&mut socket, json!({"type":"edit","instruction":"longer"})).await;
    let edit = frame_of_type(&mut socket, "edit").await;
    assert_eq!(edit["text"], "Drop the cache column. The index is enough.");

    say(&mut socket, json!({"type":"done"})).await;
    let done = frame_of_type(&mut socket, "done").await;
    let edits = done["note"]["edits"].as_array().unwrap();
    assert_eq!(edits.len(), 2, "undone edits are not in the record");
    assert_eq!(edits[0]["instruction"], "turn that into a list");
    assert_eq!(edits[1]["instruction"], "longer");
    assert_eq!(
        edits[1]["before"],
        "- Drop the cache column.\n- The index is enough."
    );
    assert_eq!(edits[1]["after"], done["text"]);
}

#[tokio::test]
async fn a_cleaner_that_invents_a_word_is_refused_and_the_readers_words_win() {
    let fake = fake().await;
    *fake.hears.lock().unwrap() = "add a cache column here, I think".into();
    let (url, _dir) = start_server(Some(&fake)).await;
    let id = create_session(&url).await;
    let mut socket = connect(&url, &id).await.unwrap();
    next_frame(&mut socket).await;
    let final_ = speak_and_pause(&mut socket).await;
    assert_eq!(final_["text"], "Add a cache column here, I think.");

    *fake.rewrites.lock().unwrap() = "Add a caching layer with Redis.".into();
    say(&mut socket, json!({"type":"edit","instruction":"shorter"})).await;
    let edit = frame_of_type(&mut socket, "edit").await;
    assert_eq!(edit["applied"], false);
    assert_eq!(
        edit["message"],
        "the cleaner said something you did not; kept your words"
    );
    assert_eq!(edit["text"], "Add a cache column here, I think.");

    *fake.rewrites.lock().unwrap() = "Add a cache column, I think.".into();
    say(&mut socket, json!({"type":"edit","instruction":"shorter"})).await;
    let edit = frame_of_type(&mut socket, "edit").await;
    assert_eq!(edit["applied"], true);
    assert_eq!(
        edit["text"], "Add a cache column, I think.",
        "a hedge the reader said survives"
    );

    *fake.rewrites.lock().unwrap() = "Should we add a cache column here?".into();
    say(
        &mut socket,
        json!({"type":"edit","instruction":"make it a question"}),
    )
    .await;
    let edit = frame_of_type(&mut socket, "edit").await;
    assert_eq!(edit["applied"], true, "question words carry no content");
}

#[tokio::test]
async fn stop_ends_the_capture_before_the_silence_gate_does() {
    let fake = fake().await;
    *fake.hears.lock().unwrap() = "ship it".into();
    let (url, _dir) = start_server(Some(&fake)).await;
    let id = create_session(&url).await;
    let mut socket = connect(&url, &id).await.unwrap();
    next_frame(&mut socket).await;
    speak(&mut socket, &tone(1.0)).await;
    say(&mut socket, json!({"type":"stop"})).await;
    let final_ = frame_of_type(&mut socket, "final").await;
    assert_eq!(final_["text"], "Ship it.");
    let seconds = final_["seconds"].as_f64().unwrap();
    assert!((0.9..=1.1).contains(&seconds), "{seconds}");
}

#[tokio::test]
async fn nothing_heard_is_an_error_not_an_empty_note() {
    let fake = fake().await;
    *fake.hears.lock().unwrap() = "this would be a hallucination".into();
    let (url, _dir) = start_server(Some(&fake)).await;
    let id = create_session(&url).await;
    let mut socket = connect(&url, &id).await.unwrap();
    next_frame(&mut socket).await;
    speak(&mut socket, &silence(3.0)).await;
    say(&mut socket, json!({"type":"stop"})).await;
    let error = next_frame(&mut socket).await;
    assert_eq!(error["type"], "error");
    assert_eq!(error["message"], "nothing was heard");
    assert!(
        fake.transcriptions.lock().unwrap().is_empty(),
        "silence is never sent to the recogniser"
    );
    assert_eq!(next_frame(&mut socket).await["type"], "closed");
}

#[tokio::test]
async fn one_capture_per_session_and_none_on_a_finished_or_unknown_one() {
    let fake = fake().await;
    let (url, _dir) = start_server(Some(&fake)).await;
    let id = create_session(&url).await;
    let first = connect(&url, &id).await.unwrap();
    assert_eq!(
        connect(&url, &id).await.err(),
        Some(409),
        "a second capture is a conflict, like a second submit"
    );
    drop(first);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        connect(&url, &id).await.is_ok(),
        "the slot is released with the socket"
    );

    assert_eq!(connect(&url, "nope").await.err(), Some(404));
    reqwest::Client::new()
        .delete(format!("{url}/api/sessions/{id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        connect(&url, &id).await.err(),
        Some(409),
        "a finished session has no page to speak at"
    );

    let (url, _dir) = start_server(None).await;
    let id = create_session(&url).await;
    assert_eq!(
        connect(&url, &id).await.err(),
        Some(503),
        "no recogniser, said up front"
    );
}

#[tokio::test]
async fn the_size_bound_closes_a_capture_instead_of_letting_it_grow() {
    let fake = fake().await;
    *fake.hears.lock().unwrap() = "a very long note".into();
    let (url, _dir) = start_server(Some(&fake)).await;
    let id = create_session(&url).await;
    let mut socket = connect(&url, &id).await.unwrap();
    next_frame(&mut socket).await;
    let loud = tone(2.0);
    // The tablet streams until it is told to stop, so the client does too. The writes end
    // when the server closes the capture; a write that fails after the bound fired is the
    // close arriving, not a bug, and on a slow machine it used to panic the test.
    // 80 bursts of two seconds is 160 s: past the four-megabyte bound at 16 kHz and far
    // short of the test suite's patience if the bound never fires.
    let mut final_ = None;
    'streaming: for _ in 0..80 {
        for chunk in loud.chunks(64 * 1024) {
            if socket
                .send(WsMessage::Binary(chunk.to_vec()))
                .await
                .is_err()
            {
                break 'streaming;
            }
            // Check between bursts, so the frame the bound sent is read before the close.
            if let Ok(Some(Ok(WsMessage::Text(text)))) =
                tokio::time::timeout(Duration::from_millis(20), socket.next()).await
            {
                let frame: Value = serde_json::from_str(&text).unwrap();
                if frame["type"] == "final" {
                    final_ = Some(frame);
                    break 'streaming;
                }
            }
        }
    }
    let final_ = match final_ {
        Some(frame) => frame,
        None => frame_of_type(&mut socket, "final").await,
    };
    let seconds = final_["seconds"].as_f64().unwrap();
    assert!(
        seconds <= 4.0 * 1024.0 * 1024.0 / 32_000.0 + 0.1,
        "{seconds} s is more than 4 MB"
    );
    assert!(
        seconds > 100.0,
        "{seconds} s: the bound closed it, not the gate"
    );
}

#[tokio::test]
async fn a_capture_nobody_speaks_into_closes_itself() {
    // The bug this pins: the gate needed speech before it would end a capture, and in a
    // kitchen it had already decided the extractor fan was speech — so a capture with
    // nobody at the tablet ran for seventy seconds and kept recording.
    let fake = fake().await;
    let (url, _dir) = start_server(Some(&fake)).await;
    let id = create_session(&url).await;
    let mut socket = connect(&url, &id).await.unwrap();
    next_frame(&mut socket).await; // ready

    let frame = silence_until_the_capture_ends_itself(&mut socket).await;
    assert_eq!(
        frame["type"], "error",
        "an empty capture has to end by itself"
    );
    assert_eq!(frame["message"], "nothing was heard");
}

#[tokio::test]
async fn discard_throws_the_audio_away_and_a_name_cannot_leave_the_session() {
    let fake = fake().await;
    *fake.hears.lock().unwrap() = "never mind".into();
    let (url, dir) = start_server(Some(&fake)).await;
    let id = create_session(&url).await;
    let mut socket = connect(&url, &id).await.unwrap();
    next_frame(&mut socket).await;
    let final_ = speak_and_pause(&mut socket).await;
    let audio = final_["audio"].as_str().unwrap().to_string();
    assert_eq!(
        reqwest::get(format!("{url}{audio}"))
            .await
            .unwrap()
            .status(),
        200
    );
    say(&mut socket, json!({"type":"discard"})).await;
    assert_eq!(next_frame(&mut socket).await["type"], "closed");
    assert_eq!(
        reqwest::get(format!("{url}{audio}"))
            .await
            .unwrap()
            .status(),
        404
    );

    let session_dir = dir.path().join("sessions").join(&id);
    std::fs::create_dir_all(session_dir.join("voice")).unwrap();
    std::fs::write(session_dir.join("escape.wav"), b"RIFF").unwrap();
    for name in [
        "..%2Fescape.wav",
        "escape.wav",
        "ABCDEFABCDEFABCDEFABCDEFABCDEFAB.wav",
    ] {
        let status = reqwest::get(format!("{url}/api/sessions/{id}/voice/{name}"))
            .await
            .unwrap()
            .status();
        assert_eq!(status, 404, "{name} is not a capture");
    }
}

#[tokio::test]
async fn speech_reads_a_note_aloud_with_the_configured_voice() {
    let fake = fake().await;
    let (url, _dir) = start_server(Some(&fake)).await;
    let id = create_session(&url).await;
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{url}/api/sessions/{id}/speech"))
        .json(&json!({"text":"Drop the cache column."}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "audio/wav");
    let wav = response.bytes().await.unwrap();
    assert_eq!(&wav[..4], b"RIFF");
    assert_eq!(
        u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]),
        16_000,
        "the voice's own rate"
    );

    let empty = client
        .post(format!("{url}/api/sessions/{id}/speech"))
        .json(&json!({"text":"  "}))
        .send()
        .await
        .unwrap();
    assert_eq!(empty.status(), 400);
    let unknown = client
        .post(format!("{url}/api/sessions/nope/speech"))
        .json(&json!({"text":"hi"}))
        .send()
        .await
        .unwrap();
    assert_eq!(unknown.status(), 404);

    let (url, _dir) = start_server(None).await;
    let id = create_session(&url).await;
    let none = client
        .post(format!("{url}/api/sessions/{id}/speech"))
        .json(&json!({"text":"hi"}))
        .send()
        .await
        .unwrap();
    assert_eq!(none.status(), 503);
    assert_eq!(none.text().await.unwrap(), "no voice configured");
}
