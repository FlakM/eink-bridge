use base64::Engine as _;
use image::{ImageBuffer, Rgb, RgbImage};
use std::io::Cursor;
use std::time::Instant;
use tokio::sync::Mutex;
use tracing::info;

mod cache;
#[cfg(test)]
mod http_tests;
mod providers;
mod race;

pub use race::ProviderBenchmark;

pub struct OcrEngine {
    debug_save: bool,
    client: reqwest::Client,
    ollama_url: String,
    ollama_model: String,
    llama_url: Option<String>,
    openai: Option<providers::OpenAi>,
    cache: Mutex<cache::RecognitionCache>,
    /// How long a local model may take before the attempt is abandoned. A stalled model
    /// used to hold a caller for two minutes with nothing to show for it.
    local_timeout: std::time::Duration,
    /// The shorter bound for calls a reader is waiting on, which block a response.
    interactive_timeout: std::time::Duration,
}

/// Bounds for the local OCR calls, from the environment, with a usable default and a hard
/// ceiling. A zero would abandon every attempt before it was sent.
fn local_timeouts() -> (std::time::Duration, std::time::Duration) {
    local_timeouts_from(
        std::env::var("EINK_OCR_TIMEOUT_SECONDS").ok().as_deref(),
        std::env::var("EINK_OCR_INTERACTIVE_TIMEOUT_SECONDS")
            .ok()
            .as_deref(),
    )
}

/// The bounds, from values rather than from the environment, so the defaults are
/// something a test can hold on to: a batch call may wait two minutes, but a reader
/// holding a pen is waiting on the answer.
fn local_timeouts_from(
    general: Option<&str>,
    interactive: Option<&str>,
) -> (std::time::Duration, std::time::Duration) {
    (
        std::time::Duration::from_secs(providers::bounded_seconds(general, 120, 300)),
        std::time::Duration::from_secs(providers::bounded_seconds(interactive, 20, 120)),
    )
}

impl OcrEngine {
    pub fn new() -> Result<Self, String> {
        let debug_save = std::env::var("EINK_OCR_DEBUG").is_ok();
        let ollama_url = std::env::var("EINK_OLLAMA_URL")
            .unwrap_or_else(|_| "http://localhost:11434".to_string());
        let ollama_model =
            std::env::var("EINK_OLLAMA_MODEL").unwrap_or_else(|_| "qwen2.5vl:7b".to_string());
        let llama_url = std::env::var("EINK_LLAMA_OCR_URL")
            .ok()
            .filter(|url| !url.trim().is_empty());
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| format!("failed to build HTTP client: {e}"))?;
        info!(ollama_url = %ollama_url, ollama_model = %ollama_model, "OCR engine initialized");
        if let Some(url) = &llama_url {
            info!(url, "Dedicated local OCR runtime selected");
        }
        let (local_timeout, interactive_timeout) = local_timeouts();
        Ok(Self {
            debug_save,
            client,
            ollama_url,
            ollama_model,
            llama_url,
            openai: providers::OpenAi::from_env(),
            cache: Mutex::new(cache::RecognitionCache::default()),
            local_timeout,
            interactive_timeout,
        })
    }

    pub async fn recognize_image(&self, png: &[u8]) -> Result<String, String> {
        self.recognize_image_within(png, self.local_timeout).await
    }

    /// Recognize with an explicit bound, for a caller with a deadline of its own.
    pub async fn recognize_image_within(
        &self,
        png: &[u8],
        timeout: std::time::Duration,
    ) -> Result<String, String> {
        let t = Instant::now();
        let cell = self.cache.lock().await.entry(png, Instant::now());
        let hit = cell.get().is_some_and(Result::is_ok);
        let mut initialized = false;
        let result = cell
            .get_or_init(|| {
                initialized = true;
                self.recognize_uncached(png, timeout)
            })
            .await;
        if result.is_err() {
            // Existing waiters share this failure, but a later request may retry.
            self.cache.lock().await.discard(png, &cell);
        }
        crate::metrics::OCR_CACHE
            .with_label_values(&[if hit {
                "hit"
            } else if initialized {
                "miss"
            } else {
                "coalesced"
            }])
            .inc();
        crate::metrics::OCR_REQUESTS
            .with_label_values(&[if result.is_ok() { "ok" } else { "error" }])
            .inc();
        crate::metrics::OCR_DURATION.observe(t.elapsed().as_secs_f64());
        result
            .as_ref()
            .map(|value| {
                info!(
                    provider = value.provider,
                    cache_hit = hit,
                    wall_ms = t.elapsed().as_millis(),
                    "OCR complete"
                );
                value.text.clone()
            })
            .map_err(Clone::clone)
    }

    async fn recognize_uncached(
        &self,
        png: &[u8],
        timeout: std::time::Duration,
    ) -> Result<race::Recognition, String> {
        if self.debug_save {
            let _ = save_debug_png(png).map(|path| info!(path = %path, "OCR debug PNG saved"));
        }
        let image = base64::engine::general_purpose::STANDARD.encode(png);
        let local = race::observe("local", self.recognize_local(&image, timeout));
        let result = if let Some(openai) = &self.openai {
            race::first_valid(local, race::observe("openai", openai.recognize(&image))).await
        } else {
            local.await
        }?;
        crate::metrics::OCR_PROVIDER_WINS
            .with_label_values(&[result.provider])
            .inc();
        Ok(result)
    }

    async fn recognize_local(
        &self,
        image: &str,
        timeout: std::time::Duration,
    ) -> Result<String, String> {
        if let Some(url) = &self.llama_url {
            providers::llama(&self.client, url, image, timeout).await
        } else {
            providers::ollama(
                &self.client,
                &self.ollama_url,
                &self.ollama_model,
                image,
                timeout,
            )
            .await
        }
    }

    /// How long a call a reader is waiting on may take.
    pub fn interactive_timeout(&self) -> std::time::Duration {
        self.interactive_timeout
    }

    /// Explicit benchmark: bypasses the cache and lets both providers finish.
    /// Cloud calls still require EINK_OCR_OPENAI_ENABLED and a server-side key.
    pub async fn compare_image(&self, png: &[u8]) -> Vec<ProviderBenchmark> {
        let image = base64::engine::general_purpose::STANDARD.encode(png);
        let local = race::benchmark("local", self.recognize_local(&image, self.local_timeout));
        if let Some(openai) = &self.openai {
            let (local, cloud) =
                tokio::join!(local, race::benchmark("openai", openai.recognize(&image)));
            vec![local, cloud]
        } else {
            vec![local.await]
        }
    }

    pub async fn recognize_strokes(
        &self,
        strokes: &[Vec<[f64; 2]>],
        pressures: &[Vec<f64>],
    ) -> Result<String, String> {
        self.recognize_strokes_within(strokes, pressures, self.local_timeout)
            .await
    }

    /// Recognize strokes with an explicit bound, for the path where a reader is waiting
    /// on the answer.
    pub async fn recognize_strokes_within(
        &self,
        strokes: &[Vec<[f64; 2]>],
        pressures: &[Vec<f64>],
        timeout: std::time::Duration,
    ) -> Result<String, String> {
        if strokes.is_empty() {
            return Ok(String::new());
        }
        let total_points: usize = strokes.iter().map(|s| s.len()).sum();
        info!(
            strokes = strokes.len(),
            points = total_points,
            "OCR: rendering strokes"
        );
        let png = render_strokes_to_png(strokes, pressures)?;
        self.recognize_image_within(&png, timeout).await
    }
}

fn save_debug_png(png: &[u8]) -> Result<String, std::io::Error> {
    use std::io::Write as _;
    let path = format!(
        "/tmp/eink-ocr-{}.png",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );
    let mut f = std::fs::File::create(&path)?;
    f.write_all(png)?;
    Ok(path)
}

const BASE_SCALE: f64 = 2.0;
const MAX_SIDE: u32 = 512;
const PADDING: u32 = 15;
const MIN_DIM: u32 = 80;
const PRESSURE_MIN_SCALE: f64 = 0.4;
const PRESSURE_MAX_SCALE: f64 = 1.6;

fn effective_width(base: f64, pressure: f64) -> f64 {
    let p = if pressure.is_nan() {
        0.5
    } else {
        pressure.clamp(0.0, 1.0)
    };
    base * (PRESSURE_MIN_SCALE + (PRESSURE_MAX_SCALE - PRESSURE_MIN_SCALE) * p)
}

fn render_strokes_to_png(
    strokes: &[Vec<[f64; 2]>],
    pressures: &[Vec<f64>],
) -> Result<Vec<u8>, String> {
    let (mut min_x, mut min_y) = (f64::MAX, f64::MAX);
    let (mut max_x, mut max_y) = (f64::MIN, f64::MIN);

    for stroke in strokes {
        for &[x, y] in stroke {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }

    if min_x > max_x || min_y > max_y {
        return Err("no stroke points".into());
    }

    // Reserve padding before scaling, otherwise clamping the image dimensions
    // cuts off the final letters in wide or tall annotation groups.
    let max_extent = (max_x - min_x).max(max_y - min_y);
    let available = (MAX_SIDE - 2 * PADDING) as f64;
    let scale = if max_extent * BASE_SCALE > available {
        available / max_extent
    } else {
        BASE_SCALE
    };
    let stroke_width = 3.0 * scale;

    let w = ((max_x - min_x) * scale).ceil() as u32 + 2 * PADDING;
    let w = w.clamp(MIN_DIM, MAX_SIDE);
    let h = ((max_y - min_y) * scale).ceil() as u32 + 2 * PADDING;
    let h = h.clamp(MIN_DIM, MAX_SIDE);

    let mut img: RgbImage = ImageBuffer::from_pixel(w, h, Rgb([255, 255, 255]));

    for (si, stroke) in strokes.iter().enumerate() {
        let stroke_pressures = pressures.get(si);
        let has_pressure = stroke_pressures.is_some_and(|p| p.len() == stroke.len());
        for (i, window) in stroke.windows(2).enumerate() {
            let [x0, y0] = window[0];
            let [x1, y1] = window[1];
            let seg_width = if has_pressure {
                let ps = stroke_pressures.unwrap();
                let avg = (ps[i] + ps[i + 1]) * 0.5;
                effective_width(stroke_width, avg)
            } else {
                stroke_width
            };
            draw_thick_line(
                &mut img,
                (
                    (x0 - min_x) * scale + PADDING as f64,
                    (y0 - min_y) * scale + PADDING as f64,
                ),
                (
                    (x1 - min_x) * scale + PADDING as f64,
                    (y1 - min_y) * scale + PADDING as f64,
                ),
                seg_width,
            );
        }
    }

    let mut buf = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(Cursor::new(&mut buf));
    image::ImageEncoder::write_image(encoder, img.as_raw(), w, h, image::ExtendedColorType::Rgb8)
        .map_err(|e| format!("PNG encode failed: {e}"))?;
    crate::metrics::OCR_IMAGE_PIXELS.observe((w * h) as f64);
    info!(
        w,
        h,
        png_bytes = buf.len(),
        scale = scale,
        "OCR: PNG rendered"
    );
    Ok(buf)
}

fn draw_thick_line(img: &mut RgbImage, from: (f64, f64), to: (f64, f64), width: f64) {
    let (x0, y0) = from;
    let (x1, y1) = to;
    let dx = x1 - x0;
    let dy = y1 - y0;
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let steps = len.ceil() as usize;
    let half = width / 2.0;

    let (w, h) = (img.width() as f64, img.height() as f64);
    let black = Rgb([0u8, 0, 0]);

    for i in 0..=steps {
        let t = i as f64 / steps as f64;
        let cx = x0 + dx * t;
        let cy = y0 + dy * t;
        let r = half.ceil() as i32;
        for oy in -r..=r {
            for ox in -r..=r {
                if (ox * ox + oy * oy) as f64 <= half * half {
                    let px = cx as i32 + ox;
                    let py = cy as i32 + oy;
                    if px >= 0 && py >= 0 && (px as f64) < w && (py as f64) < h {
                        img.put_pixel(px as u32, py as u32, black);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mock_engine(
        response: serde_json::Value,
        model: &str,
    ) -> (OcrEngine, tokio::sync::oneshot::Receiver<serde_json::Value>) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let sender = std::sync::Arc::new(tokio::sync::Mutex::new(Some(tx)));
        let app = axum::Router::new().route(
            "/api/generate",
            axum::routing::post(move |axum::Json(body): axum::Json<serde_json::Value>| {
                let sender = sender.clone();
                let response = response.clone();
                async move {
                    sender.lock().await.take().unwrap().send(body).unwrap();
                    axum::Json(response)
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (
            OcrEngine {
                debug_save: false,
                client: reqwest::Client::new(),
                ollama_url: format!("http://{address}"),
                ollama_model: model.into(),
                llama_url: None,
                openai: None,
                cache: Mutex::new(cache::RecognitionCache::default()),
                local_timeout: std::time::Duration::from_secs(2),
                interactive_timeout: std::time::Duration::from_millis(200),
            },
            rx,
        )
    }

    #[tokio::test]
    async fn ocr_request_bounds_generation_without_cutting_off_markdown() {
        let (engine, request) = mock_engine(
            serde_json::json!({"response": "REMOVE IT", "done_reason": "stop"}),
            "qwen2.5vl:7b",
        )
        .await;
        assert_eq!(engine.recognize_image(b"png").await.unwrap(), "REMOVE IT");
        let body = request.await.unwrap();
        assert_eq!(body["model"], "qwen2.5vl:7b");
        assert!(body["prompt"].as_str().unwrap().contains("handwritten"));
        assert_eq!(body["options"]["num_predict"], 256);
        assert!(body["options"]["stop"].is_null());
        assert_eq!(body["images"][0], "cG5n");
    }

    #[tokio::test]
    async fn empty_or_truncated_model_output_is_not_successful_ocr() {
        for response in [
            serde_json::json!({"response": "   ", "done_reason": "stop"}),
            serde_json::json!({"response": "REMOVE REMOVE", "done_reason": "length"}),
            serde_json::json!({"error": "model failed"}),
        ] {
            let (engine, _request) = mock_engine(response, "glm-ocr").await;
            assert!(engine.recognize_image(b"png").await.is_err());
        }
    }

    #[tokio::test]
    async fn repeated_images_reuse_recognition_without_a_second_provider_call() {
        let (engine, request) = mock_engine(
            serde_json::json!({"response": "REMOVE IT", "done_reason": "stop"}),
            "qwen2.5vl:7b",
        )
        .await;
        assert_eq!(
            engine.recognize_image(b"same image").await.unwrap(),
            "REMOVE IT"
        );
        request.await.unwrap();
        assert_eq!(
            engine.recognize_image(b"same image").await.unwrap(),
            "REMOVE IT"
        );
    }

    #[test]
    fn large_stroke_groups_keep_padding_on_the_right_and_bottom() {
        for stroke in [
            vec![[0.0, 0.0], [1000.0, 100.0]],
            vec![[0.0, 0.0], [100.0, 1000.0]],
        ] {
            let png = render_strokes_to_png(&[stroke], &[]).unwrap();
            let image = image::load_from_memory(&png).unwrap().to_rgb8();
            let black: Vec<_> = image
                .enumerate_pixels()
                .filter(|(_, _, p)| p.0 == [0, 0, 0])
                .collect();
            let right = black.iter().map(|(x, _, _)| *x).max().unwrap();
            let bottom = black.iter().map(|(_, y, _)| *y).max().unwrap();
            assert!(
                right < image.width() - PADDING / 2,
                "right edge was clipped"
            );
            assert!(
                bottom < image.height() - PADDING / 2,
                "bottom edge was clipped"
            );
        }
    }

    #[test]
    fn render_strokes_produces_valid_png() {
        let strokes = vec![vec![[10.0, 10.0], [50.0, 10.0], [50.0, 50.0]]];
        let png = render_strokes_to_png(&strokes, &[]).unwrap();
        assert!(!png.is_empty());
        assert_eq!(&png[..4], &[0x89, 0x50, 0x4E, 0x47]);
    }

    #[test]
    fn render_empty_strokes_errors() {
        let strokes: Vec<Vec<[f64; 2]>> = vec![];
        assert!(render_strokes_to_png(&strokes, &[]).is_err());
    }

    #[test]
    fn render_single_point_stroke() {
        let strokes = vec![vec![[100.0, 100.0]]];
        let png = render_strokes_to_png(&strokes, &[]).unwrap();
        assert!(!png.is_empty());
    }

    fn count_black_pixels(png: &[u8]) -> usize {
        let img = image::load_from_memory(png).unwrap().to_rgb8();
        img.pixels().filter(|p| p.0 == [0, 0, 0]).count()
    }

    #[test]
    fn render_high_pressure_draws_more_pixels_than_low() {
        let stroke = vec![[10.0, 10.0], [60.0, 10.0], [60.0, 60.0], [10.0, 60.0]];
        let strokes = vec![stroke];
        let low = vec![vec![0.05_f64; 4]];
        let high = vec![vec![0.95_f64; 4]];
        let png_low = render_strokes_to_png(&strokes, &low).unwrap();
        let png_high = render_strokes_to_png(&strokes, &high).unwrap();
        let pixels_low = count_black_pixels(&png_low);
        let pixels_high = count_black_pixels(&png_high);
        assert!(
            pixels_high > pixels_low,
            "expected high-pressure render to have more black pixels than low ({pixels_high} !> {pixels_low})"
        );
    }

    #[test]
    fn render_mismatched_pressure_length_falls_back_to_constant_width() {
        let strokes = vec![vec![[10.0, 10.0], [50.0, 10.0], [50.0, 50.0]]];
        let mismatched = vec![vec![0.9_f64]]; // only 1 pressure for 3 points
        let png_ignored = render_strokes_to_png(&strokes, &mismatched).unwrap();
        let png_none = render_strokes_to_png(&strokes, &[]).unwrap();
        assert_eq!(
            count_black_pixels(&png_ignored),
            count_black_pixels(&png_none),
        );
    }

    #[test]
    fn effective_width_midpoint_equals_base() {
        let w = effective_width(10.0, 0.5);
        assert!((w - 10.0).abs() < 1e-9);
    }

    #[test]
    fn effective_width_clamps_out_of_range() {
        let low = effective_width(10.0, -1.0);
        let high = effective_width(10.0, 2.0);
        assert!((low - 4.0).abs() < 1e-9);
        assert!((high - 16.0).abs() < 1e-9);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ocr_engine_initializes() {
        match OcrEngine::new() {
            Ok(_) => {}
            Err(e) => eprintln!("OCR engine init skipped: {e}"),
        }
    }
}

#[cfg(test)]
mod bound_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn the_interactive_bound_is_shorter_than_the_batch_one_by_default() {
        let (general, interactive) = local_timeouts_from(None, None);
        assert_eq!(general, Duration::from_secs(120));
        assert_eq!(
            interactive,
            Duration::from_secs(20),
            "a reader waiting on OCR should not be held for the batch timeout"
        );
        assert!(interactive < general);
    }

    #[test]
    fn the_bounds_follow_the_environment_when_it_is_usable() {
        let (general, interactive) = local_timeouts_from(Some("300"), Some("90"));
        assert_eq!(general, Duration::from_secs(300));
        assert_eq!(interactive, Duration::from_secs(90));

        let (general, interactive) = local_timeouts_from(Some("45"), None);
        assert_eq!(general, Duration::from_secs(45));
        assert_eq!(interactive, Duration::from_secs(20));
    }

    #[test]
    fn an_unusable_bound_falls_back_rather_than_disabling_ocr() {
        for (general, interactive) in [
            (Some("0"), Some("9999")),
            (Some("abc"), Some("-5")),
            (Some("99999"), Some("0")),
        ] {
            let (general_secs, interactive_secs) = local_timeouts_from(general, interactive);
            assert_eq!(general_secs, Duration::from_secs(120), "from {general:?}");
            assert_eq!(
                interactive_secs,
                Duration::from_secs(20),
                "from {interactive:?}"
            );
        }
    }
}
