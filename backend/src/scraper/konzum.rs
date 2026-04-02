use super::{col, csv_reader, field, normalize_unit, parse_decimal, RawProduct};
use anyhow::{anyhow, Result};
use chrono::NaiveDate;
use reqwest::Client;

const CJENICI_BASE: &str = "https://www.konzum.hr/cjenici";
const STORE_FRAGMENT: &str = "DUGO SELO";
const MAX_PAGES: usize = 20;

pub async fn fetch(client: &Client, date: NaiveDate) -> Result<Vec<RawProduct>> {
    let date_str = date.format("%Y-%m-%d").to_string();
    let csv_url = find_csv_url(client, &date_str).await?;
    let text = client
        .get(&csv_url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await?
        .text()
        .await?;
    parse_csv(text.as_bytes())
}

async fn find_csv_url(client: &Client, date_str: &str) -> Result<String> {
    for page in 1..=MAX_PAGES {
        let url = format!("{CJENICI_BASE}?date={date_str}&page={page}");
        let html = client
            .get(&url)
            .header("User-Agent", "Mozilla/5.0")
            .send()
            .await?
            .text()
            .await?;

        if let Some(csv_url) = find_store_link(&html, STORE_FRAGMENT) {
            return Ok(csv_url);
        }

        // Stop if no next page
        if !html.to_lowercase().contains("page=") || page_count_exceeded(&html, page) {
            break;
        }
    }
    Err(anyhow!("Konzum: '{STORE_FRAGMENT}' not found for {date_str}"))
}

/// Find an href in the HTML that is near the text STORE_FRAGMENT and points to a CSV.
fn find_store_link(html: &str, fragment: &str) -> Option<String> {
    let up = html.to_uppercase();
    let frag_pos = up.find(fragment)?;

    // Search backwards and forwards from the fragment for an href containing a .CSV link
    let window_start = frag_pos.saturating_sub(2000);
    let window_end = (frag_pos + 2000).min(html.len());
    let window = &html[window_start..window_end];

    extract_csv_href(window)
        .map(|href| absolutise(href, "https://www.konzum.hr"))
}

fn extract_csv_href(html: &str) -> Option<&str> {
    let lower = html.to_lowercase();
    // Look for href="..." where value ends in .csv
    let mut search = lower.as_str();
    let mut offset = 0;
    while let Some(pos) = search.find("href=\"") {
        let abs_pos = offset + pos + 6;
        let after = &html[abs_pos..];
        let end = after.find('"')?;
        let href = &after[..end];
        if href.to_lowercase().ends_with(".csv") || href.to_lowercase().contains(".csv") {
            return Some(href);
        }
        search = &search[pos + 6..];
        offset += pos + 6;
    }
    None
}

fn absolutise(href: &str, base: &str) -> String {
    if href.starts_with("http") {
        href.to_string()
    } else if href.starts_with("//") {
        format!("https:{href}")
    } else {
        format!("{base}{href}")
    }
}

fn page_count_exceeded(html: &str, current: usize) -> bool {
    // Heuristic: if the page HTML is very short or contains no product rows, stop
    html.len() < 500 || current >= MAX_PAGES
}

fn parse_csv(data: &[u8]) -> Result<Vec<RawProduct>> {
    let mut rdr = csv_reader(data);

    let headers = rdr.headers()?.clone();

    let i_name = col(&headers, &["naziv_proizvoda", "naziv"]);
    let i_brand = col(&headers, &["marka_proizvoda", "marka", "brend"]);
    let i_barcode = col(&headers, &["barkod", "ean"]);
    let i_price = col(&headers, &["maloprodajna_cijena", "mpc", "cijena"]);
    let i_unit_price = col(&headers, &["cijena_za_jedinicu_mjere", "cijena/jed"]);
    // Konzum uses "ko" for pieces — normalize_unit handles this
    let i_unit = col(&headers, &["jedinica_mjere", "jedinica mjere"]);

    let i_name = i_name.ok_or_else(|| anyhow!("Konzum CSV: 'naziv' column not found"))?;
    let i_price = i_price.ok_or_else(|| anyhow!("Konzum CSV: 'cijena' column not found"))?;

    let mut out = Vec::new();
    for row in rdr.records() {
        let row = row?;
        let name = match field(&row, i_name) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let shelf_price = match field(&row, i_price).and_then(parse_decimal) {
            Some(p) => p,
            None => continue,
        };
        let unit_price = i_unit_price
            .and_then(|i| field(&row, i))
            .and_then(parse_decimal);
        let unit = i_unit
            .and_then(|i| field(&row, i))
            .and_then(normalize_unit)
            .map(str::to_string);
        out.push(RawProduct {
            name,
            brand: i_brand.and_then(|i| field(&row, i)).map(str::to_string),
            barcode: i_barcode.and_then(|i| field(&row, i)).map(str::to_string),
            shelf_price,
            unit_price,
            unit,
        });
    }
    Ok(out)
}
