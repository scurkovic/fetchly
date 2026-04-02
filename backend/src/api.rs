use crate::{
    search,
    sync::{self, chain_display_name},
    todoist::{self, TaskMeta},
    AppState,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

// ── Search ─────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: String,
}

#[derive(Serialize)]
pub struct SearchProduct {
    pub name: String,
    pub brand: Option<String>,
    pub store: String,
    pub store_display: String,
    pub price: f64,
    pub unit_price: Option<f64>,
    pub unit: Option<String>,
}

#[derive(Serialize)]
pub struct SearchResponse {
    pub products: Vec<SearchProduct>,
}

pub async fn search(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<SearchQuery>,
) -> Result<Json<SearchResponse>, StatusCode> {
    let results = search::search(
        &state.db,
        state.gemini.as_deref(),
        &params.q,
        &[],
        10,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let products = results
        .into_iter()
        .map(|p| SearchProduct {
            store_display: chain_display_name(&p.store).to_string(),
            store: p.store,
            name: p.name,
            brand: p.brand,
            price: p.shelf_price,
            unit_price: p.unit_price,
            unit: p.unit,
        })
        .collect();

    Ok(Json(SearchResponse { products }))
}

// ── Item response ──────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct ItemResponse {
    pub id: String,
    pub query: String,
    pub priority: String,
    pub blacklisted_brands: Vec<String>,
    pub current_chain: Option<String>,
    pub current_product_name: Option<String>,
    pub current_brand: Option<String>,
    pub current_price: Option<f64>,
    pub current_unit_price: Option<f64>,
}

fn task_to_response(task: &todoist::Task) -> ItemResponse {
    let (human, meta) = todoist::parse_description(&task.description);
    let parsed = parse_human_description(&human);
    ItemResponse {
        id: task.id.clone(),
        query: task.content.clone(),
        priority: meta.priority,
        blacklisted_brands: meta.blacklisted_brands,
        current_chain: parsed.chain,
        current_product_name: parsed.product_name,
        current_brand: parsed.brand,
        current_price: parsed.price,
        current_unit_price: parsed.unit_price,
    }
}

struct ParsedDescription {
    chain: Option<String>,
    product_name: Option<String>,
    brand: Option<String>,
    price: Option<f64>,
    unit_price: Option<f64>,
}

fn parse_human_description(s: &str) -> ParsedDescription {
    let mut out = ParsedDescription {
        chain: None,
        product_name: None,
        brand: None,
        price: None,
        unit_price: None,
    };
    for line in s.lines() {
        if let Some(v) = line.strip_prefix("Proizvod: ") {
            out.product_name = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("Brand: ") {
            let v = v.trim();
            if v != "-" {
                out.brand = Some(v.to_string());
            }
        } else if let Some(v) = line.strip_prefix("Cijena: ") {
            out.price = v.trim().strip_suffix(" EUR").and_then(|n| n.parse().ok());
        } else if let Some(v) = line.strip_prefix("Lanac: ") {
            out.chain = Some(v.trim().to_string());
        } else if let Some(v) = line
            .strip_prefix("Cijena/l: ")
            .or_else(|| line.strip_prefix("Cijena/kg: "))
        {
            // "0.44 EUR" or "0.44 EUR  (1 l)" — both accepted
            if let Some(price_part) = v.split(" EUR").next() {
                out.unit_price = price_part.trim().parse().ok();
            }
        }
    }
    out
}

// ── List ───────────────────────────────────────────────────────────────────────

pub async fn list_items(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<ItemResponse>>, StatusCode> {
    let tasks = state
        .todoist
        .list_tasks(&state.project_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(tasks.iter().map(task_to_response).collect()))
}

// ── Create ─────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateItemBody {
    pub query: String,
    pub priority: Option<String>,
    pub blacklisted_brands: Option<Vec<String>>,
}

pub async fn create_item(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateItemBody>,
) -> Result<Json<ItemResponse>, StatusCode> {
    let priority = body.priority.as_deref().unwrap_or("immediate");
    let brands = body.blacklisted_brands.unwrap_or_default();
    let meta = TaskMeta::new(priority, brands);
    let query = capitalize(&body.query);

    let due_date = if priority == "immediate" {
        Some(Utc::now().format("%Y-%m-%d").to_string())
    } else {
        None
    };

    let description = todoist::build_description("", &meta);
    let task = state
        .todoist
        .create_task(
            &query,
            &description,
            &state.project_id,
            None,
            due_date.as_deref(),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Background sync
    let db = state.db.clone();
    let todoist = Arc::clone(&state.todoist);
    let gemini = state.gemini.clone();
    let project_id = state.project_id.clone();
    let sections_cache = Arc::clone(&state.sections_cache);
    let task_clone = task.clone();
    tokio::spawn(async move {
        if let Err(e) = sync::sync_task(
            &db,
            &todoist,
            &project_id,
            &sections_cache,
            gemini.as_deref(),
            &task_clone,
        )
        .await
        {
            tracing::warn!("sync on create failed: {e}");
        }
    });

    Ok(Json(task_to_response(&task)))
}

// ── Delete ─────────────────────────────────────────────────────────────────────

pub async fn delete_item(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    state
        .todoist
        .delete_task(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Update blacklisted brands ──────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct UpdateBrandsBody {
    pub brands: Vec<String>,
}

pub async fn update_brands(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateBrandsBody>,
) -> Result<Json<ItemResponse>, StatusCode> {
    let task = state
        .todoist
        .get_task(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let (human, mut meta) = todoist::parse_description(&task.description);
    meta.blacklisted_brands = body.brands;
    let new_description = todoist::build_description(&human, &meta);

    state
        .todoist
        .update_task(&id, &task.content, &new_description, None)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let db = state.db.clone();
    let todoist = Arc::clone(&state.todoist);
    let gemini = state.gemini.clone();
    let project_id = state.project_id.clone();
    let sections_cache = Arc::clone(&state.sections_cache);
    let mut task_clone = task.clone();
    task_clone.description = new_description;
    tokio::spawn(async move {
        if let Err(e) = sync::sync_task(
            &db,
            &todoist,
            &project_id,
            &sections_cache,
            gemini.as_deref(),
            &task_clone,
        )
        .await
        {
            tracing::warn!("sync after brand update failed: {e}");
        }
    });

    let updated = state
        .todoist
        .get_task(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(task_to_response(&updated)))
}

// ── Update priority ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct UpdatePriorityBody {
    pub priority: String,
}

pub async fn update_priority(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<UpdatePriorityBody>,
) -> Result<Json<ItemResponse>, StatusCode> {
    if body.priority != "immediate" && body.priority != "soon" {
        return Err(StatusCode::BAD_REQUEST);
    }
    let task = state
        .todoist
        .get_task(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let (human, mut meta) = todoist::parse_description(&task.description);
    meta.priority = body.priority.clone();
    let new_description = todoist::build_description(&human, &meta);

    let due_date = if body.priority == "immediate" {
        Some(Utc::now().format("%Y-%m-%d").to_string())
    } else {
        None
    };

    state
        .todoist
        .update_task(&id, &task.content, &new_description, due_date.as_deref())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let updated = state
        .todoist
        .get_task(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(task_to_response(&updated)))
}

// ── Manual refresh ─────────────────────────────────────────────────────────────

pub async fn refresh_item(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ItemResponse>, StatusCode> {
    let task = state
        .todoist
        .get_task(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    sync::sync_task(
        &state.db,
        &state.todoist,
        &state.project_id,
        &state.sections_cache,
        state.gemini.as_deref(),
        &task,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let updated = state
        .todoist
        .get_task(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(task_to_response(&updated)))
}

#[derive(Serialize)]
pub struct RefreshAllResponse {
    pub synced: usize,
}

pub async fn refresh_all(
    State(state): State<Arc<AppState>>,
) -> Result<Json<RefreshAllResponse>, StatusCode> {
    let synced = sync::sync_all(
        &state.db,
        &state.todoist,
        &state.project_id,
        &state.sections_cache,
        state.gemini.as_deref(),
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(RefreshAllResponse { synced }))
}
