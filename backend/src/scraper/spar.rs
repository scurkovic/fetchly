use super::{col, csv_reader, decode_cp1250, field, normalize_unit, parse_decimal, RawProduct};
use anyhow::{anyhow, Result};
use chrono::NaiveDate;
use reqwest::Client;
use serde::Deserialize;

const STORE_KEY_FRAGMENT: &str = "dugo_selo";
const STORE_KEY_ID: &str = "0335";

pub async fn fetch(client: &Client, date: NaiveDate) -> Result<Vec<RawProduct>> {
    let index_url = format!(
        "https://www.spar.hr/datoteke_cjenici/Cjenik{}.json",
        date.format("%Y%m%d")
    );

    let resp = client
        .get(&index_url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow!("SPAR index {}: HTTP {}", date, resp.status()));
    }

    // The JSON is an index: {"files": [{"name": "...", "URL": "...", "SHA": "..."}], "count": N}
    let index: SparIndex = resp.json().await?;

    let entry = index
        .files
        .into_iter()
        .find(|f| {
            let n = f.name.to_lowercase();
            n.contains(STORE_KEY_FRAGMENT) && n.contains(STORE_KEY_ID)
        })
        .ok_or_else(|| anyhow!("SPAR: Dugo Selo entry not found for {}", date))?;

    // Download the CSV (Windows-1250 encoded)
    let csv_bytes = client
        .get(&entry.url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await?
        .bytes()
        .await?;
    parse_csv(decode_cp1250(&csv_bytes).as_bytes())
}

fn parse_csv(data: &[u8]) -> Result<Vec<RawProduct>> {
    let mut rdr = csv_reader(data);

    let headers = rdr.headers()?.clone();

    let i_name = col(&headers, &["naziv_proizvoda", "naziv"]);
    let i_brand = col(&headers, &["marka_proizvoda", "marka", "brend"]);
    let i_barcode = col(&headers, &["barkod", "ean"]);
    // SPAR has two price columns: regular "MPC (EUR)" and sale "MPC za vrijeme posebnog oblika prodaje (EUR)".
    // Either may be empty; try both and use whichever is present.
    let i_price_regular = col(&headers, &["maloprodajna_cijena", "mpc", "cijena"]);
    let i_price_sale = col(&headers, &["mpc za vrijeme posebnog oblika prodaje"]);
    let i_unit_price = col(&headers, &["cijena_za_jedinicu_mjere", "cijena/jed"]);
    let i_unit = col(&headers, &["jedinica_mjere", "jedinica mjere"]);

    let i_name = i_name.ok_or_else(|| anyhow!("SPAR CSV: 'naziv' column not found"))?;
    if i_price_regular.is_none() && i_price_sale.is_none() {
        return Err(anyhow!("SPAR CSV: 'cijena' column not found"));
    }

    let mut out = Vec::new();
    for row in rdr.records() {
        let row = row?;
        let name = match field(&row, i_name) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let shelf_price = match i_price_regular
            .and_then(|i| field(&row, i))
            .or_else(|| i_price_sale.and_then(|i| field(&row, i)))
            .and_then(parse_decimal)
        {
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
struct SparIndex {
    files: Vec<SparEntry>,
}

#[derive(Deserialize)]
struct SparEntry {
    name: String,
    #[serde(rename = "URL")]
    url: String,
}
