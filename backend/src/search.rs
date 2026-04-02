use crate::{
    db::{self, Db},
    gemini::GeminiClient,
};
use anyhow::Result;

/// Run FTS5 → Gemini filter → blacklist filter, return up to `limit` products sorted cheapest first.
pub async fn search(
    db: &Db,
    gemini: Option<&GeminiClient>,
    query: &str,
    blacklisted_brands: &[String],
    limit: usize,
) -> Result<Vec<db::Product>> {
    let db2 = db.clone();
    let q = query.to_string();

    let mut candidates =
        tokio::task::spawn_blocking(move || db::search_fts(&db2, &q)).await??;

    if candidates.is_empty() {
        return Ok(vec![]);
    }

    // Gemini semantic filter (skip when only 1 candidate or no Gemini key)
    if candidates.len() > 1 {
        if let Some(g) = gemini {
            let names: Vec<(&str, Option<&str>)> = candidates
                .iter()
                .map(|p| (p.name.as_str(), p.brand.as_deref()))
                .collect();
            match g.filter(query, &names).await {
                Ok(indices) if !indices.is_empty() => {
                    tracing::debug!(
                        "Gemini: {}/{} candidates kept for {:?}",
                        indices.len(), candidates.len(), query
                    );
                    candidates = indices
                        .into_iter()
                        .filter_map(|i| candidates.get(i).cloned())
                        .collect();
                }
                Ok(_) => tracing::debug!("Gemini: returned empty for {:?}, keeping all", query),
                Err(e) => tracing::warn!("Gemini filter error for {:?}: {e}", query),
            }
        }
    }

    // Blacklist filter
    if !blacklisted_brands.is_empty() {
        candidates.retain(|p| {
            let brand = p.brand.as_deref().unwrap_or("").to_lowercase();
            !blacklisted_brands
                .iter()
                .any(|b| brand.contains(&b.to_lowercase()))
        });
    }

    // Sort cheapest first: unit_price when same unit, else shelf_price
    candidates.sort_by(|a, b| effective_price(a).partial_cmp(&effective_price(b)).unwrap());

    candidates.truncate(limit);
    Ok(candidates)
}

/// Find a single cheapest product for syncing to Todoist.
pub async fn find_best(
    db: &Db,
    gemini: Option<&GeminiClient>,
    query: &str,
    blacklisted_brands: &[String],
) -> Result<Option<db::Product>> {
    let mut results = search(db, gemini, query, blacklisted_brands, 50).await?;
    if results.is_empty() {
        return Ok(None);
    }
    // Among results, pick the one with the lowest effective price
    results.sort_by(|a, b| effective_price(a).partial_cmp(&effective_price(b)).unwrap());
    Ok(results.into_iter().next())
}

fn effective_price(p: &db::Product) -> f64 {
    // Prefer unit_price for comparable units; fall back to shelf_price
    p.unit_price.unwrap_or(p.shelf_price)
}
