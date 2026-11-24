use crate::api::Rejected;
use axum::http::StatusCode;

/// Longest brand title or subtitle kept, in characters (not bytes).
pub const MAX_BRAND_LENGTH: usize = 200;
/// Largest logo image file accepted (100 KB), the limit the branding dialog
/// also applies before upload.
pub const MAX_LOGO_FILE_BYTES: usize = 100_000;
/// Largest stored logo value in bytes: a base64 data: URL of a
/// [`MAX_LOGO_FILE_BYTES`] file (4/3 of it) plus room for the
/// `data:<mime>;base64,` prefix.
pub const MAX_LOGO_BYTES: usize = MAX_LOGO_FILE_BYTES.div_ceil(3) * 4 + 128;

/// Sanitize a branding string: strip HTML tags, trim, and keep at most
/// [`MAX_BRAND_LENGTH`] characters. Cuts on a character boundary, so a title
/// in any script (or with emoji) never splits a character.
pub fn sanitize_brand_input(input: &str) -> String {
    let without_html = strip_html_tags(input.trim());
    let kept: String = without_html.chars().take(MAX_BRAND_LENGTH).collect();
    kept.trim_end().to_string()
}

/// Check logo input (a data: URL or a link) and trim it. A logo over
/// [`MAX_LOGO_BYTES`] is refused with a 400 rather than cut: a cut image is
/// a corrupt image. Use `?` in a handler returning `AppError` to answer 400.
pub fn sanitize_logo_input(input: &str) -> Result<String, Rejected> {
    let trimmed = input.trim();
    if trimmed.len() > MAX_LOGO_BYTES {
        return Err(Rejected(
            StatusCode::BAD_REQUEST,
            format!(
                "Logo is too large: the image file may be at most {} KB",
                MAX_LOGO_FILE_BYTES / 1000
            ),
        ));
    }
    Ok(trimmed.to_string())
}

/// Check a brand accent colour: `#rgb` or `#rrggbb` hex, returned in
/// lower case; an empty string clears it (the default accent applies).
pub fn sanitize_brand_color(input: &str) -> Result<String, Rejected> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    let hex = trimmed.strip_prefix('#').unwrap_or("");
    if (hex.len() == 3 || hex.len() == 6) && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(trimmed.to_ascii_lowercase())
    } else {
        Err(Rejected(
            StatusCode::BAD_REQUEST,
            "Accent colour must be a hex colour like #22c55e".to_string(),
        ))
    }
}

pub(crate) fn strip_html_tags(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut in_tag = false;
    for ch in input.chars() {
        if ch == '<' {
            in_tag = true;
        } else if ch == '>' {
            in_tag = false;
        } else if !in_tag {
            result.push(ch);
        }
    }
    result
}
