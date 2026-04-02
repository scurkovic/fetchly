use super::{col, csv_reader, decode_cp1250, field, normalize_unit, parse_decimal, RawProduct};
use anyhow::{anyhow, Result};
use chrono::NaiveDate;
use reqwest::Client;
use std::io::Read;

pub async fn fetch(client: &Client, date: NaiveDate) -> Result<Vec<RawProduct>> {
    // Try today, then yesterday (file may not be published until morning)
    for d in [date, date.pred_opt().unwrap_or(date)] {
        let url = format!(
            "https://www.eurospin.hr/wp-content/themes/eurospin/documenti-prezzi/cjenik_{}-7.30.zip",
            d.format("%d.%m.%Y")
        );
        match try_fetch(client, &url).await {
            Ok(products) => return Ok(products),
            Err(e) => tracing::warn!("Eurospin {d}: {e}"),
        }
    }
    Err(anyhow!("Eurospin: no ZIP found for {}", date))
}

async fn try_fetch(client: &Client, url: &str) -> Result<Vec<RawProduct>> {
    let resp = client
        .get(url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow!("HTTP {}", resp.status()));
    }
    let bytes = resp.bytes().await?;
    parse_zip(&bytes)
}

fn parse_zip(bytes: &[u8]) -> Result<Vec<RawProduct>> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor)?;

    // Find the Dugo Selo store CSV; fall back to first CSV
    let mut target_idx: Option<usize> = None;
    let mut fallback_idx: Option<usize> = None;
    for i in 0..archive.len() {
        let entry = archive.by_index(i)?;
        let name = entry.name().to_lowercase();
        drop(entry);
        if !name.ends_with(".csv") && !name.ends_with(".txt") {
            continue;
        }
        if name.contains("dugo_selo") || name.contains("dugo selo") {
            target_idx = Some(i);
            break;
        }
        if fallback_idx.is_none() {
            fallback_idx = Some(i);
        }
    }
    let idx = target_idx
        .or(fallback_idx)
        .ok_or_else(|| anyhow!("Eurospin ZIP: no CSV found"))?;
    let mut entry = archive.by_index(idx)?;
    let mut raw = Vec::new();
    entry.read_to_end(&mut raw)?;
    // Eurospin CSVs are Windows-1250 encoded
    parse_csv(decode_cp1250(&raw).as_bytes())
}

fn parse_csv(data: &[u8]) -> Result<Vec<RawProduct>> {
    let mut rdr = csv_reader(data);

    let headers = rdr.headers()?.clone();

    let i_name = col(&headers, &["naziv_proizvoda", "naziv"]);
    let i_brand = col(&headers, &["marka_proizvoda", "marka", "brend"]);
    let i_barcode = col(&headers, &["barkod", "ean"]);
    let i_price = col(&headers, &["maloprodajna_cijena", "maloprod.cijena", "mpc", "cijena"]);
    let i_unit_price = col(&headers, &["cijena_za_jedinicu_mjere", "cijena/jed"]);
    let i_unit = col(&headers, &["jedinica_mjere", "jedinica mjere"]);

    let i_name = i_name.ok_or_else(|| anyhow!("Eurospin CSV: 'naziv' column not found"))?;
    let i_price = i_price.ok_or_else(|| anyhow!("Eurospin CSV: 'cijena' column not found"))?;

    let mut out = Vec::new();
    for row in rdr.records() {
        let row = row?;
        let name = match field(&row, i_name) {
            Some(s) => s.to_string(),
            None => continue,
        };
        // Skip deli counter items (prefixed with *)
        if name.starts_with('*') {
            continue;
        }
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
