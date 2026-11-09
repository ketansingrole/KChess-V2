//! Which failed Lichess response becomes which error. The
//! `LichessError` builds its message from these fields.

use serde_json::{Value, json};

use super::{Out, arg};
use crate::js;

/// The most of a response body an error shows (JavaScript's `slice(0, 250)`).
const DETAIL_UNITS: usize = 250;

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    Some(match method {
        "lichessError" => match arg(args, 0).and_then(|response| response.get("status")) {
            Some(status) => Ok(error_fields(
                status,
                arg(args, 1),
                arg(args, 2).and_then(Value::as_str).unwrap_or(""),
            )),
            None => Err("response.status must be a number".into()),
        },
        _ => return None,
    })
}

/// `{ status, endpoint, detail }`: an HTML page (Lichess's 404 for unknown users, or its 429 text
/// with a refresh tag) never becomes the detail; a JSON body does, cut to 250 characters.
fn error_fields(status: &Value, error: Option<&Value>, endpoint: &str) -> Value {
    let page = error.and_then(Value::as_str).is_some_and(|text| {
        text.trim_start_matches(js::is_space).starts_with('<')
            || ["<meta", "<html", "<!doctype"]
                .iter()
                .any(|tag| text.to_ascii_lowercase().contains(tag))
    });
    let detail = if page {
        match status.as_f64() {
            Some(404.0) => "not found".to_string(),
            Some(429.0) => "too many requests".to_string(),
            _ => "unexpected page response".to_string(),
        }
    } else {
        match error {
            None | Some(Value::Null) => String::new(),
            Some(value) => js::slice_utf16(&js::stringify(value), DETAIL_UNITS).to_string(),
        }
    };
    json!({ "status": status, "endpoint": endpoint, "detail": detail })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn html_pages_never_become_detail() {
        let page = json!("  <html>");
        let fields = error_fields(&json!(404), Some(&page), "GET /x");
        assert_eq!(fields["detail"], json!("not found"));
        let fields = error_fields(&json!(500), Some(&page), "GET /x");
        assert_eq!(fields["detail"], json!("unexpected page response"));
        // Lichess's 429: a line of text, then a refresh tag to its rate-limit page.
        let limited = json!(
            "Too many requests. Read https://http.lichess.org/429\n\n<meta http-equiv=\"refresh\" content=\"0;URL=https://http.lichess.org/429\">\n"
        );
        let fields = error_fields(&json!(429), Some(&limited), "GET /x");
        assert_eq!(fields["detail"], json!("too many requests"));
    }

    #[test]
    fn detail_is_cut_to_250_units() {
        let long = json!("x".repeat(400));
        let fields = error_fields(&json!(400), Some(&long), "POST /y");
        assert_eq!(fields["detail"].as_str().unwrap().len(), 250);
    }
}
