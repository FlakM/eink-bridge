//! Speech to text. Local `whisper-server` first, because it is fast enough and keeps a
//! comment about an unfinished PR on the machine; the hosted path is a fallback for a
//! noisy room, behind the same opt-in as cloud OCR. Which one answered is recorded.
use crate::api::SttProvenance;
use crate::ocr::providers::{bounded_seconds, enabled, openai_key};
use reqwest::{
    Client,
    header::{AUTHORIZATION, HeaderValue},
    multipart::{Form, Part},
};
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub struct Transcript {
    pub text: String,
    /// The worst segment of the clip, 0..1.
    pub confidence: f32,
    pub stt: SttProvenance,
}

pub struct Asr {
    client: Client,
    local: Option<Local>,
    cloud: Option<Cloud>,
    timeout: Duration,
}

struct Local {
    url: String,
    model: String,
}

/// No Debug: never expose the authorization header.
struct Cloud {
    authorization: HeaderValue,
    endpoint: String,
    model: String,
}

impl Asr {
    /// `EINK_ASR_URL` (default `http://127.0.0.1:8090`, empty to disable) names the
    /// whisper server; `EINK_ASR_OPENAI_ENABLED=1` adds the hosted fallback. Neither
    /// means no voice.
    pub fn from_env() -> Option<Self> {
        let local = match std::env::var("EINK_ASR_URL") {
            Ok(url) if url.trim().is_empty() => None,
            Ok(url) => Some(url),
            Err(_) => Some("http://127.0.0.1:8090".to_string()),
        }
        .map(|url| Local {
            url,
            model: std::env::var("EINK_ASR_MODEL").unwrap_or_else(|_| "small.en".into()),
        });
        let cloud = enabled(std::env::var("EINK_ASR_OPENAI_ENABLED").ok().as_deref())
            .then(openai_key)
            .flatten()
            .and_then(|key| {
                let model =
                    std::env::var("EINK_ASR_OPENAI_MODEL").unwrap_or_else(|_| "whisper-1".into());
                Cloud::new(
                    &key,
                    model,
                    "https://api.openai.com/v1/audio/transcriptions".into(),
                )
                .ok()
            });
        if local.is_none() && cloud.is_none() {
            tracing::info!("voice disabled: no speech recogniser configured");
            return None;
        }
        let timeout = Duration::from_secs(bounded_seconds(
            std::env::var("EINK_ASR_TIMEOUT_SECONDS").ok().as_deref(),
            30,
            120,
        ));
        if let Some(local) = &local {
            tracing::info!(url = %local.url, model = %local.model, "voice: local recogniser");
        }
        if cloud.is_some() {
            tracing::info!("voice: hosted recogniser enabled as the fallback");
        }
        Some(Self::build(local, cloud, timeout))
    }

    /// A recogniser at `url` and nothing else: what a test points at its fake.
    pub fn local(url: &str, timeout: Duration) -> Self {
        Self::build(
            Some(Local {
                url: url.to_string(),
                model: "test".into(),
            }),
            None,
            timeout,
        )
    }

    fn build(local: Option<Local>, cloud: Option<Cloud>, timeout: Duration) -> Self {
        Self {
            client: Client::builder()
                .timeout(timeout + Duration::from_secs(1))
                .connect_timeout(Duration::from_secs(3))
                .build()
                .expect("a default HTTP client can be built"),
            local,
            cloud,
            timeout,
        }
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Which recogniser will answer first, for the `ready` frame.
    pub fn provenance(&self) -> SttProvenance {
        match (&self.local, &self.cloud) {
            (Some(local), _) => SttProvenance {
                provider: "whisper".into(),
                model: local.model.clone(),
                language: "en".into(),
            },
            (None, Some(cloud)) => SttProvenance {
                provider: "openai".into(),
                model: cloud.model.clone(),
                language: "en".into(),
            },
            (None, None) => SttProvenance::default(),
        }
    }

    /// The transcript of a WAV clip. No initial prompt: feeding whisper the document's
    /// vocabulary made it drop the first clause of a clip ("So um, reanchor.rs should
    /// refuse this. The index is enough." came back as "The index is enough."), and the
    /// repair pass in `clean` spells identifiers the document's way without it.
    pub async fn transcribe(&self, wav: Vec<u8>) -> Result<Transcript, String> {
        let started = std::time::Instant::now();
        let mut failure = None;
        if let Some(local) = &self.local {
            match self.local_call(local, wav.clone()).await {
                Ok(transcript) => {
                    observe("local", "ok", started);
                    return Ok(transcript);
                }
                Err(e) => {
                    observe("local", "error", started);
                    tracing::warn!(error = %e, "local recogniser failed");
                    failure = Some(e);
                }
            }
        }
        if let Some(cloud) = &self.cloud {
            match self.cloud_call(cloud, wav).await {
                Ok(transcript) => {
                    observe("openai", "ok", started);
                    return Ok(transcript);
                }
                Err(e) => {
                    observe("openai", "error", started);
                    tracing::warn!(error = %e, "hosted recogniser failed");
                    failure = Some(e);
                }
            }
        }
        Err(failure.unwrap_or_else(|| "no speech recogniser configured".into()))
    }

    async fn local_call(&self, local: &Local, wav: Vec<u8>) -> Result<Transcript, String> {
        let form = Form::new()
            .part(
                "file",
                Part::bytes(wav)
                    .file_name("capture.wav")
                    .mime_str("audio/wav")
                    .unwrap(),
            )
            .text("response_format", "verbose_json")
            .text("temperature", "0");
        let response = self
            .client
            .post(format!("{}/inference", local.url.trim_end_matches('/')))
            .timeout(self.timeout)
            .multipart(form)
            .send()
            .await
            .map_err(|e| transport_error("local", &e))?;
        if !response.status().is_success() {
            return Err(format!(
                "local recogniser returned HTTP {}",
                response.status().as_u16()
            ));
        }
        let body: Value = response
            .json()
            .await
            .map_err(|e| transport_error("local", &e))?;
        parse_verbose(&body, "whisper", &local.model)
    }

    async fn cloud_call(&self, cloud: &Cloud, wav: Vec<u8>) -> Result<Transcript, String> {
        let form = Form::new()
            .part(
                "file",
                Part::bytes(wav)
                    .file_name("capture.wav")
                    .mime_str("audio/wav")
                    .unwrap(),
            )
            .text("model", cloud.model.clone())
            .text("response_format", "verbose_json")
            .text("language", "en")
            .text("temperature", "0");
        let response = self
            .client
            .post(&cloud.endpoint)
            .header(AUTHORIZATION, cloud.authorization.clone())
            .timeout(self.timeout)
            .multipart(form)
            .send()
            .await
            .map_err(|e| transport_error("openai", &e))?;
        if !response.status().is_success() {
            // Do not log remote error bodies: they may contain input or credentials.
            return Err(format!(
                "openai recogniser returned HTTP {}",
                response.status().as_u16()
            ));
        }
        let body: Value = response
            .json()
            .await
            .map_err(|e| transport_error("openai", &e))?;
        parse_verbose(&body, "openai", &cloud.model)
    }
}

impl Cloud {
    fn new(key: &str, model: String, endpoint: String) -> Result<Self, String> {
        let mut authorization = HeaderValue::from_str(&format!("Bearer {key}"))
            .map_err(|_| "Invalid OpenAI API key header")?;
        authorization.set_sensitive(true);
        Ok(Self {
            authorization,
            endpoint,
            model,
        })
    }
}

/// The `verbose_json` shape both whisper.cpp and OpenAI produce: `text`, and `segments`
/// with an `avg_logprob` each. The clip's confidence is its worst segment.
fn parse_verbose(body: &Value, provider: &str, model: &str) -> Result<Transcript, String> {
    if let Some(error) = body.get("error").and_then(Value::as_str) {
        return Err(format!("{provider} recogniser failed: {error}"));
    }
    // Segments come joined by newlines; the note is one line of prose.
    let text = body["text"]
        .as_str()
        .ok_or_else(|| format!("{provider} recogniser answered without text"))?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let confidence = body["segments"]
        .as_array()
        .map(|segments| {
            segments
                .iter()
                .filter_map(|s| s["avg_logprob"].as_f64())
                .map(|lp| lp.exp().clamp(0.0, 1.0) as f32)
                .fold(1.0f32, f32::min)
        })
        .unwrap_or(1.0);
    let language = body["language"]
        .as_str()
        .map(|l| {
            if l == "english" {
                "en".to_string()
            } else {
                l.to_string()
            }
        })
        .unwrap_or_else(|| "en".into());
    Ok(Transcript {
        text,
        confidence,
        stt: SttProvenance {
            provider: provider.into(),
            model: model.into(),
            language,
        },
    })
}

fn observe(provider: &str, result: &str, started: std::time::Instant) {
    crate::metrics::ASR_REQUESTS
        .with_label_values(&[provider, result])
        .inc();
    crate::metrics::ASR_DURATION
        .with_label_values(&[provider])
        .observe(started.elapsed().as_secs_f64());
}

fn transport_error(provider: &str, error: &reqwest::Error) -> String {
    let kind = if error.is_timeout() {
        "timed out"
    } else if error.is_decode() {
        "returned invalid JSON"
    } else {
        "transport failed"
    };
    format!("{provider} recogniser {kind}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn confidence_is_the_worst_segment_not_the_average() {
        let body = json!({"text":" drop the cache column.\n the index is enough\n","language":"english",
            "segments":[{"avg_logprob":-0.1},{"avg_logprob":-1.2},{"avg_logprob":-0.3}]});
        let t = parse_verbose(&body, "whisper", "small.en").unwrap();
        assert_eq!(
            t.text, "drop the cache column. the index is enough",
            "segments joined on one line"
        );
        assert!((t.confidence - (-1.2f64).exp() as f32).abs() < 1e-6);
        assert_eq!(t.stt.provider, "whisper");
        assert_eq!(t.stt.language, "en");
    }

    #[test]
    fn a_recogniser_error_is_an_error_not_an_empty_note() {
        assert!(
            parse_verbose(
                &json!({"error":"FFmpeg conversion failed."}),
                "whisper",
                "m"
            )
            .is_err()
        );
        assert!(parse_verbose(&json!({"segments":[]}), "whisper", "m").is_err());
        let silent = parse_verbose(&json!({"text":"","segments":[]}), "whisper", "m").unwrap();
        assert_eq!(silent.text, "");
        assert_eq!(silent.confidence, 1.0);
    }

    #[test]
    fn a_key_that_cannot_be_a_header_is_refused_rather_than_sent() {
        assert!(Cloud::new("sk-ok", "m".into(), "http://x".into()).is_ok());
        assert!(Cloud::new("bad\nkey", "m".into(), "http://x".into()).is_err());
    }
}
