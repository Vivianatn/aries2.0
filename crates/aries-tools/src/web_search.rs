//! Recherche web via SearXNG (méta-moteur auto-hébergé, API JSON, sans clé).

use std::{fmt::Write, time::Duration};

use aries_governance::RiskLevel;
use aries_llm::ToolSpec;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Tool, ToolError, parse_arguments, truncate_chars};

const SNIPPET_MAX_CHARS: usize = 300;

pub struct WebSearch {
    client: reqwest::Client,
    url: String,
    max_results: usize,
}

impl WebSearch {
    pub fn new(searxng_url: &str, max_results: usize) -> Result<Self, ToolError> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .build()?,
            url: format!("{}/search", searxng_url.trim_end_matches('/')),
            max_results,
        })
    }
}

#[derive(Deserialize)]
struct Arguments {
    query: String,
}

#[derive(Deserialize)]
struct SearchResponse {
    #[serde(default)]
    results: Vec<SearchResult>,
}

#[derive(Deserialize)]
struct SearchResult {
    title: String,
    url: String,
    #[serde(default)]
    content: String,
}

#[async_trait]
impl Tool for WebSearch {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "web_search".into(),
            description: "Recherche sur Internet. À utiliser pour toute information récente, \
                          factuelle ou que tu ne connais pas. Renvoie titres, adresses et extraits."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Requête, en mots-clés" }
                },
                "required": ["query"]
            }),
        }
    }

    fn risk(&self) -> RiskLevel {
        RiskLevel::Read
    }

    async fn call(&self, arguments: Value) -> Result<String, ToolError> {
        let Arguments { query } = parse_arguments(arguments)?;
        let response = self
            .client
            .get(&self.url)
            .query(&[("q", query.as_str()), ("format", "json")])
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(ToolError::Status(response.status()));
        }
        let response: SearchResponse = response.json().await?;
        Ok(format_results(&response.results, self.max_results))
    }
}

fn format_results(results: &[SearchResult], max: usize) -> String {
    if results.is_empty() {
        return "Aucun résultat.".into();
    }
    results
        .iter()
        .take(max)
        .enumerate()
        .fold(String::new(), |mut out, (rank, result)| {
            let _ = writeln!(
                out,
                "{}. {}\n   {}\n   {}",
                rank + 1,
                result.title.trim(),
                result.url,
                truncate_chars(result.content.trim(), SNIPPET_MAX_CHARS)
            );
            out
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_ranked_results() {
        let response: SearchResponse = serde_json::from_value(json!({
            "results": [
                { "title": "A", "url": "https://a.fr", "content": "extrait" },
                { "title": "B", "url": "https://b.fr" },
                { "title": "C", "url": "https://c.fr" }
            ]
        }))
        .unwrap();
        let text = format_results(&response.results, 2);
        assert!(text.starts_with("1. A\n   https://a.fr\n   extrait"));
        assert!(text.contains("2. B"));
        assert!(!text.contains("3. C"));
        assert_eq!(format_results(&[], 5), "Aucun résultat.");
    }
}
