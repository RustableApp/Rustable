//! Single-location JSON paths only: no wildcard, slice, union or recursive edits.
pub(crate) fn validate(path: &[u8], removing: bool) -> Result<(), String> {
    if path.is_empty() && !removing {
        return Ok(());
    } // Legacy root replacement.
    let error = "Use a single JSON path such as $.name, $[\"name\"][0], or $ for the document; no wildcards or filters.";
    if path.len() > 4096 || path.first() != Some(&b'$') || std::str::from_utf8(path).is_err() {
        return Err(error.into());
    }
    if removing && path == b"$" {
        return Err("Use Delete key to remove the whole document.".into());
    }
    let mut i = 1;
    let mut segments = 0;
    while i < path.len() {
        segments += 1;
        if segments > 128 {
            return Err("JSON paths are limited to 128 segments.".into());
        }
        match path[i] {
            b'.' => {
                i += 1;
                if i == path.len() || !(path[i].is_ascii_alphabetic() || path[i] == b'_') {
                    return Err(error.into());
                }
                i += 1;
                while i < path.len() && (path[i].is_ascii_alphanumeric() || path[i] == b'_') {
                    i += 1;
                }
            }
            b'[' => {
                i += 1;
                if path.get(i) == Some(&b'"') {
                    let start = i;
                    i += 1;
                    loop {
                        match path.get(i) {
                            Some(b'\\') => i += 2,
                            Some(b'"') => {
                                i += 1;
                                break;
                            }
                            Some(_) => i += 1,
                            None => return Err(error.into()),
                        }
                    }
                    serde_json::from_slice::<String>(&path[start..i])
                        .map_err(|_| error.to_string())?;
                } else {
                    let start = i;
                    while i < path.len() && path[i].is_ascii_digit() {
                        i += 1;
                    }
                    if start == i
                        || (i - start > 1 && path[start] == b'0')
                        || std::str::from_utf8(&path[start..i])
                            .unwrap()
                            .parse::<u32>()
                            .is_err()
                    {
                        return Err(error.into());
                    }
                }
                if path.get(i) != Some(&b']') {
                    return Err(error.into());
                }
                i += 1;
            }
            _ => return Err(error.into()),
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_single_locations_are_allowed() {
        for path in [
            "$",
            "$.name",
            "$[\"with.dot\"][0].child",
            "$[\"🙂\"]",
            "$[\"quote\\\"key\"]",
            "$[\"\"]",
        ] {
            assert!(validate(path.as_bytes(), false).is_ok(), "{path}");
        }
        for path in [
            "$..name",
            "$.*",
            "$[0:2]",
            "$[1,2]",
            "$[-1]",
            "$[01]",
            "$[4294967296]",
            "$[?(@.x)]",
            "$['name']",
            "$[\"a\",\"b\"]",
            "$[\"x\\",
            "$[\"x\"]garbage",
            "$.",
        ] {
            assert!(validate(path.as_bytes(), false).is_err(), "{path}");
        }
        assert!(validate(b"$", true).is_err());
        assert!(validate(b"", true).is_err());
        assert!(validate(b"", false).is_ok());
        assert!(validate(format!("${}", ".x".repeat(129)).as_bytes(), false).is_err());
    }
}
