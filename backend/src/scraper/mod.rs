pub mod eurospin;
pub mod kaufland;
pub mod konzum;
pub mod ktc;
pub mod lidl;
pub mod spar;

/// A normalised product row ready for DB insertion.
#[derive(Debug, Clone)]
pub struct RawProduct {
    pub name: String,
    pub brand: Option<String>,
    pub barcode: Option<String>,
    pub shelf_price: f64,
    /// EUR per reference unit (l / kg / kom). Already computed by the retailer.
    pub unit_price: Option<f64>,
    /// Normalised reference unit: "l", "kg", or "kom".
    pub unit: Option<String>,
}

// ── Common utilities ───────────────────────────────────────────────────────────

/// Normalise a raw unit string to "l", "kg", or "kom".
/// Input may be e.g. "L", "LT", "ml", "kg", "KOM", "ko", "komad", "30komad", "1,5l".
pub fn normalize_unit(raw: &str) -> Option<&'static str> {
    // Strip leading digits + punctuation that some stores embed (e.g. "1,5l", "30komad")
    let alpha_start = raw
        .find(|c: char| c.is_ascii_alphabetic())
        .unwrap_or(raw.len());
    let unit_part = raw[alpha_start..].trim();
    match unit_part.to_uppercase().as_str() {
        "ML" | "CL" | "DL" | "L" | "LI" | "LIT" | "LT" => Some("l"),
        "G" | "GR" | "GRM" | "DAG" | "KG" => Some("kg"),
        "KOM" | "KO" | "KOS" | "KS" | "KOMAD" | "PAK" => Some("kom"),
        _ => None,
    }
}

/// Parse a decimal number that may use comma or dot as separator.
pub fn parse_decimal(s: &str) -> Option<f64> {
    let v: f64 = s.trim().replace(',', ".").parse().ok()?;
    if v >= 0.0 { Some(v) } else { None }
}

/// Find the index of a header by trying multiple candidate names.
/// Normalises both sides: lowercase + collapse spaces/hyphens to underscores.
/// Also strips surrounding quotes from headers (some stores quote header names).
/// Falls back to prefix-match where the remainder starts with `(` (handles unit suffixes
/// like `"MPC (EUR)"` matching candidate `"mpc"`, or `"maloprod.cijena(EUR)"` matching
/// candidate `"maloprod.cijena"`).
pub fn col(headers: &csv::StringRecord, candidates: &[&str]) -> Option<usize> {
    fn norm(s: &str) -> String {
        s.trim()
            .trim_matches('"')
            .to_lowercase()
            .replace([' ', '-'], "_")
    }
    let lower: Vec<String> = headers.iter().map(|h| norm(h)).collect();
    for c in candidates {
        let needle = norm(c);
        // Exact match
        if let Some(i) = lower.iter().position(|h| *h == needle) {
            return Some(i);
        }
        // Prefix match: needle followed by optional `_` then `(` (for unit/currency suffixes)
        if let Some(i) = lower.iter().position(|h| {
            h.starts_with(&needle)
                && h[needle.len()..].trim_start_matches('_').starts_with('(')
        }) {
            return Some(i);
        }
    }
    None
}

/// Get a trimmed field value by column index, returning None for empty strings.
pub fn field(row: &csv::StringRecord, idx: usize) -> Option<&str> {
    row.get(idx).map(str::trim).filter(|s| !s.is_empty())
}

/// Decode Windows-1250 bytes to a UTF-8 string.
pub fn decode_cp1250(raw: &[u8]) -> std::borrow::Cow<'_, str> {
    let (cow, _, _) = encoding_rs::WINDOWS_1250.decode(raw);
    cow
}

/// Build a CSV reader with auto-detected field delimiter (tab, semicolon, or comma).
/// Looks at the first line; whichever separator is most frequent wins.
pub fn csv_reader(data: &[u8]) -> csv::Reader<&[u8]> {
    // Skip UTF-8 BOM if present
    let data = data.strip_prefix(b"\xef\xbb\xbf").unwrap_or(data);
    let first = data.split(|&b| b == b'\n' || b == b'\r').next().unwrap_or(&[]);
    let tabs = first.iter().filter(|&&b| b == b'\t').count();
    let semis = first.iter().filter(|&&b| b == b';').count();
    let commas = first.iter().filter(|&&b| b == b',').count();
    let delim = if tabs >= semis && tabs >= commas {
        b'\t'
    } else if semis >= commas {
        b';'
    } else {
        b','
    };
    csv::ReaderBuilder::new()
        .delimiter(delim)
        .flexible(true)
        .from_reader(data)
}
