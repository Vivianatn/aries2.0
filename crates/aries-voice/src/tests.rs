use axum::{Json, Router, http::HeaderMap, routing::post};
use serde_json::Value;

use super::*;

/// Faux whisper-server + Piper sur un port libre ; renvoie l'URL de base.
async fn fake_services() -> String {
    let app = Router::new()
        .route(
            "/inference",
            post(|headers: HeaderMap, body: Bytes| async move {
                let multipart = headers[reqwest::header::CONTENT_TYPE]
                    .to_str()
                    .is_ok_and(|v| v.starts_with("multipart/form-data"));
                let body = String::from_utf8_lossy(&body);
                assert!(multipart && body.contains("name=\"language\"\r\n\r\nfr"));
                Json(json!({ "text": " Bonjour ArIès. \n" }))
            }),
        )
        .route(
            "/synthesize",
            post(|Json(body): Json<Value>| async move {
                format!("WAV:{}:{}", body["text"], body["length_scale"])
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/")
}

fn voice(url: &str, max_text_chars: usize) -> Voice {
    Voice::new(&VoiceConfig {
        stt_url: url.into(),
        tts_url: url.into(),
        language: default_language(),
        length_scale: Some(0.9),
        max_text_chars,
    })
    .unwrap()
}

#[tokio::test]
async fn transcribe_trims_whisper_text() {
    let url = fake_services().await;
    let text = voice(&url, 100)
        .transcribe(Bytes::from_static(b"audio"), "audio/webm")
        .await
        .unwrap();
    assert_eq!(text, "Bonjour ArIès.");
}

#[tokio::test]
async fn synthesize_sends_text_and_rate() {
    let url = fake_services().await;
    let wav = voice(&url, 100).synthesize("Salut !").await.unwrap();
    assert_eq!(&wav[..], "WAV:\"Salut !\":0.9".as_bytes());
}

#[tokio::test]
async fn synthesize_rejects_long_text() {
    let err = voice("http://127.0.0.1:9", 3).synthesize("Salut").await;
    assert!(matches!(err, Err(VoiceError::TextTooLong(5))));
}
