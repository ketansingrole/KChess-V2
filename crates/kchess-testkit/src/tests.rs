use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::AtomicUsize;

use serde_json::json;

use super::*;

/// Sends `request` to `server` and reads the whole response.
fn send(server: &Server, request: &str) -> String {
    let mut socket = TcpStream::connect(server.base().trim_start_matches("http://")).unwrap();
    socket.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    let mut reader = BufReader::new(socket);
    // Up to the end of a sized body, or of a chunked one; the server waits for us to hang up.
    let mut length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap() == 0 {
            return response;
        }
        response.push_str(&line);
        if let Some(value) = line.strip_prefix("Content-Length: ") {
            length = value.trim().parse::<usize>().ok();
        }
        if line == "\r\n" {
            break;
        }
    }
    match length {
        Some(length) => {
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            response.push_str(&String::from_utf8(body).unwrap());
        }
        None => loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap() == 0 {
                break;
            }
            response.push_str(&line);
            if line == "0\r\n" {
                break;
            }
        },
    }
    response
}

#[test]
fn a_request_is_read_with_its_headers_query_and_body() {
    let text = "POST /api/token?a=1&b=two%20words HTTP/1.1\r\nHost: x\r\ncontent-length: 7\r\nAuthorization: Bearer lip_x\r\n\r\ncode=42rest";
    let request = Request::read(&mut BufReader::new(text.as_bytes())).unwrap();
    assert_eq!(request.method, "POST");
    assert_eq!(request.path(), "/api/token");
    assert_eq!(request.query_string(), "a=1&b=two%20words");
    assert_eq!(request.query("b").as_deref(), Some("two%20words"));
    assert_eq!(request.query("c"), None);
    assert_eq!(request.header("authorization"), Some("Bearer lip_x"));
    assert_eq!(request.content_length(), 7);
    assert_eq!(request.body, "code=42");
    // A client that hangs up before the blank line sent no request.
    assert_eq!(
        Request::read(&mut BufReader::new(&b"GET / HTTP/1.1\r\n"[..])),
        None
    );
    assert_eq!(Request::read(&mut BufReader::new(&b""[..])), None);
    let bare = Request::from_head("GET /x HTTP/1.1").unwrap();
    assert_eq!(
        (bare.path(), bare.query_string(), bare.content_length()),
        ("/x", "", 0)
    );
}

#[test]
fn heads_name_their_status_length_and_chunks() {
    assert_eq!(
        sized_head(404, &[("Content-Type", "application/json")], 2),
        "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n"
    );
    assert!(chunked_head(200, "application/x-ndjson").contains("Transfer-Encoding: chunked\r\n"));
    assert_eq!(chunk("ab\n"), "3\r\nab\n\r\n");
    assert_eq!(chunk(""), "0\r\n\r\n");
    assert_eq!(status_text(429), "Too Many Requests");
    assert_eq!(status_text(299), "Status");
}

#[test]
fn a_server_answers_each_request_through_its_handler() {
    let server = Server::start(|request, out| match request.path() {
        "/json" => json(out, 200, &json!({ "body": request.body })),
        "/lines" => ndjson(out, &[json!(1), json!(2)]),
        _ => json(out, 404, &json!({ "error": "Not found" })),
    })
    .unwrap();
    let answer = send(
        &server,
        "POST /json HTTP/1.1\r\nContent-Length: 2\r\n\r\nhi",
    );
    assert!(answer.starts_with("HTTP/1.1 200 OK\r\n"), "{answer}");
    assert!(answer.ends_with("{\"body\":\"hi\"}"), "{answer}");
    let lines = send(&server, "GET /lines HTTP/1.1\r\n\r\n");
    assert!(
        lines.contains("application/x-ndjson") && lines.ends_with("1\n2\n"),
        "{lines}"
    );
    let missing = send(&server, "GET /nothing HTTP/1.1\r\n\r\n");
    assert!(missing.starts_with("HTTP/1.1 404 Not Found"), "{missing}");
}

#[test]
fn a_stream_sends_its_lines_then_keep_alives_until_it_closes() {
    static OPEN_FOR: AtomicUsize = AtomicUsize::new(0);
    let server = Server::start(|_, out| {
        ndjson_stream(
            out,
            vec![json!({ "first": true })],
            Duration::ZERO,
            || OPEN_FOR.fetch_add(1, Ordering::SeqCst) < 4,
            |sent| (sent == 1).then(|| json!({ "second": sent })),
        )
    })
    .unwrap();
    let answer = send(&server, "GET /stream HTTP/1.1\r\n\r\n");
    let body = answer.split_once("\r\n\r\n").unwrap().1;
    assert_eq!(
        body,
        "f\r\n{\"first\":true}\n\r\nd\r\n{\"second\":1}\n\r\n1\r\n\n\r\n1\r\n\n\r\n1\r\n\n\r\n0\r\n"
    );
}
