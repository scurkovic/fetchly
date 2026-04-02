mod api;
mod db;
mod gemini;
mod scraper;
mod search;
mod sync;
mod todoist;
use axum::{
    routing::{delete, get, patch, post},
    Router,
};
use chrono::Local;
use clap::{Parser, Subcommand};
use db::Db;
use gemini::GeminiClient;
use reqwest::Client;
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use todoist::TodoistClient;
use tokio::sync::Mutex;
use tower_http::{cors::{Any, CorsLayer}, services::ServeDir};
use tracing::{info, warn};

// ── CLI ────────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "fetchly", about = "Shopping list manager")]
struct Cli {
    #[arg(long, env = "TODOIST_API_TOKEN")]
    todoist_token: String,

    #[arg(long, env = "TODOIST_PROJECT_ID", default_value = "6Vfp6cjrVF7gVv6F")]
    project_id: String,

    #[arg(long, env = "GEMINI_API_KEY")]
    gemini_key: Option<String>,

    #[arg(long, env = "DB_PATH", default_value = "/data/fetchly.db")]
    db_path: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the web server
    Serve {
        #[arg(long, env = "PORT", default_value = "3001")]
        port: u16,
        /// Directory containing the pre-built frontend static files
        #[arg(long, env = "STATIC_DIR", default_value = "/app/static")]
        static_dir: String,
    },
    /// Download price data for all stores and re-sync Todoist tasks
    Sync,
}

// ── Shared state ───────────────────────────────────────────────────────────────

pub struct AppState {
    pub db: Db,
    pub todoist: Arc<TodoistClient>,
    pub gemini: Option<Arc<GeminiClient>>,
    pub project_id: String,
    pub sections_cache: Arc<Mutex<HashMap<String, String>>>,
}

// ── Entry point ────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "fetchly=info".into()),
        )
        .init();

    let cli = Cli::parse();

    let db = db::open(&cli.db_path)?;
    let todoist = Arc::new(TodoistClient::new(cli.todoist_token));
    let gemini = cli.gemini_key.map(|k| Arc::new(GeminiClient::new(k)));
    let sections_cache: Arc<Mutex<HashMap<String, String>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Pre-load sections
    {
        let sections = todoist
            .list_sections(&cli.project_id)
            .await
            .unwrap_or_default();
        let mut lock = sections_cache.lock().await;
        for s in sections {
            lock.insert(s.name.clone(), s.id.clone());
        }
        info!("Loaded {} Todoist sections", lock.len());
    }

    match cli.command {
        Command::Serve { port, static_dir } => {
            serve(db, todoist, gemini, cli.project_id, sections_cache, port, static_dir).await
        }
        Command::Sync => {
            run_sync(db, todoist, gemini, cli.project_id, sections_cache).await
        }
    }
}

// ── serve ──────────────────────────────────────────────────────────────────────

async fn serve(
    db: Db,
    todoist: Arc<TodoistClient>,
    gemini: Option<Arc<GeminiClient>>,
    project_id: String,
    sections_cache: Arc<Mutex<HashMap<String, String>>>,
    port: u16,
    static_dir: String,
) -> anyhow::Result<()> {
    let state = Arc::new(AppState {
        db,
        todoist,
        gemini,
        project_id,
        sections_cache,
    });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/api/search", get(api::search))
        .route("/api/items", get(api::list_items))
        .route("/api/items", post(api::create_item))
        .route("/api/items/{id}", delete(api::delete_item))
        .route("/api/items/{id}/brands", patch(api::update_brands))
        .route("/api/items/{id}/priority", patch(api::update_priority))
        .route("/api/items/{id}/refresh", post(api::refresh_item))
        .route("/api/refresh", post(api::refresh_all))
        .layer(cors)
        .with_state(state)
        .fallback_service(ServeDir::new(&static_dir));

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("Listening on {addr} (static: {static_dir})");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

// ── sync ───────────────────────────────────────────────────────────────────────

async fn run_sync(
    db: Db,
    todoist: Arc<TodoistClient>,
    gemini: Option<Arc<GeminiClient>>,
    project_id: String,
    sections_cache: Arc<Mutex<HashMap<String, String>>>,
) -> anyhow::Result<()> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?;
    let today = Local::now().date_naive();

    scrape_and_store(&db, today, "spar", scraper::spar::fetch(&client, today).await).await;
    scrape_and_store(&db, today, "ktc", scraper::ktc::fetch(&client, today).await).await;
    scrape_and_store(&db, today, "eurospin", scraper::eurospin::fetch(&client, today).await).await;
    scrape_and_store(&db, today, "kaufland", scraper::kaufland::fetch(&client, today).await).await;
    scrape_and_store(&db, today, "lidl", scraper::lidl::fetch(&client, today).await).await;
    scrape_and_store(&db, today, "konzum", scraper::konzum::fetch(&client, today).await).await;

    info!("Scraping done. Re-syncing Todoist tasks...");
    let n = sync::sync_all(&db, &todoist, &project_id, &sections_cache, gemini.as_deref()).await?;
    info!("Sync complete: {n} tasks processed");
    Ok(())
}

async fn scrape_and_store(
    db: &Db,
    today: chrono::NaiveDate,
    store_name: &'static str,
    result: anyhow::Result<Vec<scraper::RawProduct>>,
) {
    match result {
        Err(e) => warn!("{store_name}: scrape failed: {e} — keeping yesterday's data"),
        Ok(raw) => {
            info!("{store_name}: {} products fetched", raw.len());
            let products: Vec<db::Product> = raw
                .into_iter()
                .map(|p| db::Product {
                    name: p.name,
                    brand: p.brand,
                    store: store_name.to_string(),
                    barcode: p.barcode,
                    shelf_price: p.shelf_price,
                    unit_price: p.unit_price,
                    unit: p.unit,
                    scraped_date: today.to_string(),
                })
                .collect();
            let db2 = db.clone();
            match tokio::task::spawn_blocking(move || db::replace_store(&db2, store_name, products))
                .await
            {
                Ok(Ok(n)) => info!("{store_name}: {n} rows written to DB"),
                Ok(Err(e)) => warn!("{store_name}: DB write failed: {e}"),
                Err(e) => warn!("{store_name}: spawn_blocking error: {e}"),
            }
        }
    }
}
