use base64::Engine as _;
use image::{ImageBuffer, Rgb, RgbImage};
use std::io::Cursor;
use std::time::Instant;
use tracing::info;

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
            std::env::var("EINK_OLLAMA_MODEL").unwrap_or_else(|_| "glm-ocr".to_string());
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
        let b64 = base64::engine::general_purpose::STANDARD.encode(png);
        let body = serde_json::json!({
            "model": self.ollama_model,
            "prompt": "Transcribe the handwritten text in this image exactly as written. Output only the transcribed text, nothing else.",
            "images": [b64],
            "stream": false,
        });
        let resp = match self
            .client
            .post(format!("{}/api/generate", self.ollama_url))
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                crate::metrics::OCR_REQUESTS
                    .with_label_values(&["error"])
                    .inc();
                return Err(format!("Ollama request: {e}"));
            }
        };
        if !resp.status().is_success() {
            crate::metrics::OCR_REQUESTS
                .with_label_values(&["error"])
                .inc();
            return Err(format!("Ollama returned {}", resp.status()));
        }
        let json: serde_json::Value = match resp.json().await {
            Ok(j) => j,
            Err(e) => {
                crate::metrics::OCR_REQUESTS
                    .with_label_values(&["error"])
                    .inc();
                return Err(format!("Ollama parse: {e}"));
            }
        };
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
        let eval_tokens = json["eval_count"].as_u64().unwrap_or(0);
        let prompt_tokens = json["prompt_eval_count"].as_u64().unwrap_or(0);
        info!(
            text = %text,
            wall_ms = elapsed.as_millis(),
            prompt_eval_ms = prompt_ms,
            token_eval_ms = eval_ms,
            "OCR complete"
        );
        crate::metrics::OCR_REQUESTS
            .with_label_values(&["ok"])
            .inc();
        crate::metrics::OCR_DURATION.observe(elapsed.as_secs_f64());
        crate::metrics::OCR_TOKENS
            .with_label_values(&["prompt"])
            .inc_by(prompt_tokens);
        crate::metrics::OCR_TOKENS
            .with_label_values(&["eval"])
            .inc_by(eval_tokens);
        Ok(text)
    }

    pub async fn recognize_strokes(
        &self,
        strokes: &[Vec<[f64; 2]>],
        pressures: &[Vec<f64>],
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
        self.recognize_image(&png).await
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

    // Cap scale so the longer side stays within MAX_SIDE
    let natural_w = (max_x - min_x) * BASE_SCALE + 2.0 * PADDING as f64;
    let natural_h = (max_y - min_y) * BASE_SCALE + 2.0 * PADDING as f64;
    let scale = if natural_w.max(natural_h) > MAX_SIDE as f64 {
        BASE_SCALE * MAX_SIDE as f64 / natural_w.max(natural_h)
    } else {
        BASE_SCALE
    };
    let stroke_width = 3.0 * scale;

    let w = ((max_x - min_x) * scale) as u32 + 2 * PADDING;
    let w = w.clamp(MIN_DIM, MAX_SIDE);
    let h = ((max_y - min_y) * scale) as u32 + 2 * PADDING;
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
