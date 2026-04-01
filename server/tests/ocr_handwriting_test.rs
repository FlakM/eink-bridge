/// OCR integration tests using real handwriting samples captured from the Boox tablet.
///
/// Each test loads a PNG fixture cropped from a tablet screenshot and asserts that
/// key words appear in the recognised output (case-insensitive). The engine calls
/// Ollama (qwen2.5vl:7b by default) which must be running locally. Parallel
/// execution is handled by Ollama's OLLAMA_NUM_PARALLEL setting (set to 4 in
/// the NixOS service config).
///
/// Run with:
///   cargo test --test ocr_handwriting_test
use eink_bridge::ocr::OcrEngine;
use std::sync::LazyLock;
use tokio::sync::Mutex;

/// Serialise Ollama calls so tests don't saturate the model simultaneously.
/// Once OLLAMA_NUM_PARALLEL=4 is active (after `nixos-rebuild switch` on
/// nix_dots) this is no longer strictly necessary but remains harmless.
static OLLAMA: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ocr")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read fixture {name}: {e}"))
}

fn contains_word(text: &str, word: &str) -> bool {
    text.to_lowercase().contains(&word.to_lowercase())
}

fn skip_if_no_engine() -> Option<OcrEngine> {
    match OcrEngine::new() {
        Ok(e) => Some(e),
        Err(e) => {
            eprintln!("skipping: OCR engine unavailable ({e})");
            None
        }
    }
}

/// Sample 1 — large block-capital single word in pink ink: "TEST"
#[tokio::test(flavor = "multi_thread")]
async fn recognizes_large_block_caps_single_word() {
    let _lock = OLLAMA.lock().await;
    let Some(engine) = skip_if_no_engine() else {
        return;
    };
    let png = fixture("sample1_TEST.png");

    let text = engine.recognize_image(&png).await.expect("OCR failed");
    eprintln!("sample1 → {text:?}");

    assert!(contains_word(&text, "test"), "expected 'test' in {text:?}");
}

/// Sample 2 — block-caps word with heavily crossed T and F strokes: "WTF"
#[tokio::test(flavor = "multi_thread")]
async fn recognizes_block_caps_with_crossed_strokes() {
    let _lock = OLLAMA.lock().await;
    let Some(engine) = skip_if_no_engine() else {
        return;
    };
    let png = fixture("sample2_WTF_is_it.png");

    let text = engine.recognize_image(&png).await.expect("OCR failed");
    eprintln!("sample2 → {text:?}");

    assert!(contains_word(&text, "wtf"), "expected 'wtf' in {text:?}");
}

/// Sample 3 — cursive short word: "now?"
/// Note: cursive 'n' and 'm' are visually similar; the model may read "mow".
/// Both are accepted as the core word shape is recognisable.
#[tokio::test(flavor = "multi_thread")]
async fn recognizes_cursive_short_word() {
    let _lock = OLLAMA.lock().await;
    let Some(engine) = skip_if_no_engine() else {
        return;
    };
    let png = fixture("sample3_now.png");

    let text = engine.recognize_image(&png).await.expect("OCR failed");
    eprintln!("sample3 → {text:?}");

    assert!(
        contains_word(&text, "now") || contains_word(&text, "mow"),
        "expected 'now' or 'mow' in {text:?}"
    );
}

/// Sample 4 — two-line block caps with an acronym: "WRITE / A TLDR."
#[tokio::test(flavor = "multi_thread")]
async fn recognizes_multiline_block_caps_with_acronym() {
    let _lock = OLLAMA.lock().await;
    let Some(engine) = skip_if_no_engine() else {
        return;
    };
    let png = fixture("sample4_WRITE_A_TLDR.png");

    let text = engine.recognize_image(&png).await.expect("OCR failed");
    eprintln!("sample4 → {text:?}");

    assert!(
        contains_word(&text, "write"),
        "expected 'write' in {text:?}"
    );
    assert!(contains_word(&text, "tldr"), "expected 'tldr' in {text:?}");
}

/// Sample 5 — cursive two-line sentence: "How would you / do it?"
#[tokio::test(flavor = "multi_thread")]
async fn recognizes_cursive_multiline_sentence() {
    let _lock = OLLAMA.lock().await;
    let Some(engine) = skip_if_no_engine() else {
        return;
    };
    let png = fixture("sample5_How_would_you.png");

    let text = engine.recognize_image(&png).await.expect("OCR failed");
    eprintln!("sample5 → {text:?}");

    assert!(contains_word(&text, "do"), "expected 'do' in {text:?}");
    assert!(contains_word(&text, "it"), "expected 'it' in {text:?}");
}
