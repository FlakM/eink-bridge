use prometheus::{
    Encoder, Histogram, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge,
    TextEncoder,
};
use std::sync::LazyLock;

macro_rules! reg {
    (counter $name:expr, $help:expr) => {{
        let m = IntCounter::new($name, $help).expect("valid metric");
        prometheus::register(Box::new(m.clone())).expect("register");
        m
    }};
    (counter_vec $name:expr, $help:expr, $labels:expr) => {{
        let m =
            IntCounterVec::new(prometheus::Opts::new($name, $help), $labels).expect("valid metric");
        prometheus::register(Box::new(m.clone())).expect("register");
        m
    }};
    (gauge $name:expr, $help:expr) => {{
        let m = IntGauge::new($name, $help).expect("valid metric");
        prometheus::register(Box::new(m.clone())).expect("register");
        m
    }};
    (histogram $name:expr, $help:expr, $buckets:expr) => {{
        let m = Histogram::with_opts(HistogramOpts::new($name, $help).buckets($buckets))
            .expect("valid metric");
        prometheus::register(Box::new(m.clone())).expect("register");
        m
    }};
    (histogram_vec $name:expr, $help:expr, $labels:expr, $buckets:expr) => {{
        let m = HistogramVec::new(HistogramOpts::new($name, $help).buckets($buckets), $labels)
            .expect("valid metric");
        prometheus::register(Box::new(m.clone())).expect("register");
        m
    }};
}

// Session lifecycle
pub static SESSIONS_CREATED: LazyLock<IntCounter> =
    LazyLock::new(|| reg!(counter "eink_sessions_created_total", "Sessions created"));
pub static SESSIONS_SUBMITTED: LazyLock<IntCounter> =
    LazyLock::new(|| reg!(counter "eink_sessions_submitted_total", "Sessions submitted"));
pub static SESSIONS_CANCELLED: LazyLock<IntCounter> =
    LazyLock::new(|| reg!(counter "eink_sessions_cancelled_total", "Sessions cancelled"));
pub static SESSIONS_EXPIRED: LazyLock<IntCounter> = LazyLock::new(
    || reg!(counter "eink_sessions_expired_total", "Sessions expired due to staleness"),
);
pub static SESSIONS_ACTIVE: LazyLock<IntGauge> =
    LazyLock::new(|| reg!(gauge "eink_sessions_active", "Currently active sessions"));
pub static SESSION_CONTENT_BYTES: LazyLock<Histogram> = LazyLock::new(|| {
    reg!(
        histogram "eink_session_content_bytes",
        "Session content size in bytes",
        vec![256.0, 1_024.0, 4_096.0, 16_384.0, 65_536.0, 262_144.0, 1_048_576.0]
    )
});
pub static SESSION_ANNOTATION_IMAGES: LazyLock<Histogram> = LazyLock::new(|| {
    reg!(
        histogram "eink_session_annotation_images",
        "Annotation images attached per submission",
        vec![0.5, 1.0, 2.0, 5.0, 10.0, 20.0]
    )
});

// OCR
pub static OCR_REQUESTS: LazyLock<IntCounterVec> =
    LazyLock::new(|| reg!(counter_vec "eink_ocr_requests_total", "OCR requests", &["result"]));
pub static OCR_DURATION: LazyLock<Histogram> = LazyLock::new(|| {
    reg!(
        histogram "eink_ocr_duration_seconds",
        "OCR wall-clock duration",
        vec![0.1, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0]
    )
});
pub static OCR_TOKENS: LazyLock<IntCounterVec> =
    LazyLock::new(|| reg!(counter_vec "eink_ocr_tokens_total", "Ollama token counts", &["kind"]));
pub static OCR_IMAGE_PIXELS: LazyLock<Histogram> = LazyLock::new(|| {
    reg!(
        histogram "eink_ocr_image_pixels",
        "OCR input image size in pixels (width * height)",
        vec![
            10_000.0, 40_000.0, 100_000.0, 250_000.0, 500_000.0,
            750_000.0, 1_000_000.0, 1_500_000.0,
        ]
    )
});

// WebSocket
pub static WS_CONNECTIONS_ACTIVE: LazyLock<IntGauge> =
    LazyLock::new(|| reg!(gauge "eink_ws_connections_active", "Active WebSocket connections"));

// Long-poll
pub static LONG_POLL_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(
    || reg!(counter_vec "eink_long_poll_total", "Long-poll result outcomes", &["result"]),
);

// Webhooks
pub static WEBHOOK_TOTAL: LazyLock<IntCounterVec> =
    LazyLock::new(|| reg!(counter_vec "eink_webhook_total", "Webhook fire outcomes", &["result"]));

// HTTP
pub static HTTP_REQUESTS: LazyLock<IntCounterVec> = LazyLock::new(|| {
    reg!(
        counter_vec "eink_http_requests_total",
        "HTTP requests by method, path and status",
        &["method", "path", "status"]
    )
});
pub static HTTP_DURATION: LazyLock<HistogramVec> = LazyLock::new(|| {
    reg!(
        histogram_vec "eink_http_request_duration_seconds",
        "HTTP request latency",
        &["method", "path"],
        vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0]
    )
});

/// Force-register all metrics so they appear in /metrics before any events occur.
pub fn init() {
    let _ = &*SESSIONS_CREATED;
    let _ = &*SESSIONS_SUBMITTED;
    let _ = &*SESSIONS_CANCELLED;
    let _ = &*SESSIONS_EXPIRED;
    let _ = &*SESSIONS_ACTIVE;
    let _ = &*SESSION_CONTENT_BYTES;
    let _ = &*SESSION_ANNOTATION_IMAGES;
    let _ = &*OCR_REQUESTS;
    let _ = &*OCR_DURATION;
    let _ = &*OCR_TOKENS;
    let _ = &*OCR_IMAGE_PIXELS;
    let _ = &*WS_CONNECTIONS_ACTIVE;
    let _ = &*LONG_POLL_TOTAL;
    let _ = &*WEBHOOK_TOTAL;
    let _ = &*HTTP_REQUESTS;
    let _ = &*HTTP_DURATION;
}

pub fn render() -> String {
    let encoder = TextEncoder::new();
    let families = prometheus::gather();
    let mut buf = Vec::new();
    encoder.encode(&families, &mut buf).unwrap_or_default();
    String::from_utf8(buf).unwrap_or_default()
}
