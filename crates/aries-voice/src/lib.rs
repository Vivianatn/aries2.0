//! Voix : l'audio du micro est transcrit par `whisper-server` (whisper.cpp), et les
//! réponses sont lues par Piper. Les deux tournent dans leurs propres conteneurs, sur le
//! réseau Docker interne : aucun son ne sort du serveur.

use std::time::Duration;

use bytes::Bytes;
use reqwest::multipart::{Form, Part};
use serde::Deserialize;
use serde_json::json;

/// Section `[voice]` de `config/aries.toml`. Absente : la voix est désactivée.
#[derive(Debug, Clone, Deserialize)]
pub struct VoiceConfig {
    /// `whisper-server`, lancé avec `--convert` (ffmpeg) pour accepter WebM, Ogg ou MP4.
    pub stt_url: String,
    /// Serveur HTTP de Piper (`python -m piper.http_server`).
    pub tts_url: String,
    /// Langue imposée à Whisper (plus rapide et plus fiable que la détection).
    #[serde(default = "default_language")]
    pub language: String,
    /// Débit de parole de Piper : moins de 1 parle plus vite.
    pub length_scale: Option<f64>,
    /// Texte max d'une synthèse (une phrase ou un paragraphe).
    #[serde(default = "default_max_text_chars")]
    pub max_text_chars: usize,
}

fn default_language() -> String {
    "fr".into()
}

fn default_max_text_chars() -> usize {
    1000
}

#[derive(Debug, thiserror::Error)]
pub enum VoiceError {
    #[error("service vocal injoignable : {0}")]
    Http(#[from] reqwest::Error),
    #[error("texte trop long pour la synthèse ({0} caractères)")]
    TextTooLong(usize),
}

pub struct Voice {
    client: reqwest::Client,
    stt_url: String,
    tts_url: String,
    language: String,
    length_scale: Option<f64>,
    max_text_chars: usize,
}

#[derive(Deserialize)]
struct Transcription {
    text: String,
}

impl Voice {
    pub fn new(config: &VoiceConfig) -> Result<Self, VoiceError> {
        Ok(Self {
            // Une minute d'audio sur 2 vCPU partagés peut prendre plusieurs dizaines de secondes.
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(120))
                .build()?,
            stt_url: format!("{}/inference", config.stt_url.trim_end_matches('/')),
            tts_url: format!("{}/synthesize", config.tts_url.trim_end_matches('/')),
            language: config.language.clone(),
            length_scale: config.length_scale,
            max_text_chars: config.max_text_chars,
        })
    }

    /// Transcrit un enregistrement (tout format lisible par ffmpeg).
    pub async fn transcribe(&self, audio: Bytes, mime: &str) -> Result<String, VoiceError> {
        let len = audio.len() as u64;
        let file = Part::stream_with_length(audio, len)
            .file_name("audio")
            .mime_str(mime)?;
        let form = Form::new()
            .part("file", file)
            .text("response_format", "json")
            .text("language", self.language.clone());
        let transcription: Transcription = self
            .client
            .post(&self.stt_url)
            .multipart(form)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(transcription.text.trim().to_owned())
    }

    /// Lit un texte à voix haute : renvoie un fichier WAV.
    pub async fn synthesize(&self, text: &str) -> Result<Bytes, VoiceError> {
        let len = text.chars().count();
        if len > self.max_text_chars {
            return Err(VoiceError::TextTooLong(len));
        }
        let mut body = json!({ "text": text });
        if let Some(scale) = self.length_scale {
            body["length_scale"] = json!(scale);
        }
        Ok(self
            .client
            .post(&self.tts_url)
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?)
    }
}

#[cfg(test)]
mod tests;
