//! Voice comments: touch a block, speak, and the words become a note on that block.
//!
//! Capture is native on the tablet, recognition and cleanup are here, and the card is
//! drawn by the page. This module is the middle: the speech recogniser the tablet
//! streams to (`capture`), the cleanup contract (`clean`), the model-driven rewrites
//! behind the guard (`llm`), and the one voice that reads a note back (`tts`).
pub mod asr;
pub mod capture;
pub mod clean;
pub mod llm;
pub mod tts;

use std::sync::Arc;

/// What the tablet sends: 16 kHz mono signed 16-bit little-endian.
pub const SAMPLE_RATE: u32 = 16_000;
pub const BYTES_PER_SECOND: usize = SAMPLE_RATE as usize * 2;

/// Everything voice needs, or nothing: without a recogniser there is no voice.
pub struct Voice {
    pub asr: asr::Asr,
    pub cleaner: Option<llm::Cleaner>,
    pub tts: Option<tts::Tts>,
}

impl Voice {
    pub fn from_env() -> Option<Arc<Self>> {
        let asr = asr::Asr::from_env()?;
        Some(Arc::new(Self {
            asr,
            cleaner: llm::Cleaner::from_env(),
            tts: tts::Tts::from_env(),
        }))
    }

    pub fn new(asr: asr::Asr, cleaner: Option<llm::Cleaner>, tts: Option<tts::Tts>) -> Arc<Self> {
        Arc::new(Self { asr, cleaner, tts })
    }
}

/// PCM wrapped as a WAV: 16-bit, mono, at `sample_rate`. What the recogniser accepts
/// without ffmpeg, what the session keeps, and what the tablet plays.
pub fn wav(pcm: &[u8], sample_rate: u32) -> Vec<u8> {
    let data_len = pcm.len() as u32;
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wav_header_describes_the_pcm_it_wraps() {
        let pcm = vec![1u8, 0, 2, 0, 3, 0];
        let wav = wav(&pcm, 16_000);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), 1, "mono");
        assert_eq!(
            u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]),
            16_000
        );
        assert_eq!(
            u16::from_le_bytes([wav[34], wav[35]]),
            16,
            "bits per sample"
        );
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(
            u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]) as usize,
            pcm.len()
        );
        assert_eq!(&wav[44..], &pcm[..]);
        assert_eq!(wav.len(), 44 + pcm.len());
    }
}
