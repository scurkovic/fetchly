use crate::{
    db::{self, Db},
    gemini::GeminiClient,
    search,
    todoist::{self, TodoistClient},
};
use anyhow::Result;
use chrono::Utc;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;
use tracing::{info, warn};

pub fn chain_display_name(code: &str) -> &str {
    match code {
        "lidl" => "Lidl",
        "kaufland" => "Kaufland",
        "konzum" => "Konzum",
        "spar" => "Spar",
        "ktc" => "KTC",
        "eurospin" => "Eurospin",
        other => other,
    }
}

pub fn format_human_description(p: &db::Product) -> String {
    let brand = p.brand.as_deref().unwrap_or("-");
    let chain = chain_display_name(&p.store);
    let mut s = format!(
        "Proizvod: {}\nBrand: {}\nCijena: {:.2} EUR\nLanac: {}",
        p.name, brand, p.shelf_price, chain
    );
    if let (Some(up), Some(unit)) = (p.unit_price, p.unit.as_deref()) {
        if unit == "l" || unit == "kg" {
            s.push_str(&format!("\nCijena/{unit}: {up:.2} EUR"));
        }
    }
    s
}

pub async fn sync_task(
    db: &Db,
    todoist: &TodoistClient,
    project_id: &str,
    sections_cache: &Arc<Mutex<HashMap<String, String>>>,
    gemini: Option<&GeminiClient>,
    task: &todoist::Task,
) -> Result<()> {
    let (_, meta) = todoist::parse_description(&task.description);
    let best = search::find_best(db, gemini, &task.content, &meta.blacklisted_brands).await?;

    let Some(best) = best else {
        warn!("No products found for '{}'", task.content);
        return Ok(());
    };

    let section_name = chain_display_name(&best.store).to_string();
    let section_id =
        get_or_create_section(todoist, project_id, &section_name, sections_cache).await?;

    let human = format_human_description(&best);
    let new_description = todoist::build_description(&human, &meta);

    let due_date = if meta.priority == "immediate" {
        Some(Utc::now().format("%Y-%m-%d").to_string())
    } else {
        None
    };

    todoist
        .update_task(&task.id, &task.content, &new_description, due_date.as_deref())
        .await?;

    if task.section_id.as_deref() != Some(&section_id) {
        todoist.move_task_to_section(&task.id, &section_id).await?;
    }

    info!(
        "Synced '{}': {} @ {:.2} EUR ({})",
        task.content,
        best.name,
        best.shelf_price,
        chain_display_name(&best.store),
    );
    Ok(())
}

pub async fn sync_all(
    db: &Db,
    todoist: &TodoistClient,
    project_id: &str,
    sections_cache: &Arc<Mutex<HashMap<String, String>>>,
    gemini: Option<&GeminiClient>,
) -> Result<usize> {
    let tasks = todoist.list_tasks(project_id).await?;
    let count = tasks.len();
    for task in &tasks {
        if let Err(e) = sync_task(db, todoist, project_id, sections_cache, gemini, task).await {
            warn!("Failed to sync '{}': {}", task.content, e);
        }
    }
    Ok(count)
}

pub async fn get_or_create_section(
    todoist: &TodoistClient,
    project_id: &str,
    name: &str,
    cache: &Arc<Mutex<HashMap<String, String>>>,
) -> Result<String> {
    {
        let lock = cache.lock().await;
        if let Some(id) = lock.get(name) {
            return Ok(id.clone());
        }
    }
    let sections = todoist.list_sections(project_id).await?;
    let mut lock = cache.lock().await;
    for s in &sections {
        lock.insert(s.name.clone(), s.id.clone());
    }
    if let Some(id) = lock.get(name) {
        return Ok(id.clone());
    }
    drop(lock);
    let section = todoist.create_section(project_id, name).await?;
    let id = section.id.clone();
    cache.lock().await.insert(name.to_string(), id.clone());
    Ok(id)
}
