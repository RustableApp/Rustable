//! Ordered, byte-bounded vector-set element pages (Redis 8.4+).
use crate::redis_value::{Cursor, PAGE_SIZE};
pub(crate) const PAGE_SCRIPT: &str = r#"
local values = redis.call('VRANGE', KEYS[1], ARGV[1], '+', 201)
local rows = {}
local bytes = 0
for i=1,math.min(#values,200) do
 if #values[i] > 65536 then return redis.error_reply('Vector element name exceeds the 64 KiB inspector limit') end
 bytes = bytes + #values[i]
 if bytes > 1048576 then return redis.error_reply('Vector page exceeds the 1 MiB inspector limit') end
 rows[#rows+1] = values[i]
end
return {rows, #values > 200 and 1 or 0}
"#;
pub(crate) fn boundary(cursor: &Cursor) -> Result<Vec<u8>, String> {
    match cursor {
        Cursor::Lex(None) => Ok(b"-".to_vec()),
        Cursor::Lex(Some(last)) if last.len() <= 65536 => {
            let mut boundary = Vec::with_capacity(last.len() + 1);
            boundary.push(b'(');
            boundary.extend_from_slice(last);
            Ok(boundary)
        }
        _ => Err("Invalid vector-set cursor. Refresh the key.".into()),
    }
}
pub(crate) fn page(
    value: redis::Value,
    cursor: &Cursor,
) -> Result<(redis::Value, Option<Cursor>), String> {
    let (rows, more): (Vec<Vec<u8>>, u8) =
        redis::from_redis_value(value).map_err(|e| format!("Invalid vector page: {e}"))?;
    if rows.len() > PAGE_SIZE as usize
        || more > 1
        || (more == 1 && rows.len() != PAGE_SIZE as usize)
        || rows.iter().any(|row| row.len() > 65536)
        || rows.iter().map(Vec::len).sum::<usize>() > 1048576
        || rows.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err("Vector page exceeds bounds or is not strictly ordered.".into());
    }
    if let Cursor::Lex(Some(last)) = cursor
        && rows.first().is_some_and(|row| row <= last)
    {
        return Err("Vector page did not advance beyond its cursor.".into());
    }
    let next = (more == 1).then(|| Cursor::Lex(rows.last().cloned()));
    Ok((
        redis::Value::Array(rows.into_iter().map(redis::Value::BulkString).collect()),
        next,
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn reply(rows: Vec<Vec<u8>>, more: i64) -> redis::Value {
        redis::Value::Array(vec![
            redis::Value::Array(rows.into_iter().map(redis::Value::BulkString).collect()),
            redis::Value::Int(more),
        ])
    }
    #[test]
    fn binary_cursors_and_response_bounds() {
        assert_eq!(
            boundary(&Cursor::Lex(Some(vec![0, 255]))).unwrap(),
            vec![b'(', 0, 255]
        );
        assert_eq!(boundary(&Cursor::Lex(Some(vec![]))).unwrap(), b"(");
        let rows: Vec<_> = (0..200)
            .map(|i| format!("member:{i:03}").into_bytes())
            .collect();
        assert_eq!(
            page(reply(rows, 1), &Cursor::Lex(None)).unwrap().1,
            Some(Cursor::Lex(Some(b"member:199".to_vec())))
        );
        for invalid in [
            reply(vec![b"b".to_vec(), b"a".to_vec()], 0),
            reply(vec![vec![0; 65537]], 0),
            reply(vec![], 1),
            reply(vec![], 2),
            reply((0..17).map(|i| vec![i; 65536]).collect(), 0),
            reply((0..201).map(|i| vec![i]).collect(), 0),
        ] {
            assert!(page(invalid, &Cursor::Lex(None)).is_err());
        }
        assert!(
            page(
                reply(vec![b"a".to_vec()], 0),
                &Cursor::Lex(Some(b"a".to_vec()))
            )
            .is_err()
        );
        assert!(boundary(&Cursor::Offset(0)).is_err());
    }
}
