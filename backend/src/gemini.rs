use anyhow::Result;
use reqwest::Client;
use serde::Deserialize;

pub struct GeminiClient {
    client: Client,
    api_key: String,
}

impl GeminiClient {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
        }
    }

    /// Ask Gemini Flash which products from `candidates` semantically match the user `query`.
    /// Returns 0-based indices into `candidates`.  Falls back to all indices on any error.
    pub async fn filter(
        &self,
        query: &str,
        candidates: &[(&str, Option<&str>)],
    ) -> Result<Vec<usize>> {
        if candidates.is_empty() {
            return Ok(vec![]);
        }

        let list: String = candidates
            .iter()
            .enumerate()
            .map(|(i, (name, brand))| match brand {
                Some(b) => format!("{i}: {name} ({b})"),
                None => format!("{i}: {name}"),
            })
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = format!(
            "User is looking for: \"{query}\"\n\nProducts:\n{list}\n\n\
             Which product indices (0-based) are a good semantic match? \
             Reply with ONLY a JSON array of integers, e.g. [0,3,7]. \
             Return [] if none match."
        );

        let body = serde_json::json!({
            "contents": [{ "parts": [{ "text": prompt }] }],
            "generationConfig": { "response_mime_type": "application/json" }
        });

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash-lite:generateContent?key={}",
            self.api_key
        );

        let raw = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await?
            .text()
            .await?;
        let resp: GeminiResponse = serde_json::from_str(&raw).map_err(|e| {
            tracing::warn!("Gemini response parse error: {e}\nBody: {raw}");
            e
        })?;

        let text = resp
            .candidates
            .into_iter()
            .next()
            .and_then(|c| c.content.parts.into_iter().next())
            .map(|p| p.text)
            .unwrap_or_default();

        let max = candidates.len();
        let indices: Vec<usize> = serde_json::from_str::<Vec<usize>>(&text)
            .unwrap_or_default()
            .into_iter()
            .filter(|&i| i < max)
            .collect();
        Ok(indices)
    }
}

#[derive(Deserialize)]
struct GeminiResponse {
    candidates: Vec<Candidate>,
}
#[derive(Deserialize)]
struct Candidate {
    content: Content,
}
#[derive(Deserialize)]
struct Content {
    parts: Vec<Part>,
}
#[derive(Deserialize)]
struct Part {
    text: String,
}
