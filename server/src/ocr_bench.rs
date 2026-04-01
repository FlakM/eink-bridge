/// OCR benchmark — measures latency of the Ollama vision model for each
/// handwriting fixture, sequentially then in parallel (bounded by
/// OLLAMA_NUM_PARALLEL, default 4).
///
/// Run with:
///   cargo run --bin eink-ocr-bench
use eink_bridge::ocr::OcrEngine;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

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

#[tokio::main]
async fn main() {
    let parallelism: usize = std::env::var("OLLAMA_NUM_PARALLEL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4);

    let engine = match OcrEngine::new() {
        Ok(e) => Arc::new(e),
        Err(e) => {
            eprintln!("OCR engine unavailable: {e}");
            std::process::exit(1);
        }
    };

    // ── Sequential ────────────────────────────────────────────────────────────
    println!("=== Sequential OCR ({} samples) ===", SAMPLES.len());
    println!("{:<22} {:>9}  {}", "sample", "wall ms", "recognized text");
    println!("{}", "─".repeat(70));

    let mut seq_times: Vec<u128> = Vec::new();
    for (filename, label) in SAMPLES {
        let png = fixture(filename);
        let t = Instant::now();
        match engine.recognize_image(&png).await {
            Ok(text) => {
                let ms = t.elapsed().as_millis();
                seq_times.push(ms);
                let preview = text.replace('\n', " ");
                let preview = if preview.len() > 40 {
                    format!("{}…", &preview[..40])
                } else {
                    preview
                };
                println!("{:<22} {:>9}  \"{}\"", label, ms, preview);
            }
            Err(e) => println!("{:<22}  ERROR — {e}", label),
        }
    }
    let seq_total: u128 = seq_times.iter().sum();
    let seq_min = seq_times.iter().min().copied().unwrap_or(0);
    let seq_max = seq_times.iter().max().copied().unwrap_or(0);
    let seq_mean = seq_total / seq_times.len() as u128;
    println!("{}", "─".repeat(70));
    println!("total={seq_total} ms   min={seq_min} ms   mean={seq_mean} ms   max={seq_max} ms");

    // ── Parallel ─────────────────────────────────────────────────────────────
    println!();
    println!(
        "=== Parallel OCR ({} samples, {} concurrent) ===",
        SAMPLES.len(),
        parallelism
    );
    println!("{:<22} {:>9}  {}", "sample", "wall ms", "recognized text");
    println!("{}", "─".repeat(70));

    let sem = Arc::new(Semaphore::new(parallelism));
    let mut set = JoinSet::new();
    let par_start = Instant::now();

    for (filename, label) in SAMPLES {
        let engine = engine.clone();
        let png = fixture(filename);
        let label = label.to_string();
        let sem = sem.clone();
        set.spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            let t = Instant::now();
            let result = engine.recognize_image(&png).await;
            (label, t.elapsed().as_millis(), result)
        });
    }

    let mut par_results = Vec::new();
    while let Some(res) = set.join_next().await {
        par_results.push(res.unwrap());
    }
    let par_wall_ms = par_start.elapsed().as_millis();
    par_results.sort_by_key(|(_, ms, _)| *ms);

    let mut par_times: Vec<u128> = Vec::new();
    for (label, ms, result) in &par_results {
        match result {
            Ok(text) => {
                par_times.push(*ms);
                let preview = text.replace('\n', " ");
                let preview = if preview.len() > 40 {
                    format!("{}…", &preview[..40])
                } else {
                    preview
                };
                println!("{:<22} {:>9}  \"{}\"", label, ms, preview);
            }
            Err(e) => println!("{:<22}  ERROR — {e}", label),
        }
    }

    println!("{}", "─".repeat(70));
    println!(
        "wall clock={par_wall_ms} ms   speedup vs sequential: {:.1}x",
        seq_total as f64 / par_wall_ms as f64
    );
}
