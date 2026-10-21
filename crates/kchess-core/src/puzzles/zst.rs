//! Zstandard decoding for the puzzle download, over `ruzstd` (pure Rust, no C library).
//! `ruzstd`'s streaming decoder reads one frame and leaves checksums to the caller; this module
//! decodes every frame of the stream, skips skippable frames and verifies content checksums, as
//! the reference decoder does.

use std::io::{self, BufRead, BufReader, Read};

use ruzstd::decoding::StreamingDecoder;
use ruzstd::decoding::errors::{FrameDecoderError, ReadFrameHeaderError};

const CHUNK: usize = 64 * 1024;

fn invalid(message: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Decode all frames from `source`, handing the decompressed bytes to `sink` in chunks.
pub fn decode(source: impl Read, mut sink: impl FnMut(&[u8]) -> io::Result<()>) -> io::Result<()> {
    let mut source = BufReader::new(source);
    let mut buf = vec![0u8; CHUNK];
    while !source.fill_buf()?.is_empty() {
        let mut frame = match StreamingDecoder::new(&mut source) {
            Ok(frame) => frame,
            Err(FrameDecoderError::ReadFrameHeaderError(ReadFrameHeaderError::SkipFrame {
                length,
                ..
            })) => {
                let skipped =
                    io::copy(&mut (&mut source).take(u64::from(length)), &mut io::sink())?;
                if skipped != u64::from(length) {
                    return Err(invalid("truncated skippable frame"));
                }
                continue;
            }
            Err(cause) => return Err(invalid(cause)),
        };
        loop {
            let n = frame.read(&mut buf)?;
            if n == 0 {
                break;
            }
            sink(&buf[..n])?;
        }
        let decoder = &frame.decoder;
        if let (Some(expected), Some(actual)) = (
            decoder.get_checksum_from_data(),
            decoder.get_calculated_checksum(),
        ) && expected != actual
        {
            return Err(invalid("zstd content checksum mismatch"));
        }
    }
    Ok(())
}

/// A `Read` over chunks arriving on a channel; the end of the channel is the end of input.
pub struct ChannelReader {
    receive: tokio::sync::mpsc::Receiver<Vec<u8>>,
    chunk: Vec<u8>,
    offset: usize,
}

impl ChannelReader {
    pub fn new(receive: tokio::sync::mpsc::Receiver<Vec<u8>>) -> ChannelReader {
        ChannelReader {
            receive,
            chunk: Vec::new(),
            offset: 0,
        }
    }
}

impl Read for ChannelReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.offset == self.chunk.len() {
            match self.receive.blocking_recv() {
                Some(chunk) => {
                    self.chunk = chunk;
                    self.offset = 0;
                }
                None => return Ok(0),
            }
        }
        let n = out.len().min(self.chunk.len() - self.offset);
        out[..n].copy_from_slice(&self.chunk[self.offset..self.offset + n]);
        self.offset += n;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruzstd::encoding::{CompressionLevel, compress_to_vec};

    fn decoded(input: &[u8]) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        decode(input, |chunk| {
            out.extend_from_slice(chunk);
            Ok(())
        })?;
        Ok(out)
    }

    fn frame(text: &[u8]) -> Vec<u8> {
        compress_to_vec(text, CompressionLevel::Fastest)
    }

    #[test]
    fn decodes_concatenated_and_skippable_frames() {
        let large: Vec<u8> = (0..300_000u32)
            .flat_map(|i| (i % 251).to_le_bytes())
            .collect();
        let mut input = frame(b"PuzzleId,FEN\n");
        // A skippable frame: magic 0x184D2A50, 4-byte little-endian length, payload.
        input.extend_from_slice(&0x184D_2A50u32.to_le_bytes());
        input.extend_from_slice(&3u32.to_le_bytes());
        input.extend_from_slice(b"xyz");
        input.extend_from_slice(&frame(&large));
        let mut expected = b"PuzzleId,FEN\n".to_vec();
        expected.extend_from_slice(&large);
        assert_eq!(decoded(&input).expect("decode"), expected);
    }

    #[test]
    fn empty_input_decodes_to_nothing() {
        assert_eq!(decoded(&[]).expect("decode"), Vec::<u8>::new());
    }

    #[test]
    fn rejects_garbage_and_truncation() {
        assert!(decoded(b"not zstd at all").is_err());
        let whole = frame(&[b'a'; 10_000]);
        assert!(decoded(&whole[..whole.len() - 3]).is_err());
        let mut skippable = 0x184D_2A50u32.to_le_bytes().to_vec();
        skippable.extend_from_slice(&10u32.to_le_bytes());
        skippable.extend_from_slice(b"short");
        assert!(decoded(&skippable).is_err());
    }

    #[test]
    fn channel_reader_reassembles_chunks() {
        let (send, receive) = tokio::sync::mpsc::channel(4);
        let input = frame(b"a,b,c\n1,2,3\n");
        let feeder = std::thread::spawn(move || {
            for piece in input.chunks(3) {
                send.blocking_send(piece.to_vec()).expect("send");
            }
        });
        let mut out = Vec::new();
        decode(ChannelReader::new(receive), |chunk| {
            out.extend_from_slice(chunk);
            Ok(())
        })
        .expect("decode");
        feeder.join().expect("feeder");
        assert_eq!(out, b"a,b,c\n1,2,3\n");
    }
}
