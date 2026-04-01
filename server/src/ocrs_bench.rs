/// ocrs (local neural OCR) benchmark — printed-text engine, no Ollama needed.
/// Models: ~/.local/share/eink-bridge/models/{text-detection,text-recognition}.rten
///
/// Run with:
///   cargo run --bin eink-ocrs-bench
use ocrs::{ImageSource, OcrEngine, OcrEngineParams};
use rten::Model;
use std::path::Path;
use std::time::Instant;

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ocr")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read fixture {name}: {e}"))
}

const SAMPLES: &[(&str, &str)] = &[
    ("sample1_TEST.png", "TEST"),
    ("sample2_WTF_is_it.png", "WTF"),
    ("sample3_now.png", "now?"),
    ("sample4_WRITE_A_TLDR.png", "WRITE A TLDR"),
    ("sample5_How_would_you.png", "How would you do it?"),
];

fn model_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home)
        .join(".local/share/eink-bridge/models")
}

fn ocr_png(engine: &OcrEngine, png: &[u8]) -> anyhow::Result<String> {
    let img = image::load_from_memory(png)?.into_rgb8();
    let (w, h) = img.dimensions();
    let src = ImageSource::from_bytes(img.as_raw(), (w, h))?;
    let input = engine.prepare_input(src)?;
    let words = engine.detect_words(&input)?;
    let lines = engine.find_text_lines(&input, &words);
    let recognized = engine.recognize_text(&input, &lines)?;
    let text = recognized
        .iter()
        .flatten()
        .map(|l| l.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    Ok(text)
}

fn main() -> anyhow::Result<()> {
    let dir = model_dir();
    let det_path = dir.join("text-detection.rten");
    let rec_path = dir.join("text-recognition.rten");

    if !det_path.exists() || !rec_path.exists() {
        eprintln!("Models not found in {}", dir.display());
        eprintln!("Download with:");
        eprintln!("  curl https://ocrs-models.s3-accelerate.amazonaws.com/text-detection.rten -o {}", det_path.display());
        eprintln!("  curl https://ocrs-models.s3-accelerate.amazonaws.com/text-recognition.rten -o {}", rec_path.display());
        std::process::exit(1);
    }

    let t_load = Instant::now();
    let engine = OcrEngine::new(OcrEngineParams {
        detection_model: Some(Model::load_file(&det_path)?),
        recognition_model: Some(Model::load_file(&rec_path)?),
        ..Default::default()
    })?;
    println!("models loaded in {}ms", t_load.elapsed().as_millis());
    println!();
    println!("=== ocrs OCR ({} samples) ===", SAMPLES.len());
    println!("{:<22} {:>9}  {}", "sample", "wall ms", "recognized text");
    println!("{}", "─".repeat(70));

    let mut times: Vec<u128> = Vec::new();
    for (filename, label) in SAMPLES {
        let png = fixture(filename);
        let t = Instant::now();
        match ocr_png(&engine, &png) {
            Ok(text) => {
                let ms = t.elapsed().as_millis();
                times.push(ms);
                let preview = if text.len() > 40 {
                    format!("{}…", &text[..40])
                } else {
                    text.clone()
                };
                println!("{:<22} {:>9}  {:?}", label, ms, preview);
            }
            Err(e) => println!("{:<22}  ERROR — {e}", label),
        }
    }

    println!("{}", "─".repeat(70));
    let total: u128 = times.iter().sum();
    let mean = total / times.len() as u128;
    let min = times.iter().min().copied().unwrap_or(0);
    let max = times.iter().max().copied().unwrap_or(0);
    println!("total={total}ms  min={min}ms  mean={mean}ms  max={max}ms");
    Ok(())
}
