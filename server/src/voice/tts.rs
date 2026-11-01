//! The other direction: one voice that reads a note back, so the reader can hear what
//! the agent will read without taking their eyes off the page. Piper, run per request;
//! a sentence takes about a second on a CPU and nothing stays resident.
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

/// The most a note may be before it is refused rather than read.
pub const MAX_TEXT_CHARS: usize = 2_000;
const PIPER_TIMEOUT: Duration = Duration::from_secs(60);

pub struct Tts {
    bin: PathBuf,
    voice: PathBuf,
    sample_rate: u32,
}

impl Tts {
    /// `EINK_PIPER_VOICE` names the `.onnx` voice and enables speech; `EINK_PIPER_BIN`
    /// names the binary when `piper` is not on the service's PATH.
    pub fn from_env() -> Option<Self> {
        let voice = PathBuf::from(std::env::var("EINK_PIPER_VOICE").ok()?.trim());
        if !voice.is_file() {
            tracing::warn!(voice = %voice.display(), "speech disabled: the piper voice is not a file");
            return None;
        }
        let bin = PathBuf::from(std::env::var("EINK_PIPER_BIN").unwrap_or_else(|_| "piper".into()));
        let tts = Self::new(bin, voice);
        tracing::info!(voice = %tts.voice.display(), sample_rate = tts.sample_rate, "speech: piper voice loaded");
        Some(tts)
    }

    pub fn new(bin: PathBuf, voice: PathBuf) -> Self {
        let sample_rate = sample_rate_of(&voice);
        Self {
            bin,
            voice,
            sample_rate,
        }
    }

    pub async fn speak(&self, text: &str) -> Result<Vec<u8>, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("nothing to say".into());
        }
        if text.chars().count() > MAX_TEXT_CHARS {
            return Err(format!(
                "the text is longer than {MAX_TEXT_CHARS} characters"
            ));
        }
        let mut child = tokio::process::Command::new(&self.bin)
            .arg("-m")
            .arg(&self.voice)
            .arg("--output-raw")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("cannot start piper: {e}"))?;
        let mut stdin = child.stdin.take().expect("stdin is piped");
        let line = format!("{text}\n");
        let feed = async move {
            stdin.write_all(line.as_bytes()).await?;
            stdin.shutdown().await
        };
        let output = tokio::time::timeout(PIPER_TIMEOUT, async {
            feed.await
                .map_err(|e| format!("cannot write to piper: {e}"))?;
            child
                .wait_with_output()
                .await
                .map_err(|e| format!("piper failed: {e}"))
        })
        .await
        .map_err(|_| "piper took too long".to_string())??;
        if !output.status.success() {
            return Err(format!("piper exited with {}", output.status));
        }
        if output.stdout.len() < 2 {
            return Err("piper produced no audio".into());
        }
        Ok(super::wav(&output.stdout, self.sample_rate))
    }
}

/// Piper writes raw samples at the voice's own rate, which its sidecar JSON states.
fn sample_rate_of(voice: &Path) -> u32 {
    let sidecar = PathBuf::from(format!("{}.json", voice.display()));
    std::fs::read_to_string(sidecar)
        .ok()
        .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
        .and_then(|v| v["audio"]["sample_rate"].as_u64())
        .map(|rate| rate as u32)
        .unwrap_or(22_050)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/voice")
            .join(name)
    }

    #[tokio::test]
    async fn a_note_comes_back_as_a_wav_at_the_voices_rate() {
        let tts = Tts::new(fixture("fake-piper.sh"), fixture("voice.onnx"));
        assert_eq!(
            tts.sample_rate, 16_000,
            "read from the sidecar, not assumed"
        );
        let wav = tts.speak("Drop the cache column.").await.unwrap();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(
            u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]),
            16_000
        );
        assert_eq!(
            wav.len() - 44,
            2 * "Drop the cache column.".len(),
            "one sample per character"
        );
    }

    #[tokio::test]
    async fn empty_or_endless_text_is_refused_before_piper_runs() {
        let tts = Tts::new(PathBuf::from("/nonexistent/piper"), fixture("voice.onnx"));
        assert_eq!(tts.speak("   ").await.unwrap_err(), "nothing to say");
        assert!(
            tts.speak(&"a".repeat(MAX_TEXT_CHARS + 1))
                .await
                .unwrap_err()
                .contains("longer")
        );
        assert!(
            tts.speak("hello")
                .await
                .unwrap_err()
                .starts_with("cannot start piper")
        );
    }

    #[test]
    fn a_voice_without_a_sidecar_uses_pipers_usual_rate() {
        assert_eq!(sample_rate_of(Path::new("/nonexistent.onnx")), 22_050);
    }
}
