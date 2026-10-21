//! The pipe protocol between the core and this helper.
//!
//! Requests arrive on stdin as frames: one tag byte, a little-endian `u32` payload length, then
//! the payload. Every request is answered with exactly one line of compact JSON on stdout.
//!
//! | tag | payload                                        | answer                         |
//! |-----|------------------------------------------------|--------------------------------|
//! | `G` | `{"sampleRate":16000,"grammar":["e four",…]}`  | `{"ok":true}`                  |
//! | `A` | 16-bit little-endian mono PCM                  | `{"partial":…}` or `{"result":…}` |
//! | `F` | empty                                          | `{"result":…}`                 |
//!
//! At start the helper answers `{"ready":true}` once the model has loaded, or `{"error":…}` and
//! exits. A request that fails is answered with `{"error":…}` and the helper keeps running.

use std::io::{self, Read};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The largest payload accepted: far above a 100 ms chunk, far below anything harmful.
pub const MAX_PAYLOAD: usize = 1 << 20;
/// The most phrases one grammar may hold, and the longest phrase.
pub const MAX_PHRASES: usize = 4096;
pub const MAX_PHRASE_CHARS: usize = 200;

#[derive(Debug, PartialEq)]
pub enum Request {
    Grammar(Grammar),
    Audio(Vec<i16>),
    Final,
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Grammar {
    pub sample_rate: f32,
    pub grammar: Vec<String>,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// The next request, or `None` when stdin closed between frames.
pub fn read_request(input: &mut impl Read) -> io::Result<Option<Request>> {
    let mut tag = [0u8; 1];
    if input.read(&mut tag)? == 0 {
        return Ok(None);
    }
    let mut length = [0u8; 4];
    input.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_PAYLOAD {
        return Err(invalid("request too large"));
    }
    let mut payload = vec![0u8; length];
    input.read_exact(&mut payload)?;
    match tag[0] {
        b'G' => {
            let grammar: Grammar =
                serde_json::from_slice(&payload).map_err(|_| invalid("invalid grammar"))?;
            let valid_rate = grammar.sample_rate.is_finite()
                && (8_000.0..=192_000.0).contains(&grammar.sample_rate);
            let valid_phrases = grammar.grammar.len() <= MAX_PHRASES
                && grammar
                    .grammar
                    .iter()
                    .all(|phrase| phrase.chars().count() <= MAX_PHRASE_CHARS);
            if !valid_rate || !valid_phrases {
                return Err(invalid("invalid grammar"));
            }
            Ok(Some(Request::Grammar(grammar)))
        }
        b'A' if length.is_multiple_of(2) => Ok(Some(Request::Audio(
            payload
                .chunks_exact(2)
                .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
                .collect(),
        ))),
        b'A' => Err(invalid("odd audio length")),
        b'F' if length == 0 => Ok(Some(Request::Final)),
        _ => Err(invalid("unknown request")),
    }
}

/// One recognized word with Vosk's confidence.
#[derive(Debug, PartialEq, Serialize)]
pub struct Word {
    pub word: String,
    pub conf: f64,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct Recognized {
    pub text: String,
    pub words: Vec<Word>,
}

/// An answer line.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Answer {
    Ready(bool),
    Ok(bool),
    Partial(String),
    Result(Recognized),
    Error(String),
}

impl Answer {
    pub fn line(&self) -> String {
        // Serializing these plain values cannot fail.
        let mut text = serde_json::to_string(self).unwrap_or_else(|_| "{}".into());
        text.push('\n');
        text
    }
}

/// Vosk's partial result (`{"partial" : "e two"}`) as an answer.
pub fn partial(vosk: &str) -> Answer {
    let value: Value = serde_json::from_str(vosk).unwrap_or(Value::Null);
    Answer::Partial(value["partial"].as_str().unwrap_or_default().to_owned())
}

/// Vosk's result (`{"result" : [{"conf":…,"word":…},…], "text" : …}`) as an answer.
pub fn result(vosk: &str) -> Answer {
    let value: Value = serde_json::from_str(vosk).unwrap_or(Value::Null);
    let words = value["result"]
        .as_array()
        .map(|words| {
            words
                .iter()
                .filter_map(|word| {
                    Some(Word {
                        word: word["word"].as_str()?.to_owned(),
                        conf: word["conf"].as_f64().unwrap_or(0.0),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Answer::Result(Recognized {
        text: value["text"].as_str().unwrap_or_default().to_owned(),
        words,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(tag: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(payload);
        out
    }

    fn read_all(bytes: &[u8]) -> Vec<io::Result<Option<Request>>> {
        let mut input = bytes;
        let mut out = Vec::new();
        loop {
            let next = read_request(&mut input);
            let stop = !matches!(next, Ok(Some(_)));
            out.push(next);
            if stop {
                return out;
            }
        }
    }

    #[test]
    fn reads_each_request_kind_then_end_of_input() {
        let mut bytes = frame(
            b'G',
            br#"{"sampleRate":16000,"grammar":["e four","[unk]"]}"#,
        );
        bytes.extend(frame(b'A', &[1, 0, 0xff, 0xff]));
        bytes.extend(frame(b'F', &[]));
        let requests: Vec<Request> = read_all(&bytes)
            .into_iter()
            .map_while(|request| request.expect("valid"))
            .collect();
        assert_eq!(
            requests,
            vec![
                Request::Grammar(Grammar {
                    sample_rate: 16_000.0,
                    grammar: vec!["e four".into(), "[unk]".into()],
                }),
                Request::Audio(vec![1, -1]),
                Request::Final,
            ]
        );
    }

    #[test]
    fn rejects_malformed_frames() {
        for bytes in [
            frame(b'A', &[1, 2, 3]),
            frame(b'F', &[0]),
            frame(b'X', &[]),
            frame(b'G', b"[]"),
            frame(b'G', br#"{"sampleRate":0,"grammar":[]}"#),
            frame(b'G', br#"{"sampleRate":16000,"grammar":[],"extra":1}"#),
            vec![b'A', 0xff, 0xff, 0xff, 0xff],
            vec![b'A', 4, 0, 0, 0, 1],
        ] {
            assert!(read_request(&mut bytes.as_slice()).is_err(), "{bytes:?}");
        }
        let long = format!(
            r#"{{"sampleRate":16000,"grammar":["{}"]}}"#,
            "a".repeat(MAX_PHRASE_CHARS + 1)
        );
        assert!(read_request(&mut frame(b'G', long.as_bytes()).as_slice()).is_err());
    }

    #[test]
    fn rewrites_vosk_json_as_compact_lines() {
        let vosk = "{\n  \"result\" : [{\n      \"conf\" : 0.98,\n      \"end\" : 1.2,\n      \"start\" : 0.9,\n      \"word\" : \"e\"\n    }, {\n      \"conf\" : 1.0,\n      \"word\" : \"four\"\n    }],\n  \"text\" : \"e four\"\n}";
        assert_eq!(
            result(vosk).line(),
            "{\"result\":{\"text\":\"e four\",\"words\":[{\"word\":\"e\",\"conf\":0.98},{\"word\":\"four\",\"conf\":1.0}]}}\n"
        );
        assert_eq!(
            partial("{\n  \"partial\" : \"e fo\"\n}").line(),
            "{\"partial\":\"e fo\"}\n"
        );
        assert_eq!(
            result("{\n  \"text\" : \"\"\n}").line(),
            "{\"result\":{\"text\":\"\",\"words\":[]}}\n"
        );
        assert_eq!(Answer::Ready(true).line(), "{\"ready\":true}\n");
        assert_eq!(
            Answer::Error("no \"model\"".into()).line(),
            "{\"error\":\"no \\\"model\\\"\"}\n"
        );
    }
}
