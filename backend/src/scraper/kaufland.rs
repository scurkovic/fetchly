use super::{col, csv_reader, field, normalize_unit, parse_decimal, RawProduct};
use anyhow::{anyhow, Result};
use chrono::NaiveDate;
use reqwest::Client;
use serde::Deserialize;

const ASSET_LIST_URL: &str =
    "https://www.kaufland.hr/akcije-novosti/popis-mpc.assetSearch.id=assetList_1599847924.json";
const KAUFLAND_BASE: &str = "https://www.kaufland.hr";
const STORE_FRAGMENT: &str = "Dugo_Selo";

pub async fn fetch(client: &Client, date: NaiveDate) -> Result<Vec<RawProduct>> {
    let csv_url = discover_csv_url(client, date).await?;
    let text = client
        .get(&csv_url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await?
        .text()
        .await?;
    parse_csv(text.as_bytes())
}

async fn discover_csv_url(client: &Client, date: NaiveDate) -> Result<String> {
    let resp = client
        .get(ASSET_LIST_URL)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow!("Kaufland asset list: HTTP {}", resp.status()));
    }

    let entries: Vec<KauflandEntry> = resp.json().await?;

    // Kaufland filenames use DDMMYYYY format
    let date_str = date.format("%d%m%Y").to_string();

    // First try today's exact date; fall back to any Dugo Selo entry (most recent)
    let by_date = entries.iter().find(|e| {
        e.label.contains(STORE_FRAGMENT) && e.label.contains(&date_str)
    });
    let entry = by_date
        .or_else(|| entries.iter().find(|e| e.label.contains(STORE_FRAGMENT)))
        .ok_or_else(|| anyhow!("Kaufland: no CSV for {STORE_FRAGMENT}"))?;

    let url = if entry.path.starts_with("http") {
        entry.path.clone()
    } else {
        format!("{KAUFLAND_BASE}{}", entry.path)
    };
    Ok(url)
}

fn parse_csv(data: &[u8]) -> Result<Vec<RawProduct>> {
    let mut rdr = csv_reader(data);

    let headers = rdr.headers()?.clone();

    let i_name = col(&headers, &["naziv_proizvoda", "naziv"]);
    let i_brand = col(&headers, &["marka_proizvoda", "marka", "brend"]);
    let i_barcode = col(&headers, &["barkod", "ean"]);
    // Shelf price: prefer "mpc" / "maloprodajna_cijena", fall back to plain "cijena"
    let i_price = col(&headers, &["maloprodajna_cijena", "maloprod.cijena", "mpc", "cijena"]);
    // Kaufland's reference unit price column is "cijena jed.mj."
    let i_unit_price = col(
        &headers,
        &["cijena jed.mj.", "cijena_za_jedinicu_mjere", "cijena/jed"],
    );
    // Kaufland's reference unit column is "jed.mj." (NOT "jedinica mjere")
    let i_unit = col(&headers, &["jed.mj.", "jedinica_mjere", "jedinica mjere"]);

    let i_name = i_name.ok_or_else(|| anyhow!("Kaufland CSV: 'naziv' column not found"))?;
    let i_price = i_price.ok_or_else(|| anyhow!("Kaufland CSV: 'cijena' column not found"))?;

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

#[derive(Deserialize)]
struct KauflandEntry {
    #[serde(rename = "label")]
    label: String,
    #[serde(rename = "path")]
    path: String,
}
