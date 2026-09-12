//! Bounded framing for evidence that cannot fit in a single worker response line.
use crate::{MAX_WORKER_RESPONSE_LINE_BYTES, WorkerResponse, read_worker_response_line};
use ketchup_core::graph::sha256_hex;
use std::io::{self, BufRead, Write};

// Keep the 64 KiB physical line limit; independently bound assembled evidence.
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const FRAME_PREFIX: &str = "RESPONSE_CHUNKS_V1 ";
const CHUNK_BYTES: usize = MAX_WORKER_RESPONSE_LINE_BYTES - 1;

pub fn write_worker_response(writer: &mut impl Write, response: &str) -> io::Result<()> {
    if response.len() > MAX_RESPONSE_BYTES || response.contains(['\r', '\n']) {
        return writeln!(writer, "ERR resource_limit");
    }
    if response.len() < MAX_WORKER_RESPONSE_LINE_BYTES {
        return writeln!(writer, "{response}");
    }
    writeln!(
        writer,
        "{FRAME_PREFIX}{} {}",
        response.len(),
        sha256_hex(response.as_bytes())
    )?;
    let mut remaining = response;
    while !remaining.is_empty() {
        let mut end = remaining.len().min(CHUNK_BYTES);
        while !remaining.is_char_boundary(end) {
            end -= 1;
        }
        writeln!(writer, "{}", &remaining[..end])?;
        remaining = &remaining[end..];
    }
    Ok(())
}

pub(crate) fn read_worker_response(reader: &mut impl BufRead) -> WorkerResponse {
    let line = match read_worker_response_line(reader) {
        WorkerResponse::Line(line) => line,
        other => return other,
    };
    let Some(header) = line.strip_prefix(FRAME_PREFIX) else {
        return WorkerResponse::Line(line);
    };
    let fields: Vec<_> = header.split_ascii_whitespace().collect();
    let Some(length) = fields.first().and_then(|value| value.parse::<usize>().ok()) else {
        return malformed("invalid framed response length");
    };
    if fields.len() != 2
        || !crate::is_sha256_digest(fields[1])
        || !(MAX_WORKER_RESPONSE_LINE_BYTES..=MAX_RESPONSE_BYTES).contains(&length)
    {
        return malformed("invalid or over-limit framed response header");
    }
    let mut response = String::with_capacity(length);
    while response.len() < length {
        let chunk = match read_worker_response_line(reader) {
            WorkerResponse::Line(chunk) => chunk,
            WorkerResponse::Exited => return malformed("truncated framed response"),
            other => return other,
        };
        let remaining = length - response.len();
        // Full non-final chunks prevent empty/tiny-frame streams from amplifying work.
        if chunk.is_empty()
            || chunk.len() > remaining
            || (chunk.len() < remaining && chunk.len() < CHUNK_BYTES - 3)
        {
            return malformed("invalid framed response chunk size");
        }
        response.push_str(&chunk);
    }
    if sha256_hex(response.as_bytes()) != fields[1] {
        return malformed("framed response digest mismatch");
    }
    WorkerResponse::Line(response)
}

fn malformed(message: &str) -> WorkerResponse {
    WorkerResponse::Malformed(message.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};

    fn encoded(response: &str) -> Vec<u8> {
        let mut bytes = Vec::new();
        write_worker_response(&mut bytes, response).unwrap();
        bytes
    }

    #[test]
    fn round_trip_boundaries_preserve_bytes_and_next_response() {
        for length in [
            0,
            CHUNK_BYTES,
            CHUNK_BYTES + 1,
            2 * CHUNK_BYTES,
            MAX_RESPONSE_BYTES,
        ] {
            let response = "x".repeat(length);
            let mut bytes = encoded(&response);
            assert!(
                bytes
                    .split_inclusive(|byte| *byte == b'\n')
                    .all(|line| line.len() <= MAX_WORKER_RESPONSE_LINE_BYTES)
            );
            bytes.extend_from_slice(b"PONG\n");
            let mut reader = BufReader::with_capacity(137, Cursor::new(bytes));
            assert!(
                matches!(read_worker_response(&mut reader), WorkerResponse::Line(line) if line == response)
            );
            assert!(
                matches!(read_worker_response(&mut reader), WorkerResponse::Line(line) if line == "PONG")
            );
            assert!(matches!(
                read_worker_response(&mut reader),
                WorkerResponse::Exited
            ));
        }
    }

    #[test]
    fn unicode_chunks_preserve_utf8_boundaries() {
        let response = "𐀀é".repeat(MAX_WORKER_RESPONSE_LINE_BYTES);
        assert!(
            matches!(read_worker_response(&mut Cursor::new(encoded(&response))), WorkerResponse::Line(line) if line == response)
        );
    }

    #[test]
    fn writer_refuses_oversized_or_multiline_evidence_without_partial_output() {
        for response in [
            "x".repeat(MAX_RESPONSE_BYTES + 1),
            "OK\nPONG".into(),
            "OK\rPONG".into(),
        ] {
            assert_eq!(encoded(&response), b"ERR resource_limit\n");
        }
    }

    #[test]
    fn physical_line_limit_is_unchanged() {
        for suffix in [b"\n".as_slice(), b"".as_slice()] {
            let mut bytes = vec![b'x'; MAX_WORKER_RESPONSE_LINE_BYTES];
            bytes.extend_from_slice(suffix);
            if suffix.is_empty() {
                bytes.push(b'x');
            }
            assert!(matches!(
                read_worker_response(&mut Cursor::new(bytes)),
                WorkerResponse::TooLarge
            ));
        }
    }

    #[test]
    fn rejects_invalid_headers_before_reading_body() {
        for header in [
            format!(
                "{FRAME_PREFIX}{} {}\n",
                MAX_RESPONSE_BYTES + 1,
                "a".repeat(64)
            ),
            format!("{FRAME_PREFIX}1 {}\n", "a".repeat(64)),
            format!("{FRAME_PREFIX}65536 invalid\n"),
            format!("{FRAME_PREFIX}65536 {} extra\n", "a".repeat(64)),
            format!("{FRAME_PREFIX}184467440737095516160 {}\n", "a".repeat(64)),
            format!("{FRAME_PREFIX}\n"),
        ] {
            let mut reader = Cursor::new(header.as_bytes());
            assert!(matches!(
                read_worker_response(&mut reader),
                WorkerResponse::Malformed(_)
            ));
            assert_eq!(reader.position() as usize, header.len());
        }
    }

    #[test]
    fn rejects_truncation_corruption_extra_bytes_and_tiny_chunks() {
        let response = "x".repeat(MAX_WORKER_RESPONSE_LINE_BYTES + 20);
        let valid = encoded(&response);
        let first_newline = valid.iter().position(|byte| *byte == b'\n').unwrap();
        let mut corrupt = valid.clone();
        corrupt[first_newline + 1] = b'y';
        let mut extra = valid.clone();
        extra.insert(extra.len() - 1, b'x');
        let mut tiny = valid[..=first_newline].to_vec();
        tiny.extend_from_slice(b"x\n");
        let mut empty = valid[..=first_newline].to_vec();
        empty.push(b'\n');
        for bytes in [
            valid[..valid.len() - 1].to_vec(),
            valid[..=first_newline].to_vec(),
            corrupt,
            extra,
            tiny,
            empty,
        ] {
            assert!(matches!(
                read_worker_response(&mut Cursor::new(bytes)),
                WorkerResponse::Malformed(_)
            ));
        }
    }
}
