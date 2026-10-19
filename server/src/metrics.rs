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
pub static SESSIONS_STORED: LazyLock<IntGauge> =
    LazyLock::new(|| reg!(gauge "eink_sessions_stored", "Total sessions stored"));
pub static SESSIONS_STARRED: LazyLock<IntGauge> =
    LazyLock::new(|| reg!(gauge "eink_sessions_starred", "Currently starred (pinned) sessions"));
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
pub static OCR_CACHE: LazyLock<IntCounterVec> = LazyLock::new(
    || reg!(counter_vec "eink_ocr_cache_total", "OCR cache lookups (hit, miss, coalesced)", &["result"]),
);
pub static OCR_PROVIDER_REQUESTS: LazyLock<IntCounterVec> = LazyLock::new(
    || reg!(counter_vec "eink_ocr_provider_requests_total", "Provider outcomes (ok, error, cancelled)", &["provider", "result"]),
);
pub static OCR_PROVIDER_DURATION: LazyLock<HistogramVec> = LazyLock::new(|| {
    reg!(histogram_vec "eink_ocr_provider_duration_seconds", "Provider duration including cancelled attempts", &["provider", "result"],
        vec![0.1, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0, 120.0])
});
pub static OCR_PROVIDER_WINS: LazyLock<IntCounterVec> = LazyLock::new(
    || reg!(counter_vec "eink_ocr_provider_wins_total", "Successful uncached OCR selections", &["provider"]),
);
pub static OCR_PROVIDER_TOKENS: LazyLock<IntCounterVec> = LazyLock::new(
    || reg!(counter_vec "eink_ocr_provider_tokens_total", "Tokens reported by completed provider responses", &["provider", "kind"]),
);
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
///
/// A labelled metric with no children exports nothing at all: the encoder walks the
/// children, so `eink_ocr_requests_total` is *absent* rather than zero until some code
/// path sets a label. That leaves the runbook's queries and the Grafana panels empty on a
/// quiet server, which reads as "the metric is gone". The fixed vocabularies below are
/// the values this program actually uses, so they are seeded at zero.
pub fn init() {
    let _ = &*SESSIONS_CREATED;
    let _ = &*SESSIONS_SUBMITTED;
    let _ = &*SESSIONS_CANCELLED;
    let _ = &*SESSIONS_EXPIRED;
    let _ = &*SESSIONS_ACTIVE;
    let _ = &*SESSIONS_STORED;
    let _ = &*SESSIONS_STARRED;
    let _ = &*SESSION_CONTENT_BYTES;
    let _ = &*SESSION_ANNOTATION_IMAGES;
    let _ = &*OCR_REQUESTS;
    let _ = &*OCR_DURATION;
    let _ = &*OCR_TOKENS;
    let _ = &*OCR_CACHE;
    let _ = &*OCR_PROVIDER_REQUESTS;
    let _ = &*OCR_PROVIDER_DURATION;
    let _ = &*OCR_PROVIDER_WINS;
    let _ = &*OCR_PROVIDER_TOKENS;
    let _ = &*OCR_IMAGE_PIXELS;
    let _ = &*WS_CONNECTIONS_ACTIVE;
    let _ = &*LONG_POLL_TOTAL;
    let _ = &*WEBHOOK_TOTAL;
    let _ = &*HTTP_REQUESTS;
    let _ = &*HTTP_DURATION;
    seed_label_vocabularies();
}

/// The OCR providers the race can pick between.
pub const PROVIDERS: [&str; 2] = ["local", "openai"];
/// How a provider attempt can end, as recorded by the race.
pub const PROVIDER_RESULTS: [&str; 3] = ["ok", "error", "cancelled"];

fn seed_label_vocabularies() {
    for result in ["hit", "miss", "coalesced"] {
        OCR_CACHE.with_label_values(&[result]).inc_by(0);
    }
    for result in ["ok", "error"] {
        OCR_REQUESTS.with_label_values(&[result]).inc_by(0);
    }

    for kind in ["prompt", "eval"] {
        OCR_TOKENS.with_label_values(&[kind]).inc_by(0);
    }
    for result in ["notified", "timeout"] {
        LONG_POLL_TOTAL.with_label_values(&[result]).inc_by(0);
    }
    for result in ["ok", "error"] {
        WEBHOOK_TOTAL.with_label_values(&[result]).inc_by(0);
    }
    for provider in PROVIDERS {
        OCR_PROVIDER_WINS.with_label_values(&[provider]).inc_by(0);
        for kind in ["input", "output"] {
            OCR_PROVIDER_TOKENS
                .with_label_values(&[provider, kind])
                .inc_by(0);
        }
        for result in PROVIDER_RESULTS {
            OCR_PROVIDER_REQUESTS
                .with_label_values(&[provider, result])
                .inc_by(0);
            // Not `observe(0.0)`: that would record a real observation and show up as a
            // dur of zero in the panel. Touching the child is enough to export it.
            let _ = OCR_PROVIDER_DURATION.with_label_values(&[provider, result]);
        }
    }
}

pub fn render() -> String {
    let encoder = TextEncoder::new();
    let families = prometheus::gather();
    let mut buf = Vec::new();
    encoder.encode(&families, &mut buf).unwrap_or_default();
    String::from_utf8(buf).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every gauge and counter with no labels, plus the histograms.
    #[test]
    fn unlabelled_metrics_are_present_before_any_event() {
        init();
        let exposed = render();
        for name in [
            "eink_sessions_created_total",
            "eink_sessions_submitted_total",
            "eink_sessions_cancelled_total",
            "eink_sessions_expired_total",
            "eink_sessions_active",
            "eink_sessions_stored",
            "eink_sessions_starred",
            "eink_session_content_bytes",
            "eink_session_annotation_images",
            "eink_ocr_duration_seconds",
            "eink_ocr_image_pixels",
            "eink_ws_connections_active",
        ] {
            assert!(
                exposed.contains(name),
                "{name} is not exported before its first event"
            );
        }
    }

    /// The labelled metrics, which need their vocabulary seeded: without it the family
    /// exports nothing and a scrape looks like the metric was deleted.
    #[test]
    fn labelled_metrics_export_their_whole_vocabulary_before_any_event() {
        init();
        let exposed = render();
        let mut wanted: Vec<String> = vec![
            "eink_ocr_requests_total{result=\"ok\"}".into(),
            "eink_ocr_requests_total{result=\"error\"}".into(),
            "eink_ocr_cache_total{result=\"hit\"}".into(),
            "eink_ocr_cache_total{result=\"miss\"}".into(),
            "eink_ocr_cache_total{result=\"coalesced\"}".into(),
            "eink_ocr_tokens_total{kind=\"prompt\"}".into(),
            "eink_ocr_tokens_total{kind=\"eval\"}".into(),
            "eink_long_poll_total{result=\"notified\"}".into(),
            "eink_long_poll_total{result=\"timeout\"}".into(),
            "eink_webhook_total{result=\"ok\"}".into(),
            "eink_webhook_total{result=\"error\"}".into(),
        ];
        for provider in PROVIDERS {
            wanted.push(format!(
                "eink_ocr_provider_wins_total{{provider=\"{provider}\"}}"
            ));
            for result in PROVIDER_RESULTS {
                wanted.push(format!(
                    "eink_ocr_provider_requests_total{{provider=\"{provider}\",result=\"{result}\"}}"
                ));
                wanted.push(format!(
                    "eink_ocr_provider_duration_seconds_count{{provider=\"{provider}\",result=\"{result}\"}}"
                ));
            }
            for kind in ["input", "output"] {
                wanted.push(format!(
                    "eink_ocr_provider_tokens_total{{provider=\"{provider}\",kind=\"{kind}\"}}"
                ));
            }
        }
        for series in wanted {
            let (name, labels) = series.split_once('{').expect("a labelled series");
            let labels = labels.trim_end_matches('}');
            let found = exposed.lines().any(|line| {
                line.starts_with(name) && labels.split(',').all(|pair| line.contains(pair))
            });
            assert!(
                found,
                "{series} is missing (label order is not significant): the panel that \
                 selects on it would read 'no data' until the first event"
            );
        }
    }

    /// The observation count of one histogram series.
    fn observations(label: &str) -> u64 {
        let exposed = render();
        exposed
            .lines()
            .find(|line| {
                line.starts_with(&format!("eink_ocr_provider_duration_seconds_count{label}"))
            })
            .and_then(|line| line.rsplit(' ').next())
            .and_then(|value| value.parse().ok())
            .expect("the series is exported")
    }

    #[test]
    fn seeding_does_not_invent_observations() {
        // A histogram child is created by touching it, but nothing may be *recorded*: a
        // seeded observation would put a spurious zero-duration sample in the panels.
        // Counted before and after rather than asserted to be zero, because other tests
        // in this binary legitimately observe durations.
        init();
        let label = "{provider=\"local\",result=\"ok\"}";
        let before = observations(label);
        seed_label_vocabularies();
        assert_eq!(
            observations(label),
            before,
            "seeding must create the series without recording a sample"
        );
    }

    #[test]
    fn the_metrics_the_runbook_names_are_the_ones_that_exist() {
        // METRICS.md tells the reader which metric to check for which symptom. A rename
        // that leaves the reference behind sends the next person to a metric that is gone.
        init();
        let exposed = render();
        // The HTTP metrics are labelled by path, so their family only appears once a
        // request has been made; the reference names them for that reason.
        const APPEAR_WITH_TRAFFIC: &[&str] = &[
            "eink_http_requests_total",
            "eink_http_request_duration_seconds",
        ];
        for name in documented_metrics(include_str!("../METRICS.md")) {
            if APPEAR_WITH_TRAFFIC.contains(&name.as_str()) {
                continue;
            }
            assert!(
                exposed.contains(&name),
                "the metrics reference names {name}, which no longer exists"
            );
        }
    }

    fn documented_metrics(doc: &str) -> Vec<String> {
        let mut found: Vec<String> = doc
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .filter(|word| word.starts_with("eink_") && word.len() > "eink_".len())
            .map(str::to_string)
            .collect();
        found.sort();
        found.dedup();
        found
    }

    #[test]
    fn documented_metrics_are_read_from_the_runbook() {
        // Guard the guard: if the parser stops finding names the test above passes empty.
        let found = documented_metrics(include_str!("../METRICS.md"));
        assert!(
            found.len() >= 6,
            "the reference should name several metrics, found {found:?}"
        );
    }

    #[test]
    fn a_metric_renders_its_labels_ready_for_a_query() {
        // A counter that never has its labels forced would render nothing at all, and the
        // Grafana panels that select on them would be empty.
        OCR_PROVIDER_REQUESTS
            .with_label_values(&["local", "ok"])
            .inc();
        LONG_POLL_TOTAL.with_label_values(&["timeout"]).inc();
        let exposed = render();
        assert!(exposed.contains("eink_ocr_provider_requests_total{"));
        assert!(exposed.contains("provider=\"local\""));
        assert!(exposed.contains("result=\"ok\""));
        assert!(exposed.contains("eink_long_poll_total{"));
    }
}
