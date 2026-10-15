//! Lecture d'une page web : téléchargement, extraction du texte principal (sans liens).
//!
//! Seules les adresses publiques sont accessibles : le LLM (ou une page piégée) ne doit
//! pas pouvoir atteindre les services internes (llama.cpp, Qdrant, réseau Docker…).

use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use aries_governance::RiskLevel;
use aries_llm::ToolSpec;
use async_trait::async_trait;
use dom_smoothie::{Config, Readability, TextMode};
use reqwest::{Url, header};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Tool, ToolError, parse_arguments, truncate_chars};

const MAX_REDIRECTS: usize = 5;
const MAX_BODY_BYTES: usize = 3 * 1024 * 1024;
const USER_AGENT: &str = "Mozilla/5.0 (compatible; ArIes/2.0; assistant personnel)";

pub struct ReadWebPage {
    max_chars: usize,
}

impl ReadWebPage {
    pub fn new(max_chars: usize) -> Self {
        Self { max_chars }
    }
}

#[derive(Deserialize)]
struct Arguments {
    url: String,
}

#[async_trait]
impl Tool for ReadWebPage {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read_web_page".into(),
            description: "Lit une page web et renvoie son texte principal. \
                          À utiliser après web_search pour vérifier ou approfondir un résultat."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Adresse http(s) complète" }
                },
                "required": ["url"]
            }),
        }
    }

    fn risk(&self) -> RiskLevel {
        RiskLevel::Read
    }

    async fn call(&self, arguments: Value) -> Result<String, ToolError> {
        let Arguments { url } = parse_arguments(arguments)?;
        let url = Url::parse(&url).map_err(|err| ToolError::InvalidArguments(err.to_string()))?;
        let (final_url, response) = fetch_public(url).await?;

        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("text/html")
            .to_ascii_lowercase();
        let body = read_limited(response).await?;

        let text = if content_type.contains("html") {
            let url = final_url.to_string();
            tokio::task::spawn_blocking(move || extract_article(body, &url))
                .await
                .map_err(|err| ToolError::Failed(err.to_string()))??
        } else if content_type.starts_with("text/") || content_type.contains("json") {
            body
        } else {
            return Err(ToolError::Refused(format!(
                "type de contenu non pris en charge : {content_type}"
            )));
        };

        let shown = truncate_chars(&text, self.max_chars);
        let mut out = format!("Source : {final_url}\n\n{shown}");
        if shown.len() < text.len() {
            out.push_str("\n\n[… page tronquée]");
        }
        Ok(out)
    }
}

/// GET en suivant les redirections à la main : chaque saut est vérifié, et la
/// connexion est forcée sur les adresses IP validées (pas de « DNS rebinding »).
async fn fetch_public(mut url: Url) -> Result<(Url, reqwest::Response), ToolError> {
    for _ in 0..=MAX_REDIRECTS {
        let addrs = public_addrs(&url).await?;
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .user_agent(USER_AGENT);
        if let Some(domain) = url.domain() {
            builder = builder.resolve_to_addrs(domain, &addrs);
        }
        let response = builder
            .build()?
            .get(url.clone())
            .header(header::ACCEPT_LANGUAGE, "fr,en;q=0.8")
            .send()
            .await?;

        let status = response.status();
        if status.is_redirection() {
            let location = response
                .headers()
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or(ToolError::Status(status))?;
            url = url
                .join(location)
                .map_err(|err| ToolError::Failed(format!("redirection invalide : {err}")))?;
            continue;
        }
        if !status.is_success() {
            return Err(ToolError::Status(status));
        }
        return Ok((url, response));
    }
    Err(ToolError::Failed("trop de redirections".into()))
}

/// Adresses de l'hôte, toutes publiques, sinon refus.
async fn public_addrs(url: &Url) -> Result<Vec<SocketAddr>, ToolError> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(ToolError::Refused(
            "seules les adresses http(s) sont permises".into(),
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| ToolError::InvalidArguments("adresse sans hôte".into()))?
        .trim_start_matches('[')
        .trim_end_matches(']');
    let port = url.port_or_known_default().unwrap_or(80);
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|err| ToolError::Failed(format!("hôte introuvable : {err}")))?
        .collect();
    if addrs.is_empty() || !addrs.iter().all(|addr| is_public(addr.ip())) {
        return Err(ToolError::Refused(
            "adresse privée ou locale refusée".into(),
        ));
    }
    Ok(addrs)
}

fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                || a == 0
                || (a == 100 && (64..128).contains(&b)))
        }
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_public(v4.into()),
            None => {
                !(v6.is_loopback()
                    || v6.is_unspecified()
                    || v6.is_unique_local()
                    || v6.is_unicast_link_local())
            }
        },
    }
}

async fn read_limited(mut response: reqwest::Response) -> Result<String, ToolError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(ToolError::Refused("page trop volumineuse".into()));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// Texte principal (algorithme Readability de Firefox), mise en forme conservée, liens retirés
/// pour économiser le contexte.
fn extract_article(html: String, url: &str) -> Result<String, ToolError> {
    let config = Config {
        text_mode: TextMode::Formatted,
        ..Config::default()
    };
    let article = Readability::new(html, Some(url), Some(config))
        .and_then(|mut readability| readability.parse())
        .map_err(|err| ToolError::Failed(format!("contenu illisible : {err}")))?;
    let title = article.title.trim();
    let text = article.text_content.trim();
    Ok(if title.is_empty() {
        text.to_owned()
    } else {
        format!("# {title}\n\n{text}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_internal_addresses() {
        for ip in [
            "127.0.0.1",
            "10.0.0.5",
            "172.18.0.3",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:172.18.0.3",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["1.1.1.1", "51.15.0.1", "2606:4700::1111"] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
    }

    #[tokio::test]
    async fn refuses_local_and_non_http_urls() {
        for url in [
            "http://localhost:6333/collections",
            "file:///etc/passwd",
            "http://127.0.0.1/",
        ] {
            let url = Url::parse(url).unwrap();
            assert!(
                matches!(public_addrs(&url).await, Err(ToolError::Refused(_))),
                "{url}"
            );
        }
    }

    #[test]
    fn extracts_main_content() {
        let paragraph = "ArIès est un assistant personnel écrit en Rust. ".repeat(20);
        let html = format!(
            "<html><head><title>Titre</title></head><body><nav>Menu</nav>\
             <article><h1>Titre</h1><p>{paragraph}</p><p>{paragraph}</p></article></body></html>"
        );
        let text = extract_article(html, "https://exemple.fr/page").unwrap();
        assert!(text.starts_with("# Titre"));
        assert!(text.contains("assistant personnel"));
    }

    /// Accès Internet réel : `cargo test -p aries-tools -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn reads_a_real_page() {
        let text = ReadWebPage::new(2_000)
            .call(json!({ "url": "https://fr.wikipedia.org/wiki/Rust_(langage)" }))
            .await
            .unwrap();
        println!("{text}");
        assert!(text.contains("Rust"));
    }
}
