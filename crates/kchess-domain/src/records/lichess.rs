//! Which failed Lichess response becomes which error. The
//! `LichessError` builds its message from these fields.

use serde_json::Value;

use crate::js;

/// The most of a response body an error shows (JavaScript's `slice(0, 250)`).
const DETAIL_UNITS: usize = 250;

/// The detail a failed Lichess response shows: an HTML page (Lichess's 404 for unknown users, or
/// its 429 text with a refresh tag) never becomes the detail; a JSON body does, cut to 250
/// characters. `status` is the response's, when it is a number.
pub fn error_detail(status: Option<f64>, error: Option<&Value>) -> String {
    let page = error.and_then(Value::as_str).is_some_and(|text| {
        text.trim_start_matches(js::is_space).starts_with('<')
            || ["<meta", "<html", "<!doctype"]
                .iter()
                .any(|tag| text.to_ascii_lowercase().contains(tag))
    });
    if page {
        match status {
            Some(404.0) => "not found".to_string(),
            Some(429.0) => "too many requests".to_string(),
            _ => "unexpected page response".to_string(),
        }
    } else {
        match error {
            None | Some(Value::Null) => String::new(),
            Some(value) => js::slice_utf16(&js::stringify(value), DETAIL_UNITS).to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn html_pages_never_become_detail() {
        let page = json!("  <html>");
        assert_eq!(error_detail(Some(404.0), Some(&page)), "not found");
        assert_eq!(
            error_detail(Some(500.0), Some(&page)),
            "unexpected page response"
        );
        // Lichess's 429: a line of text, then a refresh tag to its rate-limit page.
        let limited = json!(
            "Too many requests. Read https://http.lichess.org/429\n\n<meta http-equiv=\"refresh\" content=\"0;URL=https://http.lichess.org/429\">\n"
        );
        assert_eq!(
            error_detail(Some(429.0), Some(&limited)),
            "too many requests"
        );
    }

    #[test]
    fn detail_is_cut_to_250_units() {
        let long = json!("x".repeat(400));
        assert_eq!(error_detail(Some(400.0), Some(&long)).len(), 250);
    }
}
