use super::*;
use axum::{
    Json,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

struct Mock {
    url: String,
    calls: Arc<AtomicUsize>,
    requests: Arc<StdMutex<Vec<(HeaderMap, Value)>>>,
}

async fn mock(responses: Vec<(StatusCode, Value)>, delay: Duration) -> Mock {
    let calls = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(StdMutex::new(Vec::new()));
    let responses = Arc::new(responses);
    let handler_calls = calls.clone();
    let handler_requests = requests.clone();
    let app =
        axum::Router::new().fallback(post(move |headers: HeaderMap, Json(body): Json<Value>| {
            let index = handler_calls.fetch_add(1, Ordering::SeqCst);
            handler_requests.lock().unwrap().push((headers, body));
            let response = responses[index.min(responses.len() - 1)].clone();
            async move {
                tokio::time::sleep(delay).await;
                (response.0, Json(response.1))
            }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Mock {
        url,
        calls,
        requests,
    }
}

fn cloud_text(text: &str) -> Value {
    json!({"status":"completed", "output":[{"type":"message", "content":[{"type":"output_text", "text":text}]}],
        "usage":{"input_tokens":10,"output_tokens":3}})
}

fn engine(local: &Mock, cloud: Option<&Mock>) -> OcrEngine {
    OcrEngine {
        debug_save: false,
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap(),
        ollama_url: local.url.clone(),
        ollama_model: "local-model".into(),
        llama_url: None,
        local_timeout: Duration::from_secs(2),
        interactive_timeout: Duration::from_millis(200),
        openai: cloud.map(|mock| {
            providers::OpenAi::new(
                "test-key",
                "test-vision".into(),
                Duration::from_millis(100),
                format!("{}/v1/responses", mock.url),
            )
            .unwrap()
        }),
        cache: Mutex::new(cache::RecognitionCache::default()),
    }
}

#[tokio::test]
async fn dedicated_local_runtime_uses_chat_api_without_touching_ollama() {
    let unused_ollama = mock(
        vec![(StatusCode::OK, json!({"response":"wrong backend"}))],
        Duration::ZERO,
    )
    .await;
    let llama = mock(
        vec![(
            StatusCode::OK,
            json!({"choices":[{"finish_reason":"stop", "message":{"content":"REMOVE IT"}}]}),
        )],
        Duration::ZERO,
    )
    .await;
    let mut engine = engine(&unused_ollama, None);
    engine.llama_url = Some(llama.url.clone());
    assert_eq!(engine.recognize_image(b"png").await.unwrap(), "REMOVE IT");
    assert_eq!(unused_ollama.calls.load(Ordering::SeqCst), 0);
    let requests = llama.requests.lock().unwrap();
    let body = &requests[0].1;
    assert_eq!(body["max_tokens"], 256);
    assert_eq!(body["cache_prompt"], false);
    assert_eq!(
        body["messages"][0]["content"][1]["image_url"]["url"],
        "data:image/png;base64,cG5n"
    );
}

#[tokio::test]
async fn truncated_dedicated_local_output_does_not_beat_cloud() {
    let local = mock(
        vec![(
            StatusCode::OK,
            json!({"choices":[{"finish_reason":"length", "message":{"content":"repeated"}}]}),
        )],
        Duration::ZERO,
    )
    .await;
    let cloud = mock(
        vec![(StatusCode::OK, cloud_text("REMOVE IT"))],
        Duration::ZERO,
    )
    .await;
    let mut engine = engine(&local, Some(&cloud));
    engine.llama_url = Some(local.url.clone());
    assert_eq!(engine.recognize_image(b"png").await.unwrap(), "REMOVE IT");
}

#[tokio::test]
async fn either_http_provider_can_win_without_waiting_for_the_other() {
    for cloud_wins in [true, false] {
        let local = mock(
            vec![(StatusCode::OK, json!({"response":"LOCAL"}))],
            if cloud_wins {
                Duration::from_secs(1)
            } else {
                Duration::ZERO
            },
        )
        .await;
        let cloud = mock(
            vec![(StatusCode::OK, cloud_text("CLOUD"))],
            if cloud_wins {
                Duration::ZERO
            } else {
                Duration::from_secs(1)
            },
        )
        .await;
        let engine = engine(&local, Some(&cloud));
        let result =
            tokio::time::timeout(Duration::from_millis(500), engine.recognize_image(b"image"))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(result, if cloud_wins { "CLOUD" } else { "LOCAL" });
        assert_eq!(engine.recognize_image(b"image").await.unwrap(), result);
        assert_eq!(
            if cloud_wins {
                cloud.calls.load(Ordering::SeqCst)
            } else {
                local.calls.load(Ordering::SeqCst)
            },
            1
        );
    }
}

#[tokio::test]
async fn openai_receives_only_a_crop_and_server_side_authorization() {
    let local = mock(
        vec![(StatusCode::OK, json!({"response":"LOCAL"}))],
        Duration::from_secs(1),
    )
    .await;
    let cloud = mock(
        vec![(StatusCode::OK, cloud_text("REMOVE IT"))],
        Duration::ZERO,
    )
    .await;
    let engine = engine(&local, Some(&cloud));
    assert_eq!(engine.recognize_image(b"png").await.unwrap(), "REMOVE IT");
    let requests = cloud.requests.lock().unwrap();
    let (headers, body) = &requests[0];
    assert_eq!(headers["authorization"], "Bearer test-key");
    assert_eq!(body["model"], "test-vision");
    assert_eq!(body["store"], false);
    assert_eq!(body["max_output_tokens"], 256);
    assert_eq!(
        body["input"][0]["content"][1]["image_url"],
        "data:image/png;base64,cG5n"
    );
    assert_eq!(body["input"][0]["content"][1]["detail"], "high");
    assert!(!body.to_string().contains("test-key"));
}

#[tokio::test]
async fn cloud_http_errors_blank_output_and_timeout_fall_back_to_local() {
    for (status, body, delay) in [
        (
            StatusCode::UNAUTHORIZED,
            json!({"error":"test-key must not be logged"}),
            Duration::ZERO,
        ),
        (
            StatusCode::TOO_MANY_REQUESTS,
            json!({"error":"rate limited"}),
            Duration::ZERO,
        ),
        (StatusCode::OK, cloud_text("   "), Duration::ZERO),
        (
            StatusCode::OK,
            json!({"status":"incomplete"}),
            Duration::ZERO,
        ),
        (StatusCode::OK, cloud_text("late"), Duration::from_secs(1)),
    ] {
        let local = mock(
            vec![(StatusCode::OK, json!({"response":"REMOVE IT"}))],
            Duration::from_millis(150),
        )
        .await;
        let cloud = mock(vec![(status, body)], delay).await;
        assert_eq!(
            engine(&local, Some(&cloud))
                .recognize_image(b"png")
                .await
                .unwrap(),
            "REMOVE IT"
        );
    }
}

#[tokio::test]
async fn local_failure_can_be_rescued_by_cloud_and_failures_are_not_cached() {
    let local = mock(
        vec![(StatusCode::INTERNAL_SERVER_ERROR, json!({"error":"failed"}))],
        Duration::ZERO,
    )
    .await;
    let cloud = mock(
        vec![
            (StatusCode::UNAUTHORIZED, json!({"error":"test-key"})),
            (StatusCode::OK, cloud_text("REMOVE IT")),
        ],
        Duration::ZERO,
    )
    .await;
    let engine = engine(&local, Some(&cloud));
    let error = engine.recognize_image(b"png").await.unwrap_err();
    assert!(error.contains("local") && error.contains("openai"));
    assert!(!error.contains("test-key"));
    assert_eq!(engine.recognize_image(b"png").await.unwrap(), "REMOVE IT");
    assert_eq!(cloud.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn identical_concurrent_images_share_one_request_and_edits_miss_the_cache() {
    let local = mock(
        vec![(StatusCode::OK, json!({"response":"REMOVE IT"}))],
        Duration::from_millis(20),
    )
    .await;
    let unused_cloud = mock(vec![(StatusCode::OK, cloud_text("CLOUD"))], Duration::ZERO).await;
    let engine = engine(&local, None);
    let (a, b, c) = tokio::join!(
        engine.recognize_image(b"ink"),
        engine.recognize_image(b"ink"),
        engine.recognize_image(b"ink")
    );
    assert_eq!(a.unwrap(), "REMOVE IT");
    assert_eq!(b.unwrap(), "REMOVE IT");
    assert_eq!(c.unwrap(), "REMOVE IT");
    assert_eq!(local.calls.load(Ordering::SeqCst), 1);
    engine.recognize_image(b"ink edited").await.unwrap();
    assert_eq!(local.calls.load(Ordering::SeqCst), 2);
    assert_eq!(unused_cloud.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn cancelling_a_caller_does_not_poison_the_single_flight_cache() {
    use std::{
        future::{Future, poll_fn},
        task::Poll,
    };
    let local = mock(
        vec![(StatusCode::OK, json!({"response":"REMOVE IT"}))],
        Duration::from_millis(20),
    )
    .await;
    let engine = engine(&local, None);
    {
        let mut first = std::pin::pin!(engine.recognize_image(b"ink"));
        poll_fn(|cx| {
            assert!(first.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
    }
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), engine.recognize_image(b"ink"))
            .await
            .unwrap()
            .unwrap(),
        "REMOVE IT"
    );
}

#[tokio::test]
async fn concurrent_failures_are_shared_but_a_later_call_retries() {
    let local = mock(
        vec![(StatusCode::INTERNAL_SERVER_ERROR, json!({"error":"failed"}))],
        Duration::from_millis(20),
    )
    .await;
    let engine = engine(&local, None);
    let (a, b, c) = tokio::join!(
        engine.recognize_image(b"ink"),
        engine.recognize_image(b"ink"),
        engine.recognize_image(b"ink")
    );
    assert!(a.is_err() && b.is_err() && c.is_err());
    assert_eq!(local.calls.load(Ordering::SeqCst), 1);
    assert!(engine.recognize_image(b"ink").await.is_err());
    assert_eq!(local.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn benchmark_bypasses_cache_and_does_not_cancel_either_provider() {
    let local = mock(
        vec![(StatusCode::OK, json!({"response":"LOCAL"}))],
        Duration::ZERO,
    )
    .await;
    let cloud = mock(
        vec![(StatusCode::OK, cloud_text("CLOUD"))],
        Duration::from_millis(20),
    )
    .await;
    let engine = engine(&local, Some(&cloud));
    for _ in 0..2 {
        let results = engine.compare_image(b"png").await;
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].result.as_deref().unwrap(), "LOCAL");
        assert_eq!(results[1].result.as_deref().unwrap(), "CLOUD");
    }
    assert_eq!(local.calls.load(Ordering::SeqCst), 2);
    assert_eq!(cloud.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn handwriting_with_nothing_drawn_makes_no_request() {
    // The app asks for OCR before it knows whether the reader drew anything; an empty
    // canvas must not cost a round trip, and must not come back as a failure.
    let provider = mock(
        vec![(StatusCode::OK, json!({"response":"should never be read"}))],
        Duration::ZERO,
    )
    .await;
    let engine = engine(&provider, None);
    assert_eq!(engine.recognize_strokes(&[], &[]).await.unwrap(), "");
    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        0,
        "no request should have been made"
    );
}

#[tokio::test]
async fn strokes_are_drawn_into_an_image_before_they_are_sent() {
    // The provider only ever sees a picture. If the strokes are not rendered -- or are
    // rendered empty -- the reader's handwriting silently transcribes as nothing.
    let provider = mock(
        vec![(StatusCode::OK, json!({"response":"WRITE A TLDR"}))],
        Duration::ZERO,
    )
    .await;
    let engine = engine(&provider, None);
    let strokes = vec![
        vec![[10.0, 20.0], [40.0, 60.0], [80.0, 90.0]],
        vec![[10.0, 100.0], [120.0, 140.0]],
    ];
    let pressures = vec![vec![0.4, 0.6, 0.9], vec![0.2, 0.5]];

    assert_eq!(
        engine
            .recognize_strokes(&strokes, &pressures)
            .await
            .unwrap(),
        "WRITE A TLDR"
    );

    let requests = provider.requests.lock().unwrap();
    let image = requests[0].1["images"][0].as_str().unwrap();
    assert!(
        image.starts_with("iVBORw0KGgo"),
        "the provider should receive a PNG: {}",
        &image[..image.len().min(24)]
    );
    assert!(
        image.len() > 100,
        "a drawing of five strokes is not an empty image"
    );
}

#[tokio::test]
async fn a_local_runtime_http_error_is_reported_with_its_status() {
    let provider = mock(
        vec![(StatusCode::INTERNAL_SERVER_ERROR, json!({"error":"boom"}))],
        Duration::ZERO,
    )
    .await;
    let mut engine = engine(&provider, None);
    engine.llama_url = Some(provider.url.clone());
    let error = engine.recognize_image(b"png").await.unwrap_err();
    assert!(
        error.contains("500"),
        "the operator needs the status to tell a crash from a timeout: {error}"
    );
}

#[tokio::test]
async fn a_model_that_never_answers_is_abandoned_at_the_bound() {
    // A stalled model used to hold the caller for the engine's whole two-minute timeout,
    // and on the interactive path that is a reader waiting with a pen in their hand.
    let stalled = mock(
        vec![(StatusCode::OK, json!({"response": "too late"}))],
        Duration::from_secs(5),
    )
    .await;
    let engine = engine(&stalled, None);

    let started = std::time::Instant::now();
    let error = engine
        .recognize_image_within(b"png", Duration::from_millis(150))
        .await
        .unwrap_err();
    assert!(
        error.contains("timed out"),
        "the caller should be told it was a timeout: {error}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the bound has to be the caller's, not the engine's"
    );
}

#[tokio::test]
async fn the_interactive_bound_is_shorter_than_the_general_one() {
    // The tablet asks for OCR while the reader waits, so it must not inherit a bound
    // meant for a batch job.
    let engine = engine(
        &mock(
            vec![(StatusCode::OK, json!({"response": "x"}))],
            Duration::ZERO,
        )
        .await,
        None,
    );
    assert!(engine.interactive_timeout() < engine.local_timeout);
}
