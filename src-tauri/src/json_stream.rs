use std::io::Read;

use crate::error::{AppError, AppResult, ErrorCode};

const READ_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct JsonStreamLimits {
    pub max_record_bytes: usize,
    pub max_nesting_depth: usize,
}

impl Default for JsonStreamLimits {
    fn default() -> Self {
        Self {
            max_record_bytes: 32 * 1024 * 1024,
            max_nesting_depth: 256,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct JsonStreamStats {
    pub bytes_read: u64,
    pub records_seen: u64,
    pub records_too_large: u64,
}

pub fn stream_json_array<R, F>(
    mut reader: R,
    limits: JsonStreamLimits,
    mut on_record: F,
) -> AppResult<JsonStreamStats>
where
    R: Read,
    F: FnMut(&[u8], u64) -> AppResult<()>,
{
    let mut stats = JsonStreamStats::default();
    let mut chunk = [0_u8; READ_CHUNK_BYTES];
    let mut started = false;
    let mut finished = false;
    let mut in_string = false;
    let mut escaped = false;
    let mut depth = 0_usize;
    let mut record = Vec::new();
    let mut record_started = false;
    let mut record_too_large = false;

    loop {
        let count = reader.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        stats.bytes_read += count as u64;

        for &byte in &chunk[..count] {
            if finished {
                if !byte.is_ascii_whitespace() {
                    return Err(ErrorCode::MalformedJson.into());
                }
                continue;
            }

            if !started {
                if byte.is_ascii_whitespace() {
                    continue;
                }
                if byte != b'[' {
                    return Err(ErrorCode::MalformedJson.into());
                }
                started = true;
                depth = 1;
                continue;
            }

            if !record_started {
                if byte.is_ascii_whitespace() || byte == b',' {
                    continue;
                }
                if byte == b']' {
                    finished = true;
                    depth = 0;
                    continue;
                }
                record_started = true;
                record.clear();
                record_too_large = false;
            }

            let is_boundary = !in_string && depth == 1 && (byte == b',' || byte == b']');
            if is_boundary {
                stats.records_seen += 1;
                if record_too_large {
                    stats.records_too_large += 1;
                } else {
                    trim_ascii_whitespace(&mut record);
                    if record.is_empty() {
                        return Err(ErrorCode::MalformedJson.into());
                    }
                    on_record(&record, stats.records_seen - 1)?;
                }
                record_started = false;
                record.clear();
                if byte == b']' {
                    finished = true;
                    depth = 0;
                }
                continue;
            }

            if !record_too_large {
                if record.len() >= limits.max_record_bytes {
                    record_too_large = true;
                    record.clear();
                } else {
                    record.push(byte);
                }
            }

            if in_string {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    in_string = false;
                }
                continue;
            }

            match byte {
                b'"' => in_string = true,
                b'{' | b'[' => {
                    depth = depth.saturating_add(1);
                    if depth > limits.max_nesting_depth {
                        return Err(ErrorCode::ResourceLimit.into());
                    }
                }
                b'}' => {
                    if depth <= 1 {
                        return Err(ErrorCode::MalformedJson.into());
                    }
                    depth -= 1;
                }
                b']' => {
                    if depth <= 1 {
                        return Err(ErrorCode::MalformedJson.into());
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
    }

    if !started || !finished || record_started || in_string || depth != 0 {
        return Err(AppError::Public(ErrorCode::MalformedJson));
    }

    Ok(stats)
}

fn trim_ascii_whitespace(value: &mut Vec<u8>) {
    let start = value
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(value.len());
    let end = value
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(start, |position| position + 1);
    if start > 0 {
        value.drain(..start);
    }
    value.truncate(end.saturating_sub(start));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_nested_records_without_splitting_strings() {
        let input = br#" [ {"a":"x,]","n":[1,2]}, {"b":true} ] "#;
        let mut records = Vec::new();
        let stats = stream_json_array(
            &input[..],
            JsonStreamLimits::default(),
            |record, ordinal| {
                records.push((ordinal, String::from_utf8(record.to_vec()).expect("utf8")));
                Ok(())
            },
        )
        .expect("stream succeeds");

        assert_eq!(stats.records_seen, 2);
        assert_eq!(records[0].1, r#"{"a":"x,]","n":[1,2]}"#);
        assert_eq!(records[1].1, r#"{"b":true}"#);
    }

    #[test]
    fn skips_oversized_records_without_allocating_the_remainder() {
        let input = br#"[{"large":"abcdefghij"},{"small":1}]"#;
        let mut records = Vec::new();
        let stats = stream_json_array(
            &input[..],
            JsonStreamLimits {
                max_record_bytes: 12,
                max_nesting_depth: 16,
            },
            |record, _| {
                records.push(record.to_vec());
                Ok(())
            },
        )
        .expect("stream succeeds");

        assert_eq!(stats.records_seen, 2);
        assert_eq!(stats.records_too_large, 1);
        assert_eq!(records, vec![br#"{"small":1}"#.to_vec()]);
    }

    #[test]
    fn rejects_trailing_content() {
        let result =
            stream_json_array(&br#"[{}]x"#[..], JsonStreamLimits::default(), |_, _| Ok(()));
        assert!(matches!(
            result,
            Err(AppError::Public(ErrorCode::MalformedJson))
        ));
    }
}
