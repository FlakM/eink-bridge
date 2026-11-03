//! The rewrites a closed verb cannot do deterministically ("shorter", "make it a
//! question", anything free-form) go to a model. Nothing it says is believed until
//! `clean::guard` has checked that it invented no word.
//!
//! Which model is a speed decision, not a quality one: a rewrite is on the reader's
//! critical path (they are standing there waiting to hear whether the note reads right),
//! and the house Ollama runs a 7B vision model on the same CPU that is transcribing. The
//! hosted model answers in about a second; the local one in tens of seconds. Ollama stays
//! as the fallback so a machine with no key still works.
use crate::ocr::providers::{bounded_seconds, openai_key};
use reqwest::{
    Client,
    header::{AUTHORIZATION, HeaderValue},
};
use serde_json::{Value, json};
use std::time::Duration;

pub struct Cleaner {
    client: Client,
    backend: Backend,
    timeout: Duration,
}

enum Backend {
    /// Ollama's own API, which is what a home server has.
    Ollama { url: String, model: String },
    /// OpenAI's chat completions. No Debug, and the header is marked sensitive: the key
    /// must never reach a log.
    OpenAi {
        url: String,
        model: String,
        authorization: HeaderValue,
    },
}

impl Cleaner {
    /// `EINK_VOICE_LLM_PROVIDER` picks `openai` or `ollama`.
    ///
    /// The default follows whether a key is configured: a machine that already opted into
    /// OpenAI for OCR gets the fast cleaner too, because the reader is waiting on it. With
    /// no key it stays on the house Ollama. `EINK_VOICE_LLM_ENABLED=0` turns the cleaner
    /// off entirely, and the verbs that need it then say so instead of guessing.
    pub fn from_env() -> Option<Self> {
        if matches!(
            std::env::var("EINK_VOICE_LLM_ENABLED").ok().as_deref(),
            Some("0" | "false")
        ) {
            return None;
        }
        let timeout = Duration::from_secs(bounded_seconds(
            std::env::var("EINK_VOICE_LLM_TIMEOUT_SECONDS")
                .ok()
                .as_deref(),
            20,
            120,
        ));
        let local = || {
            let url = std::env::var("EINK_VOICE_LLM_URL")
                .or_else(|_| std::env::var("EINK_OLLAMA_URL"))
                .unwrap_or_else(|_| "http://localhost:11434".into());
            let model = std::env::var("EINK_VOICE_LLM_MODEL")
                .or_else(|_| std::env::var("EINK_OLLAMA_MODEL"))
                .unwrap_or_else(|_| "qwen2.5vl:7b".into());
            Self::new(url, model, timeout)
        };

        let wants_hosted = match std::env::var("EINK_VOICE_LLM_PROVIDER") {
            Ok(value) => value.trim().eq_ignore_ascii_case("openai"),
            Err(_) => true,
        };
        if wants_hosted {
            if let Some(cleaner) = openai_key().and_then(|key| {
                let model = std::env::var("EINK_VOICE_LLM_MODEL")
                    .or_else(|_| std::env::var("EINK_OPENAI_MODEL"))
                    .unwrap_or_else(|_| "gpt-4.1-mini".into());
                Self::openai(&key, model, timeout).ok()
            }) {
                tracing::info!(model = %cleaner.model_name(), "voice: cleaner is the hosted model");
                return Some(cleaner);
            }
            tracing::info!("voice: no hosted key; the cleaner stays on the local model");
        }
        let cleaner = local();
        tracing::info!(model = %cleaner.model_name(), "voice: cleaner is the local model");
        Some(cleaner)
    }

    /// The local backend, unchanged: Ollama's own API.
    pub fn new(url: String, model: String, timeout: Duration) -> Self {
        Self::build(Backend::Ollama { url, model }, timeout)
    }

    /// The hosted backend, for a machine with a key.
    pub fn openai(key: &str, model: String, timeout: Duration) -> Result<Self, String> {
        let mut authorization = HeaderValue::from_str(&format!("Bearer {key}"))
            .map_err(|_| "the OpenAI key cannot be a header")?;
        authorization.set_sensitive(true);
        Ok(Self::build(
            Backend::OpenAi {
                url: std::env::var("EINK_VOICE_LLM_URL")
                    .unwrap_or_else(|_| "https://api.openai.com/v1".into()),
                model,
                authorization,
            },
            timeout,
        ))
    }

    fn build(backend: Backend, timeout: Duration) -> Self {
        Self {
            client: Client::builder()
                .timeout(timeout + Duration::from_secs(1))
                .connect_timeout(Duration::from_secs(3))
                .build()
                .expect("a default HTTP client can be built"),
            backend,
            timeout,
        }
    }

    fn model_name(&self) -> &str {
        match &self.backend {
            Backend::Ollama { model, .. } | Backend::OpenAi { model, .. } => model,
        }
    }

    pub async fn rewrite(&self, text: &str, instruction: &str) -> Result<String, String> {
        let prompt = format!(
            "Rewrite the note below following the instruction. Rules: add nothing the note does not say; \
             keep every identifier, file name and number exactly as written; keep hedges such as \
             \"I think\", \"maybe\", \"chyba\", \"może\"; write in the language the note is already in and \
             never translate it; do not summarise or reorder unless the instruction asks. \
             Output only the rewritten note.\n\nInstruction: {instruction}\n\nNote: {text}"
        );
        let request = match &self.backend {
            Backend::Ollama { url, model } => self
                .client
                .post(format!("{}/api/generate", url.trim_end_matches('/')))
                .json(&json!({
                    "model": model, "prompt": prompt, "stream": false,
                    "options": {"temperature": 0, "num_predict": 256},
                })),
            Backend::OpenAi {
                url,
                model,
                authorization,
            } => self
                .client
                .post(format!("{}/chat/completions", url.trim_end_matches('/')))
                .header(AUTHORIZATION, authorization.clone())
                .json(&json!({
                    "model": model,
                    "temperature": 0,
                    "max_tokens": 256,
                    "messages": [{"role": "user", "content": prompt}],
                })),
        };
        let response = request.timeout(self.timeout).send().await.map_err(|e| {
            if e.is_timeout() {
                "the cleaner timed out".to_string()
            } else {
                "the cleaner is not answering".to_string()
            }
        })?;
        if !response.status().is_success() {
            // Never log the body: a hosted error may quote the note back.
            return Err(format!(
                "the cleaner returned HTTP {}",
                response.status().as_u16()
            ));
        }
        let body: Value = response
            .json()
            .await
            .map_err(|_| "the cleaner returned invalid JSON".to_string())?;
        parse_rewrite(&body)
    }
}

/// The rewritten note, whichever API answered it. Both shapes are one string under a
/// different path, and both need the same tidying: a model that wraps its answer in quotes
/// would otherwise put them in the reader's note.
fn parse_rewrite(body: &Value) -> Result<String, String> {
    if body.get("error").is_some() {
        return Err("the cleaner failed".into());
    }
    let text = body["response"]
        .as_str()
        .or_else(|| body["choices"][0]["message"]["content"].as_str())
        .map(|t| t.trim().trim_matches('"').trim().to_string())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| "the cleaner answered with nothing".to_string())?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_note_comes_back_from_either_api() {
        let ollama = json!({"response": "  \"shorter note\"  "});
        let openai =
            json!({"choices": [{"message": {"role": "assistant", "content": " shorter note "}}]});
        assert_eq!(parse_rewrite(&ollama).unwrap(), "shorter note");
        assert_eq!(parse_rewrite(&openai).unwrap(), "shorter note");
    }

    #[test]
    fn an_api_that_says_nothing_is_an_error_not_an_empty_note() {
        assert!(parse_rewrite(&json!({"choices": []})).is_err());
        assert!(parse_rewrite(&json!({"choices": [{"message": {"content": "   "}}]})).is_err());
        assert!(parse_rewrite(&json!({"error": {"message": "rate limited"}})).is_err());
    }

    #[test]
    fn a_key_that_cannot_be_a_header_is_refused_rather_than_sent() {
        assert!(Cleaner::openai("sk-ok", "m".into(), Duration::from_secs(1)).is_ok());
        assert!(Cleaner::openai("bad\nkey", "m".into(), Duration::from_secs(1)).is_err());
    }
}
