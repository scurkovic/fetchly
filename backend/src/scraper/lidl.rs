use super::{col, csv_reader, decode_cp1250, field, normalize_unit, parse_decimal, RawProduct};
use anyhow::{anyhow, Result};
use chrono::NaiveDate;
use reqwest::Client;
use std::io::Read;

const CIJENE_PAGE: &str = "https://tvrtka.lidl.hr/cijene";

pub async fn fetch(client: &Client, date: NaiveDate) -> Result<Vec<RawProduct>> {
    let html = client
        .get(CIJENE_PAGE)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await?
        .text()
        .await?;

    // ZIP files are date-named: Popis_cijena_po_trgovinama_na_dan_DD.MM.YYYY.zip
    // "DugoSelo" is only inside the ZIP, not in the URL.
    let date_fragment = date.format("%d.%m.%Y").to_string(); // e.g. "01.04.2026"

    let zip_url = find_zip_for_date(&html, &date_fragment)
        .ok_or_else(|| anyhow!("Lidl: ZIP for {} not found on {}", date, CIJENE_PAGE))?;

    let bytes = client
        .get(&zip_url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await?
        .bytes()
        .await?
        .to_vec();

    parse_zip(bytes)
}

/// Find an href pointing to a ZIP whose URL contains the given date string.
fn find_zip_for_date(html: &str, date_fragment: &str) -> Option<String> {
    for href in extract_hrefs(html) {
        let lower = href.to_lowercase();
        if lower.ends_with(".zip") && href.contains(date_fragment) {
            return Some(absolutise(&href, "https://tvrtka.lidl.hr"));
        }
    }
    // Fallback: take the most recently-linked ZIP (first one, since page is newest-first)
    for href in extract_hrefs(html) {
        let lower = href.to_lowercase();
        if lower.ends_with(".zip") && lower.contains("popis_cijena") {
            return Some(absolutise(&href, "https://tvrtka.lidl.hr"));
        }
    }
    None
}

fn parse_zip(bytes: Vec<u8>) -> Result<Vec<RawProduct>> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor)?;

    // Look for CSV with "DugoSelo" in the name; fall back to any CSV
    let mut target_idx: Option<usize> = None;
    let mut fallback_idx: Option<usize> = None;
    for i in 0..archive.len() {
        let entry = archive.by_index(i)?;
        let name = entry.name().to_string();
        drop(entry);
        if !name.to_lowercase().ends_with(".csv") {
            continue;
        }
        if name.contains("DugoSelo") || name.contains("Dugo_Selo") || name.to_lowercase().contains("dugo") {
            target_idx = Some(i);
            break;
        }
        if fallback_idx.is_none() {
            fallback_idx = Some(i);
        }
    }
    let idx = target_idx.or(fallback_idx)
        .ok_or_else(|| anyhow!("Lidl ZIP: no CSV found"))?;
    read_and_parse(&mut archive, idx)
}

fn read_and_parse(archive: &mut zip::ZipArchive<std::io::Cursor<Vec<u8>>>, idx: usize) -> Result<Vec<RawProduct>> {
    let mut entry = archive.by_index(idx)?;
    let mut raw = Vec::new();
    entry.read_to_end(&mut raw)?;
    // Lidl CSVs are Windows-1250 encoded
    parse_csv(decode_cp1250(&raw).as_bytes())
}

fn parse_csv(data: &[u8]) -> Result<Vec<RawProduct>> {
    let mut rdr = csv_reader(data);

    let headers = rdr.headers()?.clone();

    let i_name = col(&headers, &["naziv_proizvoda", "naziv"]);
    let i_brand = col(&headers, &["marka_proizvoda", "marka", "brend"]);
    let i_barcode = col(&headers, &["barkod", "ean"]);
    let i_price = col(&headers, &["maloprodajna_cijena", "mpc", "cijena"]);
    let i_unit_price = col(&headers, &["cijena_za_jedinicu_mjere", "cijena/jed"]);
    let i_unit = col(&headers, &["jedinica_mjere", "jedinica mjere"]);

    let i_name = i_name.ok_or_else(|| anyhow!("Lidl CSV: 'naziv' column not found"))?;
    let i_price = i_price.ok_or_else(|| anyhow!("Lidl CSV: 'cijena' column not found"))?;

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

fn extract_hrefs(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(pos) = rest.to_lowercase().find("href=\"") {
        rest = &rest[pos + 6..];
        if let Some(end) = rest.find('"') {
            out.push(rest[..end].to_string());
            rest = &rest[end + 1..];
        } else {
            break;
        }
    }
    out
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
