use crate::metrics;
use reqwest::{
    Client,
    header::{AUTHORIZATION, HeaderValue},
};
use serde_json::{Value, json};
use std::time::Duration;

const PROMPT: &str = "Transcribe the handwritten text in this image exactly as written. Output only the transcribed text, nothing else. Ignore circles, arrows, and underlines. If there is no legible text, return an empty string. Do not invent text.";

pub(super) async fn ollama(
    client: &Client,
    url: &str,
    model: &str,
    image: &str,
    timeout: Duration,
) -> Result<String, String> {
    let response = client
        .post(format!("{}/api/generate", url.trim_end_matches('/')))
        .timeout(timeout)
        .json(&json!({
            "model": model, "prompt": PROMPT, "images": [image],
            "stream": false, "options": { "num_predict": 256 },
        }))
        .send()
        .await
        .map_err(|e| transport_error("local", &e))?;
    if !response.status().is_success() {
        return Err(format!(
            "local OCR returned HTTP {}",
            response.status().as_u16()
        ));
    }
    let body: Value = response
        .json()
        .await
        .map_err(|e| transport_error("local", &e))?;
    if body["done_reason"] == "length" || body.get("error").is_some() {
        return Err("local OCR failed or reached its output token limit".into());
    }
    record_tokens(
        "local",
        body["prompt_eval_count"].as_u64().unwrap_or(0),
        body["eval_count"].as_u64().unwrap_or(0),
    );
    tracing::debug!(
        prompt_eval_ms = body["prompt_eval_duration"].as_u64().unwrap_or(0) / 1_000_000,
        token_eval_ms = body["eval_duration"].as_u64().unwrap_or(0) / 1_000_000,
        "Local OCR timings"
    );
    body["response"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "local OCR response had no text field".into())
}

/// Dedicated llama-server with OCR-specific image-token limits. This bypasses
/// Ollama's Qwen-VL minimum without changing the shared Ollama service.
pub(super) async fn llama(
    client: &Client,
    url: &str,
    image: &str,
    timeout: Duration,
) -> Result<String, String> {
    let response = client
        .post(format!("{}/v1/chat/completions", url.trim_end_matches('/')))
        .timeout(timeout)
        .json(&json!({
            "messages": [{"role":"user", "content":[
                {"type":"text", "text":PROMPT},
                {"type":"image_url", "image_url":{"url":format!("data:image/png;base64,{image}")}}
            ]}],
            "stream":false, "max_tokens":256, "temperature":0, "cache_prompt":false
        }))
        .send()
        .await
        .map_err(|e| transport_error("local", &e))?;
    if !response.status().is_success() {
        return Err(format!(
            "local llama OCR returned HTTP {}",
            response.status().as_u16()
        ));
    }
    let body: Value = response
        .json()
        .await
        .map_err(|e| transport_error("local", &e))?;
    let choice = &body["choices"][0];
    if choice["finish_reason"] != "stop"
        || choice["message"]
            .get("refusal")
            .is_some_and(|v| !v.is_null())
    {
        return Err("local llama OCR did not finish a transcription".into());
    }
    record_tokens(
        "local",
        body["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
        body["usage"]["completion_tokens"].as_u64().unwrap_or(0),
    );
    choice["message"]["content"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "local llama OCR response had no text".into())
}

/// No Debug implementation: never expose the server-side authorization header.
pub(super) struct OpenAi {
    client: Client,
    authorization: HeaderValue,
    endpoint: String,
    model: String,
}

impl OpenAi {
    pub(super) fn from_env() -> Option<Self> {
        if !enabled(std::env::var("EINK_OCR_OPENAI_ENABLED").ok().as_deref()) {
            return None;
        }
        let key = if let Ok(path) = std::env::var("EINK_OPENAI_API_KEY_FILE") {
            match std::fs::read_to_string(path) {
                Ok(key) => Some(key),
                Err(_) => {
                    tracing::warn!("OpenAI OCR disabled: cannot read API key file");
                    return None;
                }
            }
        } else {
            std::env::var("EINK_OPENAI_API_KEY")
                .or_else(|_| std::env::var("OPENAI_API_KEY"))
                .ok()
        };
        let Some(key) = key.filter(|key| !key.trim().is_empty()) else {
            tracing::warn!(
                "OpenAI OCR was requested but no API key is configured; using local OCR only"
            );
            return None;
        };
        let model = std::env::var("EINK_OPENAI_MODEL").unwrap_or_else(|_| "gpt-4.1-mini".into());
        let seconds = timeout_seconds(std::env::var("EINK_OPENAI_TIMEOUT_SECONDS").ok().as_deref());
        match Self::new(
            key.trim(),
            model,
            Duration::from_secs(seconds),
            "https://api.openai.com/v1/responses".into(),
        ) {
            Ok(provider) => {
                tracing::info!(model = %provider.model, timeout_seconds = seconds, "OpenAI OCR race enabled: handwriting crops will be sent to OpenAI");
                Some(provider)
            }
            Err(_) => {
                tracing::warn!("OpenAI OCR disabled: invalid client or API key configuration");
                None
            }
        }
    }

    pub(super) fn new(
        key: &str,
        model: String,
        timeout: Duration,
        endpoint: String,
    ) -> Result<Self, String> {
        let mut authorization = HeaderValue::from_str(&format!("Bearer {key}"))
            .map_err(|_| "Invalid OpenAI API key header")?;
        authorization.set_sensitive(true);
        let client = Client::builder()
            .timeout(timeout)
            .connect_timeout(Duration::from_secs(3))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "Cannot build OpenAI client")?;
        Ok(Self {
            client,
            authorization,
            endpoint,
            model,
        })
    }

    pub(super) async fn recognize(&self, image: &str) -> Result<String, String> {
        let response = self.client.post(&self.endpoint).header(AUTHORIZATION, self.authorization.clone())
            .json(&json!({
                "model": self.model,
                "store": false,
                "max_output_tokens": 256,
                "instructions": PROMPT,
                "input": [{"role": "user", "content": [
                    {"type": "input_text", "text": "Read this handwriting crop."},
                    {"type": "input_image", "image_url": format!("data:image/png;base64,{image}"), "detail": "high"}
                ]}]
            })).send().await.map_err(|e| transport_error("openai", &e))?;
        if !response.status().is_success() {
            // Do not log remote error bodies: they may contain input or credentials.
            return Err(format!(
                "openai OCR returned HTTP {}",
                response.status().as_u16()
            ));
        }
        let body: Value = response
            .json()
            .await
            .map_err(|e| transport_error("openai", &e))?;
        record_tokens(
            "openai",
            body["usage"]["input_tokens"].as_u64().unwrap_or(0),
            body["usage"]["output_tokens"].as_u64().unwrap_or(0),
        );
        parse_openai(&body)
    }
}

fn enabled(value: Option<&str>) -> bool {
    matches!(value, Some("1" | "true"))
}

/// A timeout from the environment, bounded to something usable. A zero would time out
/// every request before it was sent, which reads as "the provider never answers".
pub(super) fn bounded_seconds(value: Option<&str>, default: u64, max: u64) -> u64 {
    value
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| (1..=max).contains(v))
        .unwrap_or(default)
}

/// The cloud timeout, bounded to something usable.
fn timeout_seconds(value: Option<&str>) -> u64 {
    bounded_seconds(value, 15, 120)
}

fn parse_openai(body: &Value) -> Result<String, String> {
    if body["status"] != "completed" || body.get("error").is_some_and(|v| !v.is_null()) {
        return Err("openai OCR response was not completed".into());
    }
    let mut texts = Vec::new();
    if let Some(output) = body["output"].as_array() {
        for item in output {
            if item["type"] != "message" {
                continue;
            }
            if let Some(content) = item["content"].as_array() {
                for part in content {
                    if part["type"] == "refusal" {
                        return Err("openai declined OCR".into());
                    }
                    if part["type"] == "output_text"
                        && let Some(text) = part["text"].as_str()
                    {
                        texts.push(text);
                    }
                }
            }
        }
    }
    let text = texts.join("\n");
    if text.trim().is_empty() {
        return Err("openai OCR returned no recognized text".into());
    }
    Ok(text)
}

fn record_tokens(provider: &str, input: u64, output: u64) {
    metrics::OCR_PROVIDER_TOKENS
        .with_label_values(&[provider, "input"])
        .inc_by(input);
    metrics::OCR_PROVIDER_TOKENS
        .with_label_values(&[provider, "output"])
        .inc_by(output);
    if provider == "local" {
        metrics::OCR_TOKENS
            .with_label_values(&["prompt"])
            .inc_by(input);
        metrics::OCR_TOKENS
            .with_label_values(&["eval"])
            .inc_by(output);
    }
}

fn transport_error(provider: &str, error: &reqwest::Error) -> String {
    let kind = if error.is_timeout() {
        "timed out"
    } else if error.is_decode() {
        "returned invalid JSON"
    } else {
        "transport failed"
    };
    format!("{provider} OCR {kind}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_requires_explicit_opt_in() {
        for value in [None, Some("false"), Some("0"), Some("typo")] {
            assert!(!enabled(value));
        }
        assert!(enabled(Some("true")));
        assert!(enabled(Some("1")));
    }

    #[test]
    fn the_cloud_timeout_stays_inside_a_usable_range() {
        assert_eq!(timeout_seconds(None), 15, "the default");
        assert_eq!(timeout_seconds(Some("5")), 5);
        assert_eq!(
            timeout_seconds(Some(" 7 ")),
            7,
            "surrounding space is not a value"
        );
        assert_eq!(timeout_seconds(Some("1")), 1);
        assert_eq!(timeout_seconds(Some("120")), 120);
        for unusable in ["0", "121", "99999", "abc", "", "-5", "5.5"] {
            assert_eq!(
                timeout_seconds(Some(unusable)),
                15,
                "{unusable:?} would break every request rather than fall back"
            );
        }
    }

    #[test]
    fn a_bound_is_never_zero_and_never_unbounded() {
        // Used for the local OCR calls: a zero abandons every attempt before it is sent,
        // and no ceiling leaves a reader waiting on a model that has stopped.
        assert_eq!(bounded_seconds(None, 120, 300), 120);
        assert_eq!(bounded_seconds(Some("45"), 120, 300), 45);
        for unusable in ["0", "301", "abc", "", "  ", "-1", "1e3"] {
            assert_eq!(
                bounded_seconds(Some(unusable), 120, 300),
                120,
                "{unusable:?} should fall back rather than disable OCR"
            );
        }
        assert_eq!(bounded_seconds(Some("300"), 120, 300), 300);
        assert_eq!(bounded_seconds(Some("301"), 20, 120), 20);
    }

    #[test]
    fn a_key_that_cannot_be_a_header_is_refused_rather_than_sent() {
        // A key pasted with a newline or a stray character must disable the provider
        // instead of producing a malformed Authorization header.
        assert!(
            OpenAi::new(
                "sk-ok",
                "m".into(),
                Duration::from_secs(1),
                "http://x".into()
            )
            .is_ok()
        );
        for bad in ["bad\nkey", "bad\r\nkey", "bad\u{7}key"] {
            assert!(
                OpenAi::new(bad, "m".into(), Duration::from_secs(1), "http://x".into()).is_err(),
                "{bad:?} should not become a header"
            );
        }
    }

    #[test]
    fn extracts_only_completed_output_text_and_rejects_refusals() {
        let valid = json!({"status":"completed", "output":[
            {"type":"reasoning", "summary":[]},
            {"type":"message", "content":[{"type":"output_text", "text":"REMOVE IT"}]}
        ]});
        assert_eq!(parse_openai(&valid).unwrap(), "REMOVE IT");
        for body in [
            json!({"status":"incomplete", "output": valid["output"]}),
            json!({"status":"failed"}),
            json!({"status":"completed", "output":[]}),
            json!({"status":"completed", "output":[{"type":"message", "content":[{"type":"refusal","refusal":"no"}]}]}),
        ] {
            assert!(parse_openai(&body).is_err());
        }
    }
}
