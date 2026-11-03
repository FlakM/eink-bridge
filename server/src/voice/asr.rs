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
    /// What to ask for when the caller has no opinion. `auto` lets the recogniser decide
    /// per clip, which is what makes one tablet usable by someone who thinks in two
    /// languages; a fixed code pins every clip and skips the detection pass.
    language: Language,
}

/// The language asked of the recogniser.
///
/// Auto is the default because the reader is not asked which language they are about to
/// think in. A fixed code exists for the case where detection keeps flipping on short
/// clips: `EINK_ASR_LANGUAGE=pl` pins every capture, and the capture itself pins the first
/// language it detects so a note cannot change language halfway through.
#[derive(Debug, Clone, PartialEq)]
pub enum Language {
    Auto,
    Fixed(String),
}

impl Language {
    fn from_env() -> Self {
        match std::env::var("EINK_ASR_LANGUAGE") {
            Ok(value) => Self::parse(&value),
            Err(_) => Self::Auto,
        }
    }

    fn from_env_value(value: Option<&str>) -> Self {
        value.map(Self::parse).unwrap_or(Self::Auto)
    }

    pub fn parse(value: &str) -> Self {
        let trimmed = value.trim().to_ascii_lowercase();
        if trimmed.is_empty() || trimmed == "auto" || trimmed == "detect" {
            Self::Auto
        } else {
            // `pl,en` names a preference list; whisper takes one code per request, so the
            // first is the pinned language and detection never runs.
            Self::Fixed(trimmed.split(',').next().unwrap_or("pl").trim().to_string())
        }
    }

    /// The form value to send, or None to let the server's own default stand.
    fn request_value(&self) -> Option<&str> {
        match self {
            Self::Auto => None,
            Self::Fixed(code) => Some(code.as_str()),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Auto => "auto".into(),
            Self::Fixed(code) => code.clone(),
        }
    }
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
            // Multilingual by default: the same server answers Polish and English, and an
            // English-only model cannot be asked for anything else.
            model: std::env::var("EINK_ASR_MODEL").unwrap_or_else(|_| "small".into()),
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
        tracing::info!(language = %Language::from_env().label(), "voice: language");
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
            language: Language::from_env(),
        }
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Which recogniser will answer first, for the `ready` frame.
    pub fn provenance(&self) -> SttProvenance {
        let language = self.language.label();
        match (&self.local, &self.cloud) {
            (Some(local), _) => SttProvenance {
                provider: "whisper".into(),
                model: local.model.clone(),
                language,
            },
            (None, Some(cloud)) => SttProvenance {
                provider: "openai".into(),
                model: cloud.model.clone(),
                language,
            },
            (None, None) => SttProvenance::default(),
        }
    }

    /// The language to ask for, given what this capture has already answered.
    pub fn pinned(&self) -> Option<&str> {
        self.language.request_value()
    }

    /// The transcript of a WAV clip. No initial prompt: feeding whisper the document's
    /// vocabulary made it drop the first clause of a clip ("So um, reanchor.rs should
    /// refuse this. The index is enough." came back as "The index is enough."), and the
    /// repair pass in `clean` spells identifiers the document's way without it.
    /// `language` overrides the configured one: a capture pins the first language it
    /// detects so the rest of the note stays in it, instead of a short clip flipping the
    /// whole transcript between languages.
    pub async fn transcribe(
        &self,
        wav: Vec<u8>,
        language: Option<&str>,
    ) -> Result<Transcript, String> {
        let language = language.or_else(|| self.language.request_value());
        let started = std::time::Instant::now();
        let mut failure = None;
        if let Some(local) = &self.local {
            match self.local_call(local, wav.clone(), language).await {
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
            match self.cloud_call(cloud, wav, language).await {
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

    async fn local_call(
        &self,
        local: &Local,
        wav: Vec<u8>,
        language: Option<&str>,
    ) -> Result<Transcript, String> {
        let mut form = Form::new()
            .part(
                "file",
                Part::bytes(wav)
                    .file_name("capture.wav")
                    .mime_str("audio/wav")
                    .unwrap(),
            )
            .text("response_format", "verbose_json")
            .text("temperature", "0");
        if let Some(code) = language {
            form = form.text("language", code.to_string());
        }
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
        parse_verbose(&body, "whisper", &local.model, language)
    }

    async fn cloud_call(
        &self,
        cloud: &Cloud,
        wav: Vec<u8>,
        language: Option<&str>,
    ) -> Result<Transcript, String> {
        let mut form = Form::new()
            .part(
                "file",
                Part::bytes(wav)
                    .file_name("capture.wav")
                    .mime_str("audio/wav")
                    .unwrap(),
            )
            .text("model", cloud.model.clone())
            .text("response_format", "verbose_json")
            .text("temperature", "0");
        if let Some(code) = language {
            form = form.text("language", code.to_string());
        }
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
        parse_verbose(&body, "openai", &cloud.model, language)
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
fn parse_verbose(
    body: &Value,
    provider: &str,
    model: &str,
    requested: Option<&str>,
) -> Result<Transcript, String> {
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
    // What it says it heard, else what was asked for, else unknown. Reporting a
    // confident "en" for a Polish clip would be the wrong kind of tidy.
    let language = body["language"]
        .as_str()
        .map(normalise_language)
        .or_else(|| requested.map(normalise_language))
        .unwrap_or_else(|| "unknown".into());
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

/// whisper.cpp answers with a full name ("english", "polish") or a code; OpenAI with a
/// code. The note records the short one either way.
fn normalise_language(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "english" | "en" => "en".into(),
        "polish" | "pl" => "pl".into(),
        "german" | "de" => "de".into(),
        "spanish" | "es" => "es".into(),
        "french" | "fr" => "fr".into(),
        "italian" | "it" => "it".into(),
        "dutch" | "nl" => "nl".into(),
        "ukrainian" | "uk" => "uk".into(),
        other => other.to_string(),
    }
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
    fn a_language_is_either_detected_per_clip_or_pinned() {
        // The default has to work for someone who thinks in two languages: nothing is
        // asked of the reader, and the recogniser decides per clip.
        assert_eq!(Language::from_env_value(None), Language::Auto);
        assert_eq!(Language::parse(""), Language::Auto);
        assert_eq!(Language::parse("auto"), Language::Auto);
        assert_eq!(Language::parse("PL"), Language::Fixed("pl".into()));
        // A preference list keeps the first: whisper takes one code per request.
        assert_eq!(Language::parse("pl,en"), Language::Fixed("pl".into()));
        assert_eq!(Language::Auto.request_value(), None, "auto asks for nothing");
        assert_eq!(Language::Fixed("en".into()).request_value(), Some("en"));
    }

    #[test]
    fn the_language_reported_is_the_one_that_was_used() {
        let english = json!({"text":"hi","segments":[],"language":"english"});
        assert_eq!(parse_verbose(&english, "whisper", "m", None).unwrap().stt.language, "en");
        let polish = json!({"text":"cześć","segments":[],"language":"polish"});
        assert_eq!(parse_verbose(&polish, "whisper", "m", None).unwrap().stt.language, "pl");
        // Nothing claimed and nothing asked: say so rather than assume English.
        let silent = json!({"text":"x","segments":[]});
        assert_eq!(parse_verbose(&silent, "whisper", "m", None).unwrap().stt.language, "unknown");
        assert_eq!(parse_verbose(&silent, "whisper", "m", Some("pl")).unwrap().stt.language, "pl");
    }

    #[test]
    fn confidence_is_the_worst_segment_not_the_average() {
        let body = json!({"text":" drop the cache column.\n the index is enough\n","language":"english",
            "segments":[{"avg_logprob":-0.1},{"avg_logprob":-1.2},{"avg_logprob":-0.3}]});
        let t = parse_verbose(&body, "whisper", "small.en", None).unwrap();
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
                "m",
                None
            )
            .is_err()
        );
        assert!(parse_verbose(&json!({"segments":[]}), "whisper", "m", None).is_err());
        let silent = parse_verbose(&json!({"text":"","segments":[]}), "whisper", "m", None).unwrap();
        assert_eq!(silent.text, "");
        assert_eq!(silent.confidence, 1.0);
        assert_eq!(silent.stt.language, "unknown");
    }

    #[test]
    fn a_key_that_cannot_be_a_header_is_refused_rather_than_sent() {
        assert!(Cloud::new("sk-ok", "m".into(), "http://x".into()).is_ok());
        assert!(Cloud::new("bad\nkey", "m".into(), "http://x".into()).is_err());
    }
}
