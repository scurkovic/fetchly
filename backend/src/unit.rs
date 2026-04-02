/// Parse a quantity + unit pair from the cijene-api into a base amount and
/// whether it is a volume (true) or mass (false).
///
/// Returns `None` when the data is missing or unparseable.
pub fn parse_base_amount(
    quantity: Option<&str>,
    unit: Option<&str>,
) -> Option<(f64, bool)> {
    if let Some(q) = quantity {
        if let Some(result) = try_parse_quantity_unit(q, unit) {
            return Some(result);
        }
    }
    // Fall back to unit field alone, e.g. unit = "1l" or "68g"
    if let Some(u) = unit {
        if let Some(result) = try_parse_inline(u) {
            return Some(result);
        }
    }
    None
}

/// Return the normalised price per standard unit:
///   - liquids: EUR per litre  (base_amount in ml)
///   - solids:  EUR per kg     (base_amount in g)
pub fn unit_price(price: f64, base_amount: f64) -> f64 {
    price / base_amount * 1000.0
}

/// Human-readable pack size, e.g. "1 l", "750 ml", "1 kg", "350 g".
pub fn format_pack_size(base_amount: f64, is_volume: bool) -> String {
    if is_volume {
        if base_amount >= 1000.0 {
            let litres = base_amount / 1000.0;
            if litres.fract() == 0.0 {
                format!("{:.0} l", litres)
            } else {
                format!("{:.2} l", litres)
            }
        } else {
            format!("{:.0} ml", base_amount)
        }
    } else {
        if base_amount >= 1000.0 {
            let kg = base_amount / 1000.0;
            if kg.fract() == 0.0 {
                format!("{:.0} kg", kg)
            } else {
                format!("{:.2} kg", kg)
            }
        } else {
            format!("{:.0} g", base_amount)
        }
    }
}

/// Label for the denominator used in unit price display.
pub fn unit_label(is_volume: bool) -> &'static str {
    if is_volume { "l" } else { "kg" }
}

// ── Internal helpers ───────────────────────────────────────────────────────────

fn try_parse_quantity_unit(quantity: &str, unit: Option<&str>) -> Option<(f64, bool)> {
    let (num_str, embedded_unit) = split_number_unit(quantity.trim());
    let num = parse_number(num_str)?;

    let unit_str = embedded_unit
        .or_else(|| unit.map(str::trim))
        .unwrap_or("")
        .to_uppercase();

    to_base(num, &unit_str)
}

fn try_parse_inline(s: &str) -> Option<(f64, bool)> {
    let (num_str, unit) = split_number_unit(s.trim());
    let num = parse_number(num_str)?;
    let unit_str = unit.unwrap_or("").to_uppercase();
    to_base(num, &unit_str)
}

/// Split "68 G" -> ("68", Some("G")),  "1,000" -> ("1,000", None)
fn split_number_unit(s: &str) -> (&str, Option<&str>) {
    // Walk backwards to find where the alphabetic suffix begins
    let mut end = s.len();
    for (i, c) in s.char_indices().rev() {
        if c.is_ascii_alphabetic() {
            end = i;
        } else {
            break;
        }
    }
    let num_part = s[..end].trim();
    let unit_part = s[end..].trim();
    (num_part, if unit_part.is_empty() { None } else { Some(unit_part) })
}

fn parse_number(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let n = s.replace(',', ".");
    let n = if n.starts_with('.') { format!("0{n}") } else { n };
    n.parse::<f64>().ok().filter(|&v| v > 0.0)
}

/// Convert value + unit to (base_amount, is_volume).
/// Returns None for dimensionless units like KOM/PAK.
fn to_base(value: f64, unit: &str) -> Option<(f64, bool)> {
    match unit {
        "ML"                        => Some((value,          true)),
        "CL"                        => Some((value * 10.0,   true)),
        "DL"                        => Some((value * 100.0,  true)),
        "L" | "LI" | "LIT" | "LT"  => Some((value * 1000.0, true)),
        "G" | "GR" | "GRM"         => Some((value,          false)),
        "DAG"                       => Some((value * 10.0,   false)),
        "KG"                        => Some((value * 1000.0, false)),
        _                           => None,
    }
}
