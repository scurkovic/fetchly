use super::{col, decode_cp1250, field, normalize_unit, parse_decimal, RawProduct};
use anyhow::{anyhow, Result};
use chrono::NaiveDate;
use reqwest::Client;

const BASE_URL: &str =
    "https://www.ktc.hr/ktcftp/Cjenici/RC%20DUGO%20SELO%20PJ-90/";
const STORE_PREFIX: &str =
    "TRGOVINA-II%20SAVSKI%20ODVOJAK%201%20DUGO%20SELO-PJ90-1-";

/// Timestamps to try, in priority order (HH, MM, SS).
/// The store opens around 07:00; files are generated shortly after.
const TIMESTAMPS: &[&str] = &[
    "071002", "070000", "070500", "071000", "071500",
    "072000", "073000", "075000", "080000", "065000",
];

pub async fn fetch(client: &Client, date: NaiveDate) -> Result<Vec<RawProduct>> {
    let date_str = date.format("%Y%m%d").to_string();
    for ts in TIMESTAMPS {
        let url = format!(
            "{BASE_URL}{STORE_PREFIX}{date_str}-{ts}.csv"
        );
        match try_download(client, &url).await {
            Ok(products) => return Ok(products),
            Err(e) => tracing::debug!("KTC {date_str}-{ts}: {e}"),
        }
    }
    Err(anyhow!("KTC: no CSV found for {date}"))
}

async fn try_download(client: &Client, url: &str) -> Result<Vec<RawProduct>> {
    let resp = client
        .get(url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow!("HTTP {}", resp.status()));
    }
    let bytes = resp.bytes().await?;
    parse_csv(decode_cp1250(&bytes).as_bytes())
}

fn parse_csv(data: &[u8]) -> Result<Vec<RawProduct>> {
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(b';')
        .flexible(true)
        .from_reader(data);

    let headers = rdr.headers()?.clone();

    let i_name = col(&headers, &["naziv_proizvoda", "naziv"]);
    let i_brand = col(&headers, &["marka_proizvoda", "marka", "brend"]);
    let i_barcode = col(&headers, &["barkod", "ean"]);
    let i_price = col(&headers, &["maloprodajna_cijena", "mpc", "cijena"]);
    let i_unit_price = col(&headers, &["cijena_za_jedinicu_mjere", "cijena/jed"]);
    let i_unit = col(&headers, &["jedinica_mjere", "jedinica mjere"]);

    let i_name = i_name.ok_or_else(|| anyhow!("KTC CSV: 'naziv' column not found"))?;
    let i_price = i_price.ok_or_else(|| anyhow!("KTC CSV: 'cijena' column not found"))?;

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
