//! Numbers as the window writes them, with a point; `KetchupApp::user_decimal`
//! turns the point into the user's decimal separator where they are shown.

/// A length or angle to a hundredth, trailing zeros dropped.
pub(crate) fn format_height(height: f64) -> String {
    format_decimals(height, 2)
}

/// `value` with at most `decimals` decimals, trailing zeros dropped; a value
/// that rounds to zero is `0`, never `-0`.
pub(crate) fn format_decimals(value: f64, decimals: usize) -> String {
    let formatted = fixed_decimals(value, decimals);
    if !formatted.contains('.') {
        return formatted;
    }
    formatted
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

/// `value` with exactly `decimals` decimals; a value that rounds to zero
/// carries no minus sign.
pub(crate) fn fixed_decimals(value: f64, decimals: usize) -> String {
    let formatted = format!("{value:.decimals$}");
    match formatted.strip_prefix('-') {
        Some(unsigned) if unsigned.bytes().all(|byte| matches!(byte, b'0' | b'.')) => {
            unsigned.to_owned()
        }
        _ => formatted,
    }
}
