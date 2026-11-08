//! Redis command text is not SQL: spaces separate arguments, newlines commands.
//! Quoted values and \xHH escapes preserve binary arguments without lossy UTF-8.
pub(crate) fn parse(text: &str) -> Result<Vec<Vec<Vec<u8>>>, String> {
    if text.len() > 1024 * 1024 {
        return Err("Commands exceed the 1 MiB editor limit.".into());
    }
    let mut commands = Vec::new();
    let mut args = Vec::new();
    let mut word = Vec::new();
    let mut started = false;
    let mut quote = None;
    let mut bytes = text.bytes().peekable();
    while let Some(byte) = bytes.next() {
        match byte {
            b'\\' => {
                started = true;
                let escaped = bytes.next().ok_or("Incomplete escape at end of command.")?;
                word.push(match escaped {
                    b'n' => b'\n',
                    b'r' => b'\r',
                    b't' => b'\t',
                    b'b' => 8,
                    b'a' => 7,
                    b'x' => {
                        let high = bytes
                            .next()
                            .and_then(hex)
                            .ok_or("Expected two hexadecimal digits after \\x.")?;
                        let low = bytes
                            .next()
                            .and_then(hex)
                            .ok_or("Expected two hexadecimal digits after \\x.")?;
                        high * 16 + low
                    }
                    other => other,
                });
            }
            b'\'' | b'"' if quote == Some(byte) => {
                quote = None;
            }
            b'\'' | b'"' if quote.is_none() => {
                quote = Some(byte);
                started = true;
            }
            b'\n' | b'\r' if quote.is_none() => {
                if started {
                    args.push(std::mem::take(&mut word));
                    started = false;
                }
                if !args.is_empty() {
                    commands.push(std::mem::take(&mut args));
                }
            }
            b' ' | b'\t' if quote.is_none() => {
                if started {
                    args.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            _ => {
                started = true;
                word.push(byte);
            }
        }
    }
    if quote.is_some() {
        return Err("Unclosed quoted argument.".into());
    }
    if started {
        args.push(word);
    }
    if !args.is_empty() {
        commands.push(args);
    }
    if commands.len() > 100 {
        return Err("Run at most 100 commands at a time.".into());
    }
    Ok(commands)
}
fn hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

/// Reversible display for keys/arguments, including embedded NUL and invalid UTF-8.
pub(crate) fn quote(bytes: &[u8]) -> String {
    let mut output = String::from("\"");
    for &byte in bytes {
        match byte {
            b'"' => output.push_str("\\\""),
            b'\\' => output.push_str("\\\\"),
            b'\n' => output.push_str("\\n"),
            b'\r' => output.push_str("\\r"),
            b'\t' => output.push_str("\\t"),
            32..=126 => output.push(byte as char),
            _ => {
                use std::fmt::Write;
                let _ = write!(output, "\\x{byte:02x}");
            }
        }
    }
    output.push('"');
    output
}

pub(crate) fn validate(args: &[Vec<u8>], read_only: bool) -> Result<(), String> {
    let name = args
        .first()
        .and_then(|name| std::str::from_utf8(name).ok())
        .ok_or("Enter a Redis command.")?
        .to_ascii_uppercase();
    // These commands alter protocol/session state, stream replies indefinitely,
    // or block the sole worker. Database selection is managed by the workspace.
    if matches!(
        name.as_str(),
        "AUTH"
            | "HELLO"
            | "SELECT"
            | "QUIT"
            | "RESET"
            | "MULTI"
            | "EXEC"
            | "DISCARD"
            | "WATCH"
            | "UNWATCH"
            | "MONITOR"
            | "SUBSCRIBE"
            | "PSUBSCRIBE"
            | "SSUBSCRIBE"
            | "UNSUBSCRIBE"
            | "PUNSUBSCRIBE"
            | "SUNSUBSCRIBE"
            | "SYNC"
            | "PSYNC"
            | "CLIENT"
            | "SHUTDOWN"
            | "DEBUG"
            | "BLPOP"
            | "BRPOP"
            | "BRPOPLPUSH"
            | "BLMOVE"
            | "BLMPOP"
            | "BZPOPMIN"
            | "BZPOPMAX"
            | "BZMPOP"
            | "WAIT"
            | "WAITAOF"
    ) {
        return Err(format!(
            "{name} is not supported in the command editor because it can change the session or block it. Use the connection settings for authentication and database selection."
        ));
    }
    if matches!(name.as_str(), "XREAD" | "XREADGROUP")
        && args
            .iter()
            .skip(1)
            .any(|arg| arg.eq_ignore_ascii_case(b"BLOCK"))
    {
        return Err("Blocking stream reads are not supported. Omit BLOCK.".into());
    }
    let diagnostic_read = args.get(1).is_some_and(|subcommand| {
        (name == "MEMORY" && subcommand.eq_ignore_ascii_case(b"USAGE"))
            || (name == "SLOWLOG" && subcommand.eq_ignore_ascii_case(b"GET"))
    });
    if read_only
        && !diagnostic_read
        && !matches!(
            name.as_str(),
            "PING"
                | "ECHO"
                | "GET"
                | "MGET"
                | "GETRANGE"
                | "STRLEN"
                | "TYPE"
                | "EXISTS"
                | "TTL"
                | "PTTL"
                | "EXPIRETIME"
                | "PEXPIRETIME"
                | "SCAN"
                | "DBSIZE"
                | "INFO"
                | "FT._LIST"
                | "FT.INFO"
                | "FT.SEARCH"
                | "FT.AGGREGATE"
                | "FT.PROFILE"
                | "VCARD" | "VRANGE" | "TS.RANGE" | "TS.REVRANGE" | "TS.GET" | "TS.INFO"
                | "JSON.GET"
                | "JSON.TYPE"
                | "JSON.OBJLEN"
                | "JSON.OBJKEYS"
                | "JSON.ARRLEN"
                | "JSON.STRLEN"
                | "TIME"
                | "HGET"
                | "HMGET"
                | "HGETALL"
                | "HSCAN"
                | "HLEN"
                | "HKEYS"
                | "HVALS"
                | "HEXISTS"
                | "HSTRLEN"
                | "LRANGE"
                | "LINDEX"
                | "LLEN"
                | "LPOS"
                | "SCARD"
                | "SISMEMBER"
                | "SMISMEMBER"
                | "SMEMBERS"
                | "SSCAN"
                | "ZRANGE"
                | "ZREVRANGE"
                | "ZRANGEBYSCORE"
                | "ZREVRANGEBYSCORE"
                | "ZCARD"
                | "ZCOUNT"
                | "ZSCORE"
                | "ZMSCORE"
                | "ZRANK"
                | "ZREVRANK"
                | "ZSCAN"
                | "XLEN"
                | "XRANGE"
                | "XREVRANGE"
                | "XREAD"
                | "XINFO"
        )
    {
        return Err(format!(
            "{name} is blocked in read-only mode. Unlock editing to run write or unclassified commands."
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn binary_arguments_round_trip() {
        let all: Vec<_> = (0..=255).collect();
        assert_eq!(
            parse(&format!("SET {} {}", quote(&all), quote(&[]))).unwrap(),
            vec![vec![b"SET".to_vec(), all, vec![]]]
        );
        assert_eq!(
            parse("SET \"a b\" 'hello; there'\nGET \"a b\"")
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            parse("SET café \"two\nlines\"").unwrap()[0][1],
            "café".as_bytes()
        );
    }
    #[test]
    fn malformed_input_is_rejected_before_execution() {
        for command in ["SET a 'unfinished", "GET a\\", "GET \\xgg", "GET \\x1"] {
            assert!(parse(command).is_err(), "{command}");
        }
    }
    #[test]
    fn session_and_write_guards_cannot_be_bypassed_by_case() {
        for text in [
            "set a b",
            "MEMORY PURGE",
            "SLOWLOG RESET",
            "MEMORY",
            "SLOWLOG",
            "EVAL 'return 1' 0",
            "FLUSHALL",
            "CONFIG SET x y",
            "FT.CREATE idx SCHEMA name TEXT",
            "FT.DROPINDEX idx",
            "FT.CONFIG SET TIMEOUT 0",
        ] {
            assert!(validate(&parse(text).unwrap()[0], true).is_err());
        }
        for text in [
            "gEt a",
            "mEmOrY uSaGe key SAMPLES 5",
            "sLoWlOg gEt 50",
            "SCAN 0 COUNT 100",
            "XRANGE s - + COUNT 10",
            "FT.AGGREGATE idx * GROUPBY 0 REDUCE COUNT 0",
            "FT.PROFILE idx SEARCH QUERY * LIMIT 0 10",
            "FT.INFO idx",
        ] {
            assert!(validate(&parse(text).unwrap()[0], true).is_ok());
        }
        for text in [
            "sUbScRiBe a",
            "MULTI",
            "CLIENT REPLY OFF",
            "XREAD BLOCK 0 STREAMS x $",
        ] {
            assert!(validate(&parse(text).unwrap()[0], false).is_err());
        }
    }
}
