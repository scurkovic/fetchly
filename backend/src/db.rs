use anyhow::{anyhow, Result};
use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex};

pub type Db = Arc<Mutex<Connection>>;

#[derive(Clone, Debug)]
pub struct Product {
    pub name: String,
    pub brand: Option<String>,
    pub store: String,
    pub barcode: Option<String>,
    pub shelf_price: f64,
    pub unit_price: Option<f64>,
    pub unit: Option<String>,
    pub scraped_date: String,
}

pub fn open(path: &str) -> Result<Db> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE VIRTUAL TABLE IF NOT EXISTS products USING fts5(
             name          UNINDEXED,
             name_search,
             brand         UNINDEXED,
             brand_search,
             store         UNINDEXED,
             barcode       UNINDEXED,
             shelf_price   UNINDEXED,
             unit_price    UNINDEXED,
             unit          UNINDEXED,
             scraped_date  UNINDEXED
         );",
    )?;
    Ok(Arc::new(Mutex::new(conn)))
}

pub fn replace_store(db: &Db, store: &str, products: Vec<Product>) -> Result<usize> {
    // Deduplicate by (name, brand, shelf_price) — some stores list the same product
    // under multiple barcodes (e.g. national EAN + internal PLU code).
    let mut seen = std::collections::HashSet::new();
    let products: Vec<Product> = products
        .into_iter()
        .filter(|p| {
            let key = (
                p.name.trim().to_lowercase(),
                p.brand.as_deref().unwrap_or("").to_lowercase(),
                (p.shelf_price * 100.0) as u64,
            );
            seen.insert(key)
        })
        .collect();

    let conn = db.lock().map_err(|_| anyhow!("DB lock poisoned"))?;
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM products WHERE store = ?1", params![store])?;
    let count = products.len();
    for p in &products {
        tx.execute(
            "INSERT INTO products(
                 name, name_search, brand, brand_search,
                 store, barcode, shelf_price, unit_price, unit, scraped_date
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                p.name,
                normalize(&p.name),
                p.brand,
                p.brand.as_deref().map(normalize),
                p.store,
                p.barcode,
                p.shelf_price,
                p.unit_price,
                p.unit,
                p.scraped_date,
            ],
        )?;
    }
    tx.commit()?;
    Ok(count)
}

pub fn search_fts(db: &Db, raw_query: &str) -> Result<Vec<Product>> {
    let fts_query = build_fts_query(raw_query);
    if fts_query.is_empty() {
        return Ok(vec![]);
    }
    let conn = db.lock().map_err(|_| anyhow!("DB lock poisoned"))?;
    let mut stmt = conn.prepare(
        "SELECT name, brand, store, barcode, shelf_price, unit_price, unit, scraped_date
           FROM products
          WHERE products MATCH ?1
          ORDER BY rank
          LIMIT 200",
    )?;
    let rows = stmt.query_map(params![fts_query], |row| {
        Ok(Product {
            name: row.get(0)?,
            brand: row.get(1)?,
            store: row.get(2)?,
            barcode: row.get(3)?,
            shelf_price: row.get(4)?,
            unit_price: row.get(5)?,
            unit: row.get(6)?,
            scraped_date: row.get(7)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

/// Strip Croatian diacritics and lowercase — used for both indexing and querying.
pub fn normalize(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'š' | 'Š' => 's',
            'č' | 'Č' => 'c',
            'ć' | 'Ć' => 'c',
            'ž' | 'Ž' => 'z',
            'đ' | 'Đ' => 'd',
            c => c,
        })
        .collect::<String>()
        .to_lowercase()
}

/// Build an FTS5 match expression against name_search only.
/// Tokens are OR-ed so partial matches are found; Gemini then filters semantically.
fn build_fts_query(query: &str) -> String {
    let tokens: Vec<String> = normalize(query)
        .split_whitespace()
        .map(|w| w.chars().filter(|c| c.is_alphanumeric()).collect::<String>())
        .filter(|w| !w.is_empty())
        .map(|w| format!("name_search:{w}"))
        .collect();
    tokens.join(" OR ")
}
