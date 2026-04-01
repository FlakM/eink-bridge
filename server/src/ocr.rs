use image::{ImageBuffer, Rgb, RgbImage};
use std::io::Cursor;
use std::sync::Mutex;
use tracing::{debug, warn};

use crate::api::AnnotationGroup;

pub struct OcrEngine {
    tess: Mutex<leptess::LepTess>,
}

impl OcrEngine {
    pub fn new() -> Result<Self, String> {
        let tess = leptess::LepTess::new(None, "eng")
            .map_err(|e| format!("failed to init tesseract: {e}"))?;
        Ok(Self {
            tess: Mutex::new(tess),
        })
    }

    pub fn recognize_png(&self, png_data: &[u8]) -> Result<String, String> {
        let mut tess = self
            .tess
            .lock()
            .map_err(|e| format!("lock poisoned: {e}"))?;
        tess.set_image_from_mem(png_data)
            .map_err(|e| format!("failed to set image: {e}"))?;
        let text = tess
            .get_utf8_text()
            .map_err(|e| format!("OCR failed: {e}"))?;
        Ok(text.trim().to_string())
    }

    pub fn recognize_strokes(&self, strokes: &[Vec<[f64; 2]>]) -> Result<String, String> {
        if strokes.is_empty() {
            return Ok(String::new());
        }
        let png = render_strokes_to_png(strokes)?;
        self.recognize_png(&png)
    }
}

const STROKE_WIDTH: f64 = 3.0;
const PADDING: u32 = 20;
const MIN_DIM: u32 = 40;

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

    let w = ((max_x - min_x) as u32 + 2 * PADDING).max(MIN_DIM);
    let h = ((max_y - min_y) as u32 + 2 * PADDING).max(MIN_DIM);
    let w = w.min(4000);
    let h = h.min(4000);

    let mut img: RgbImage = ImageBuffer::from_pixel(w, h, Rgb([255, 255, 255]));

    for stroke in strokes {
        for window in stroke.windows(2) {
            let [x0, y0] = window[0];
            let [x1, y1] = window[1];
            draw_thick_line(
                &mut img,
                (x0 - min_x + PADDING as f64, y0 - min_y + PADDING as f64),
                (x1 - min_x + PADDING as f64, y1 - min_y + PADDING as f64),
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

pub fn ocr_annotation_groups(engine: &OcrEngine, groups: &mut [AnnotationGroup]) {
    for (i, group) in groups.iter_mut().enumerate() {
        if group.recognized_text.is_some() {
            continue;
        }
        if group.strokes.is_empty() {
            continue;
        }
        match engine.recognize_strokes(&group.strokes) {
            Ok(text) if !text.is_empty() => {
                debug!(group = i, text = %text, "OCR recognized text");
                group.recognized_text = Some(text);
            }
            Ok(_) => {
                debug!(group = i, "OCR returned empty text");
            }
            Err(e) => {
                warn!(group = i, error = %e, "OCR failed for annotation group");
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
        assert_eq!(&png[..4], &[0x89, 0x50, 0x4E, 0x47]); // PNG magic
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

    #[test]
    fn ocr_engine_initializes() {
        match OcrEngine::new() {
            Ok(_) => {}
            Err(e) => {
                // Tesseract may not be available in all test environments
                eprintln!("OCR engine init skipped: {e}");
            }
        }
    }

    #[test]
    fn ocr_skips_groups_with_existing_text() {
        let engine = match OcrEngine::new() {
            Ok(e) => e,
            Err(_) => return, // skip if tesseract unavailable
        };
        let mut groups = vec![AnnotationGroup {
            anchor: None,
            strokes: vec![vec![[0.0, 0.0], [100.0, 0.0]]],
            recognized_text: Some("already set".into()),
        }];
        ocr_annotation_groups(&engine, &mut groups);
        assert_eq!(groups[0].recognized_text.as_deref(), Some("already set"));
    }
}
