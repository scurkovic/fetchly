use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

const BASE: &str = "https://api.todoist.com/api/v1";

// ── Metadata embedded in task description ─────────────────────────────────────
//
// We store fetchly state as a JSON block at the end of the description,
// separated by a sentinel line so it's invisible-ish in the Todoist UI:
//
//   Proizvod: ...
//   ...
//   <!-- fetchly:{"priority":"immediate","blacklisted_brands":["brand"]} -->

const META_PREFIX: &str = "<!-- fetchly:";
const META_SUFFIX: &str = " -->";

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct TaskMeta {
    #[serde(default)]
    pub priority: String,
    #[serde(default)]
    pub blacklisted_brands: Vec<String>,
}

impl TaskMeta {
    pub fn new(priority: &str, blacklisted_brands: Vec<String>) -> Self {
        Self {
            priority: priority.to_string(),
            blacklisted_brands,
        }
    }
}

/// Split a task description into (human-visible part, metadata).
pub fn parse_description(description: &str) -> (String, TaskMeta) {
    if let Some(idx) = description.find(META_PREFIX) {
        let human = description[..idx].trim_end().to_string();
        let rest = &description[idx + META_PREFIX.len()..];
        if let Some(end) = rest.find(META_SUFFIX) {
            let json = &rest[..end];
            if let Ok(meta) = serde_json::from_str(json) {
                return (human, meta);
            }
        }
    }
    (description.to_string(), TaskMeta::default())
}

/// Build a full description string from the human-visible part and metadata.
pub fn build_description(human: &str, meta: &TaskMeta) -> String {
    let json = serde_json::to_string(meta).unwrap_or_default();
    format!("{}\n{}{}{}", human, META_PREFIX, json, META_SUFFIX)
}

// ── Todoist API types ──────────────────────────────────────────────────────────

#[derive(Deserialize, Debug, Clone)]
pub struct Section {
    pub id: String,
    pub name: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Task {
    pub id: String,
    pub content: String,
    pub description: String,
    pub section_id: Option<String>,
}

#[derive(Deserialize)]
struct PagedResponse<T> {
    results: Vec<T>,
}

#[derive(Clone)]
pub struct TodoistClient {
    client: Client,
    token: String,
}

impl TodoistClient {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            token: token.into(),
        }
    }

    pub async fn list_sections(&self, project_id: &str) -> Result<Vec<Section>> {
        let resp: PagedResponse<Section> = self
            .client
            .get(format!("{BASE}/sections"))
            .bearer_auth(&self.token)
            .query(&[("project_id", project_id)])
            .send()
            .await?
            .json()
            .await
            .context("list sections")?;
        Ok(resp.results)
    }

    pub async fn create_section(&self, project_id: &str, name: &str) -> Result<Section> {
        let section: Section = self
            .client
            .post(format!("{BASE}/sections"))
            .bearer_auth(&self.token)
            .json(&serde_json::json!({ "project_id": project_id, "name": name }))
            .send()
            .await?
            .json()
            .await
            .context("create section")?;
        Ok(section)
    }

    pub async fn list_tasks(&self, project_id: &str) -> Result<Vec<Task>> {
        let resp: PagedResponse<Task> = self
            .client
            .get(format!("{BASE}/tasks"))
            .bearer_auth(&self.token)
            .query(&[("project_id", project_id)])
            .send()
            .await?
            .json()
            .await
            .context("list tasks")?;
        Ok(resp
            .results
            .into_iter()
            .filter(|t| t.description.contains(META_PREFIX))
            .collect())
    }

    pub async fn get_task(&self, task_id: &str) -> Result<Option<Task>> {
        let resp = self
            .client
            .get(format!("{BASE}/tasks/{task_id}"))
            .bearer_auth(&self.token)
            .send()
            .await?;

        if resp.status() == 404 || !resp.status().is_success() {
            return Ok(None);
        }
        Ok(Some(resp.json().await.context("get task")?))
    }

    pub async fn create_task(
        &self,
        content: &str,
        description: &str,
        project_id: &str,
        section_id: Option<&str>,
        due_date: Option<&str>,
    ) -> Result<Task> {
        let mut body = serde_json::json!({
            "content": content,
            "description": description,
            "project_id": project_id,
        });
        if let Some(sid) = section_id {
            body["section_id"] = sid.into();
        }
        if let Some(date) = due_date {
            body["due"] = serde_json::json!({ "date": date });
        }

        let task: Task = self
            .client
            .post(format!("{BASE}/tasks"))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await?
            .json()
            .await
            .context("create task")?;
        Ok(task)
    }

    pub async fn update_task(
        &self,
        task_id: &str,
        content: &str,
        description: &str,
        due_date: Option<&str>,
    ) -> Result<()> {
        let mut body = serde_json::json!({
            "content": content,
            "description": description,
        });
        if let Some(date) = due_date {
            body["due"] = serde_json::json!({ "date": date });
        }

        let resp = self
            .client
            .post(format!("{BASE}/tasks/{task_id}"))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("update task {}: {}", status, body));
        }
        Ok(())
    }

    pub async fn move_task_to_section(&self, task_id: &str, section_id: &str) -> Result<()> {
        let resp = self
            .client
            .post(format!("{BASE}/tasks/{task_id}/move"))
            .bearer_auth(&self.token)
            .json(&serde_json::json!({ "section_id": section_id }))
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("move task {}: {}", status, body));
        }
        Ok(())
    }

    pub async fn delete_task(&self, task_id: &str) -> Result<()> {
        self.client
            .delete(format!("{BASE}/tasks/{task_id}"))
            .bearer_auth(&self.token)
            .send()
            .await?;
        Ok(())
    }
}
