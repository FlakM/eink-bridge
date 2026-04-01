use base64::Engine as _;
use image::{ImageBuffer, Rgb, RgbImage};
use std::io::Cursor;
use std::time::Instant;
use tracing::{debug, info, warn};

use crate::api::AnnotationGroup;

pub struct OcrEngine {
    debug_save: bool,
    client: reqwest::Client,
    ollama_url: String,
    ollama_model: String,
}

impl OcrEngine {
    pub fn new() -> Result<Self, String> {
        let debug_save = std::env::var("EINK_OCR_DEBUG").is_ok();
        let ollama_url = std::env::var("EINK_OLLAMA_URL")
            .unwrap_or_else(|_| "http://localhost:11434".to_string());
        let ollama_model =
            std::env::var("EINK_OLLAMA_MODEL").unwrap_or_else(|_| "qwen2.5vl:7b".to_string());
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| format!("failed to build HTTP client: {e}"))?;
        info!(ollama_url = %ollama_url, ollama_model = %ollama_model, "OCR engine initialized");
        Ok(Self {
            debug_save,
            client,
            ollama_url,
            ollama_model,
        })
    }

    pub async fn recognize_image(&self, png: &[u8]) -> Result<String, String> {
        if self.debug_save {
            let _ = save_debug_png(png).map(|path| info!(path = %path, "OCR debug PNG saved"));
        }
        let t = Instant::now();
        let png = preprocess_for_qwen(png)?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
        let body = serde_json::json!({
            "model": self.ollama_model,
            "prompt": "Transcribe the handwritten text in this image exactly as written. Output only the transcribed text, nothing else.",
            "images": [b64],
            "stream": false,
        });
        let resp = self
            .client
            .post(format!("{}/api/generate", self.ollama_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Ollama request: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("Ollama returned {}", resp.status()));
        }
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Ollama parse: {e}"))?;
        let text = json["response"].as_str().unwrap_or("").trim().to_string();
        let elapsed = t.elapsed();
        let eval_ms = json["eval_duration"]
            .as_u64()
            .map(|ns| ns / 1_000_000)
            .unwrap_or(0);
        let prompt_ms = json["prompt_eval_duration"]
            .as_u64()
            .map(|ns| ns / 1_000_000)
            .unwrap_or(0);
        info!(
            text = %text,
            wall_ms = elapsed.as_millis(),
            prompt_eval_ms = prompt_ms,
            token_eval_ms = eval_ms,
            "OCR complete"
        );
        Ok(text)
    }

    pub async fn recognize_strokes(&self, strokes: &[Vec<[f64; 2]>]) -> Result<String, String> {
        if strokes.is_empty() {
            return Ok(String::new());
        }
        let total_points: usize = strokes.iter().map(|s| s.len()).sum();
        info!(
            strokes = strokes.len(),
            points = total_points,
            "OCR: rendering strokes"
        );
        let png = render_strokes_to_png(strokes)?;
        info!(png_bytes = png.len(), "OCR: PNG rendered");
        self.recognize_image(&png).await
    }
}

pub async fn ocr_annotation_groups(engine: &OcrEngine, groups: &mut [AnnotationGroup]) {
    for (i, group) in groups.iter_mut().enumerate() {
        if group.recognized_text.is_some() || group.strokes.is_empty() {
            continue;
        }
        match engine.recognize_strokes(&group.strokes).await {
            Ok(text) if !text.is_empty() => {
                debug!(group = i, text = %text, "OCR recognized text");
                group.recognized_text = Some(text);
            }
            Ok(_) => debug!(group = i, "OCR returned empty text"),
            Err(e) => warn!(group = i, error = %e, "OCR failed for annotation group"),
        }
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

/// qwen2.5vl vision encoder parameters:
/// - patch_size = 14px, spatial_merge_size = 2 → effective token = 28×28 px
/// - fullatt_block_indexes = [7,15,23,31] (4 of 32 blocks use full attention)
/// - window_size = 112 (8×8 patches for local attention)
///
/// Preprocessing rules (from qwen2.5vl smart_resize spec):
///   1. Round each dimension to nearest multiple of PATCH_TOKENS (28)
///   2. If total pixels > MAX_PIXELS, scale down (floor to multiple of 28)
///   3. Both dimensions must be ≥ MIN_PIXELS^0.5 = 56px
///
/// MAX_PIXELS = 1344×1344 = 1,806,336 → max 2,304 visual tokens
/// (stays well under the 4,840 that caused 6.9GB Vulkan allocation failures)
const PATCH_TOKENS: u32 = 28;
const MAX_PIXELS: u64 = 1344 * 1344;
const MIN_DIM_QWEN: u32 = 56;

fn preprocess_for_qwen(png: &[u8]) -> Result<Vec<u8>, String> {
    let img = image::load_from_memory(png)
        .map_err(|e| format!("PNG decode failed: {e}"))?
        .into_rgb8();
    let (w, h) = img.dimensions();

    // Round to nearest multiple of PATCH_TOKENS
    let snap = |v: u32| -> u32 {
        let r = (v + PATCH_TOKENS / 2) / PATCH_TOKENS * PATCH_TOKENS;
        r.max(MIN_DIM_QWEN)
    };
    let mut tw = snap(w);
    let mut th = snap(h);

    // Scale down if pixel budget exceeded
    if (tw as u64) * (th as u64) > MAX_PIXELS {
        let scale = (MAX_PIXELS as f64 / (w as f64 * h as f64)).sqrt();
        tw = ((w as f64 * scale / PATCH_TOKENS as f64).floor() as u32 * PATCH_TOKENS)
            .max(MIN_DIM_QWEN);
        th = ((h as f64 * scale / PATCH_TOKENS as f64).floor() as u32 * PATCH_TOKENS)
            .max(MIN_DIM_QWEN);
    }

    let tokens = (tw / PATCH_TOKENS) * (th / PATCH_TOKENS);
    let img = if tw != w || th != h {
        debug!(src_w = w, src_h = h, dst_w = tw, dst_h = th, tokens, "OCR: smart-resize");
        image::imageops::resize(&img, tw, th, image::imageops::FilterType::Lanczos3)
    } else {
        debug!(w, h, tokens, "OCR: image already optimal");
        img
    };

    let mut buf = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(std::io::Cursor::new(&mut buf));
    image::ImageEncoder::write_image(encoder, img.as_raw(), tw, th, image::ExtendedColorType::Rgb8)
        .map_err(|e| format!("PNG encode failed: {e}"))?;
    Ok(buf)
}

const SCALE: f64 = 3.0;
const STROKE_WIDTH: f64 = 3.0 * SCALE;
const PADDING: u32 = 20;
const MIN_DIM: u32 = 120;

fn render_strokes_to_png(strokes: &[Vec<[f64; 2]>]) -> Result<Vec<u8>, String> {
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

    let w = (((max_x - min_x) * SCALE) as u32 + 2 * PADDING).max(MIN_DIM);
    let h = (((max_y - min_y) * SCALE) as u32 + 2 * PADDING).max(MIN_DIM);
    let w = w.min(8000);
    let h = h.min(8000);

    let mut img: RgbImage = ImageBuffer::from_pixel(w, h, Rgb([255, 255, 255]));

    for stroke in strokes {
        for window in stroke.windows(2) {
            let [x0, y0] = window[0];
            let [x1, y1] = window[1];
            draw_thick_line(
                &mut img,
                (
                    (x0 - min_x) * SCALE + PADDING as f64,
                    (y0 - min_y) * SCALE + PADDING as f64,
                ),
                (
                    (x1 - min_x) * SCALE + PADDING as f64,
                    (y1 - min_y) * SCALE + PADDING as f64,
                ),
                STROKE_WIDTH,
            );
        }
    }

    let mut buf = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(Cursor::new(&mut buf));
    image::ImageEncoder::write_image(encoder, img.as_raw(), w, h, image::ExtendedColorType::Rgb8)
        .map_err(|e| format!("PNG encode failed: {e}"))?;
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

    #[test]
    fn render_strokes_produces_valid_png() {
        let strokes = vec![vec![[10.0, 10.0], [50.0, 10.0], [50.0, 50.0]]];
        let png = render_strokes_to_png(&strokes).unwrap();
        assert!(!png.is_empty());
        assert_eq!(&png[..4], &[0x89, 0x50, 0x4E, 0x47]);
    }

    #[test]
    fn render_empty_strokes_errors() {
        let strokes: Vec<Vec<[f64; 2]>> = vec![];
        assert!(render_strokes_to_png(&strokes).is_err());
    }

    #[test]
    fn render_single_point_stroke() {
        let strokes = vec![vec![[100.0, 100.0]]];
        let png = render_strokes_to_png(&strokes).unwrap();
        assert!(!png.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ocr_engine_initializes() {
        match OcrEngine::new() {
            Ok(_) => {}
            Err(e) => eprintln!("OCR engine init skipped: {e}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ocr_skips_groups_with_existing_text() {
        let engine = match OcrEngine::new() {
            Ok(e) => e,
            Err(_) => return,
        };
        let mut groups = vec![AnnotationGroup {
            anchor: None,
            strokes: vec![vec![[0.0, 0.0], [100.0, 0.0]]],
            recognized_text: Some("already set".into()),
        }];
        ocr_annotation_groups(&engine, &mut groups).await;
        assert_eq!(groups[0].recognized_text.as_deref(), Some("already set"));
    }
}
