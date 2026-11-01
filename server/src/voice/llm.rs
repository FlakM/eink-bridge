//! The rewrites a closed verb cannot do deterministically ("shorter", "make it a
//! question", anything free-form) go to the house model. Nothing it says is believed
//! until `clean::guard` has checked that it invented no word.
use crate::ocr::providers::bounded_seconds;
use reqwest::Client;
use serde_json::{Value, json};
use std::time::Duration;

pub struct Cleaner {
    client: Client,
    url: String,
    model: String,
    timeout: Duration,
}

impl Cleaner {
    /// `EINK_VOICE_LLM_URL`/`EINK_VOICE_LLM_MODEL`, falling back to the OCR model's
    /// Ollama, which is the one model a home server has. `EINK_VOICE_LLM_ENABLED=0`
    /// turns it off, and the free verbs then say so instead of guessing.
    pub fn from_env() -> Option<Self> {
        if matches!(
            std::env::var("EINK_VOICE_LLM_ENABLED").ok().as_deref(),
            Some("0" | "false")
        ) {
            return None;
        }
        let url = std::env::var("EINK_VOICE_LLM_URL")
            .or_else(|_| std::env::var("EINK_OLLAMA_URL"))
            .unwrap_or_else(|_| "http://localhost:11434".into());
        let model = std::env::var("EINK_VOICE_LLM_MODEL")
            .or_else(|_| std::env::var("EINK_OLLAMA_MODEL"))
            .unwrap_or_else(|_| "qwen2.5vl:7b".into());
        let timeout = Duration::from_secs(bounded_seconds(
            std::env::var("EINK_VOICE_LLM_TIMEOUT_SECONDS")
                .ok()
                .as_deref(),
            20,
            120,
        ));
        Some(Self::new(url, model, timeout))
    }

    pub fn new(url: String, model: String, timeout: Duration) -> Self {
        Self {
            client: Client::builder()
                .timeout(timeout + Duration::from_secs(1))
                .build()
                .expect("a default HTTP client can be built"),
            url,
            model,
            timeout,
        }
    }

    pub async fn rewrite(&self, text: &str, instruction: &str) -> Result<String, String> {
        let prompt = format!(
            "Rewrite the note below following the instruction. Rules: add nothing the note does not say; \
             keep every identifier, file name and number exactly as written; keep hedges such as \
             \"I think\" and \"maybe\"; do not summarise, translate or reorder unless the instruction asks. \
             Output only the rewritten note.\n\nInstruction: {instruction}\n\nNote: {text}"
        );
        let response = self
            .client
            .post(format!("{}/api/generate", self.url.trim_end_matches('/')))
            .timeout(self.timeout)
            .json(&json!({
                "model": self.model, "prompt": prompt, "stream": false,
                "options": {"temperature": 0, "num_predict": 256},
            }))
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    "the cleaner timed out".to_string()
                } else {
                    "the cleaner is not answering".to_string()
                }
            })?;
        if !response.status().is_success() {
            return Err(format!(
                "the cleaner returned HTTP {}",
                response.status().as_u16()
            ));
        }
        let body: Value = response
            .json()
            .await
            .map_err(|_| "the cleaner returned invalid JSON".to_string())?;
        if body.get("error").is_some() {
            return Err("the cleaner failed".into());
        }
        let text = body["response"]
            .as_str()
            .map(|t| t.trim().trim_matches('"').trim().to_string())
            .filter(|t| !t.is_empty())
            .ok_or_else(|| "the cleaner answered with nothing".to_string())?;
        Ok(text)
    }
}
