pub const MAX_BRAND_LENGTH: usize = 200;
const MAX_LOGO_LENGTH: usize = 50000;

/// Sanitize a branding string: strip HTML tags, trim, and limit length.
pub fn sanitize_brand_input(input: &str) -> String {
    let trimmed = input.trim();
    let without_html = strip_html_tags(trimmed);
    let limited = if without_html.len() > MAX_BRAND_LENGTH {
        &without_html[..MAX_BRAND_LENGTH]
    } else {
        &without_html
    };
    limited.to_string()
}

/// Sanitize logo input (base64 or URL): trim and limit length.
pub fn sanitize_logo_input(input: &str) -> String {
    let trimmed = input.trim();
    let limited = if trimmed.len() > MAX_LOGO_LENGTH {
        &trimmed[..MAX_LOGO_LENGTH]
    } else {
        trimmed
    };
    limited.to_string()
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
