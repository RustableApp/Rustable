//! Incremental, bounded RESP2 decoding for Pub/Sub's flat arrays.
//! Partial headers and binary payloads survive socket read timeouts.
use std::io::{self, BufRead};
pub(crate) const FRAME_LIMIT: usize = 1024 * 1024;
const ITEM_LIMIT: usize = 1000;
const HEADER_LIMIT: usize = 4096;
#[derive(Default)]
pub(crate) struct Decoder {
    header: Vec<u8>,
    bulk: Option<(usize, Vec<u8>)>,
    array: Option<(usize, Vec<redis::Value>)>,
    bytes: usize,
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
impl Decoder {
    pub fn next(&mut self, reader: &mut impl BufRead) -> io::Result<redis::Value> {
        loop {
            if let Some((length, bytes)) = &mut self.bulk {
                let input = reader.fill_buf()?;
                if input.is_empty() {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                let count = input.len().min(*length + 2 - bytes.len());
                bytes.extend_from_slice(&input[..count]);
                self.bytes += count;
                reader.consume(count);
                if bytes.len() != *length + 2 {
                    continue;
                }
                if !bytes.ends_with(b"\r\n") {
                    return Err(invalid("Malformed Pub/Sub bulk terminator"));
                }
                let (length, mut bytes) = self.bulk.take().unwrap();
                bytes.truncate(length);
                if let Some(value) = self.finish(redis::Value::BulkString(bytes)) {
                    return Ok(value);
                }
                continue;
            }
            let input = reader.fill_buf()?;
            if input.is_empty() {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            let count = input
                .iter()
                .position(|b| *b == b'\n')
                .map_or(input.len(), |i| i + 1);
            if self.header.len() + count > HEADER_LIMIT {
                return Err(invalid("Pub/Sub header exceeds 4 KiB"));
            }
            if self.bytes + count > FRAME_LIMIT {
                return Err(invalid("Pub/Sub frame exceeds 1 MiB"));
            }
            self.header.extend_from_slice(&input[..count]);
            self.bytes += count;
            reader.consume(count);
            if self.header.last() != Some(&b'\n') {
                continue;
            }
            if !self.header.ends_with(b"\r\n") || self.header.len() < 3 {
                return Err(invalid("Malformed Pub/Sub header"));
            }
            let header = std::mem::take(&mut self.header);
            let value = &header[1..header.len() - 2];
            let integer = || -> io::Result<i64> {
                std::str::from_utf8(value)
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .ok_or_else(|| invalid("Invalid Pub/Sub integer"))
            };
            let value = match header[0] {
                b'+' => redis::Value::SimpleString(
                    std::str::from_utf8(value)
                        .map_err(|_| invalid("Invalid Redis status"))?
                        .into(),
                ),
                b'-' => {
                    return Err(io::Error::other(
                        String::from_utf8_lossy(value).into_owned(),
                    ));
                }
                b':' => redis::Value::Int(integer()?),
                b'$' => {
                    let length = integer()?;
                    if length == -1 {
                        redis::Value::Nil
                    } else {
                        let length = usize::try_from(length)
                            .map_err(|_| invalid("Invalid Pub/Sub bulk length"))?;
                        if self.bytes > FRAME_LIMIT - 2 || length > FRAME_LIMIT - self.bytes - 2 {
                            return Err(invalid("Pub/Sub frame exceeds 1 MiB"));
                        }
                        self.bulk = Some((length, Vec::with_capacity(length + 2)));
                        continue;
                    }
                }
                b'*' => {
                    if self.array.is_some() {
                        return Err(invalid("Nested Pub/Sub arrays are unsupported"));
                    }
                    let length = integer()?;
                    if length == -1 {
                        redis::Value::Nil
                    } else {
                        let length = usize::try_from(length)
                            .map_err(|_| invalid("Invalid Pub/Sub array length"))?;
                        if length > ITEM_LIMIT {
                            return Err(invalid(
                                "Pub/Sub response exceeds 1,000 entries; narrow the pattern",
                            ));
                        }
                        if length == 0 {
                            redis::Value::Array(Vec::new())
                        } else {
                            self.array = Some((length, Vec::with_capacity(length)));
                            continue;
                        }
                    }
                }
                _ => return Err(invalid("Unexpected Pub/Sub RESP type")),
            };
            if let Some(value) = self.finish(value) {
                return Ok(value);
            }
        }
    }
    fn finish(&mut self, value: redis::Value) -> Option<redis::Value> {
        let value = if let Some((remaining, values)) = &mut self.array {
            values.push(value);
            *remaining -= 1;
            if *remaining != 0 {
                return None;
            }
            redis::Value::Array(self.array.take().unwrap().1)
        } else {
            value
        };
        self.bytes = 0;
        Some(value)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Read};
    #[test]
    fn binary_payloads_and_sequential_frames() {
        let wire = b"+OK\r\n*3\r\n$7\r\nmessage\r\n$3\r\nc\0x\r\n$5\r\n\0\r\n\xffx\r\n:2\r\n";
        let mut reader = BufReader::with_capacity(2, &wire[..]);
        let mut decoder = Decoder::default();
        assert!(matches!(
            decoder.next(&mut reader).unwrap(),
            redis::Value::SimpleString(_)
        ));
        let redis::Value::Array(values) = decoder.next(&mut reader).unwrap() else {
            panic!()
        };
        assert_eq!(
            values,
            vec![
                redis::Value::BulkString(b"message".to_vec()),
                redis::Value::BulkString(b"c\0x".to_vec()),
                redis::Value::BulkString(b"\0\r\n\xffx".to_vec())
            ]
        );
        assert_eq!(decoder.next(&mut reader).unwrap(), redis::Value::Int(2));
    }
    #[test]
    fn every_byte_can_be_interrupted_without_losing_state() {
        struct Fragmented {
            input: std::collections::VecDeque<u8>,
            timeout: bool,
        }
        impl Read for Fragmented {
            fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
                self.timeout = !self.timeout;
                if self.timeout {
                    return Err(io::ErrorKind::TimedOut.into());
                }
                if let Some(b) = self.input.pop_front() {
                    out[0] = b;
                    Ok(1)
                } else {
                    Ok(0)
                }
            }
        }
        let wire = b"*3\r\n$9\r\nsubscribe\r\n$5\r\nc\r\n\0x\r\n:1\r\n";
        let mut reader = BufReader::new(Fragmented {
            input: wire.to_vec().into(),
            timeout: false,
        });
        let mut decoder = Decoder::default();
        let result = loop {
            match decoder.next(&mut reader) {
                Ok(value) => break value,
                Err(error) => assert_eq!(error.kind(), io::ErrorKind::TimedOut),
            }
        };
        assert_eq!(
            result,
            redis::Value::Array(vec![
                redis::Value::BulkString(b"subscribe".to_vec()),
                redis::Value::BulkString(b"c\r\n\0x".to_vec()),
                redis::Value::Int(1)
            ])
        );
    }
    #[test]
    fn limits_and_malformed_frames_are_rejected_before_payload_allocation() {
        for wire in [
            b"$1048576\r\n".as_slice(),
            b"*1001\r\n",
            b"*1\r\n*1\r\n",
            b"$-2\r\n",
            b"$18446744073709551615\r\n",
            b"$1\r\nx!!",
            b"+bad\n",
            b"?oops\r\n",
        ] {
            let mut decoder = Decoder::default();
            assert_eq!(
                decoder.next(&mut &wire[..]).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
        let mut decoder = Decoder::default();
        assert_eq!(
            decoder
                .next(&mut &b"-NOPERM fixture\r\n"[..])
                .unwrap_err()
                .to_string(),
            "NOPERM fixture"
        );
        let input = vec![b'x'; HEADER_LIMIT + 1];
        assert!(Decoder::default().next(&mut &input[..]).is_err());
    }
}
