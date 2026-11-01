//! Conservative validation of server expressions evaluated by managed writes.
//! MariaDB permits native UDFs here, whose external effects cannot be rolled back.
use super::*;
use sqlparser::tokenizer::{Token, Tokenizer, Whitespace};

pub(super) fn validate_import(driver: &mut MysqlDriver, schema: &str, table: &str) -> Result<(), String> {
    if driver.engine != "MariaDB" { return Ok(()); }
    let columns = driver.with_connection(|conn| conn.exec::<(String,Option<String>,Option<String>),_,_>(
        "SELECT COLUMN_NAME, COLUMN_DEFAULT, GENERATION_EXPRESSION FROM information_schema.COLUMNS WHERE TABLE_SCHEMA=? AND TABLE_NAME=?",
        (schema,table)))?;
    for (name, default, generated) in columns {
        for expression in [default, generated].into_iter().flatten() {
            validate(&expression, driver.mode)
                .map_err(|error| format!("Cannot import into column {name}: {error}"))?;
        }
    }
    // MariaDB exposes TABLE_NAME directly, including column-level CHECKs.
    let checks = driver.with_connection(|conn| conn.exec::<(String,String),_,_>(
        "SELECT CONSTRAINT_NAME, CHECK_CLAUSE FROM information_schema.CHECK_CONSTRAINTS WHERE CONSTRAINT_SCHEMA=? AND TABLE_NAME=?",
        (schema,table)))?;
    for (name, expression) in checks {
        validate(&expression, driver.mode)
            .map_err(|error| format!("Cannot import into table with CHECK {name}: {error}"))?;
    }
    Ok(())
}

fn validate(expression: &str, mode: Mode) -> Result<(), String> {
    let lexer = crate::sql::mysql::Lexer(mode);
    let tokens = Tokenizer::new(&lexer, expression).tokenize()
        .map_err(|e| format!("Unable to validate server expression: {e}"))?;
    // Do not discard executable version comments as if they were whitespace.
    if tokens.iter().any(|token| matches!(token, Token::Whitespace(
        Whitespace::MultiLineComment(_) | Whitespace::SingleLineComment { .. }))) {
        return Err("Commented server expressions are not supported for imports yet.".into());
    }
    let tokens = tokens.into_iter().filter(|token| !matches!(token, Token::Whitespace(_))).collect::<Vec<_>>();
    // MariaDB also supports sequence advancement without a parenthesized call.
    if tokens.iter().any(|token| matches!(token, Token::Word(word)
        if word.quote_style.is_none() && word.value.eq_ignore_ascii_case("NEXT"))) {
        return Err("Sequence expressions are not supported for transactional imports.".into());
    }
    for (i, token) in tokens.iter().enumerate() {
        if !matches!(tokens.get(i+1), Some(Token::LParen)) { continue; }
        if let Token::Word(word) = token {
            let name = word.value.to_ascii_uppercase();
            if word.quote_style.is_some() || (i > 0 && tokens[i-1] == Token::Period)
                || !supported_call(&name) {
                return Err(format!("Function {} is not validated for transactional imports yet.", word.value));
            }
        }
    }
    Ok(())
}

fn supported_call(name: &str) -> bool {
    // Include expression grammar/type constructors that also precede '('.
    // No stored routines, sequence operations, locks or file-access functions.
    matches!(name,
        "IN" | "NOT" | "AND" | "OR" | "XOR" | "IS" | "WHEN" | "THEN" | "ELSE" |
        "CAST" | "CONVERT" | "CHAR" | "BINARY" | "DECIMAL" | "NUMERIC" |
        "SIGNED" | "UNSIGNED" | "TIME" | "DATETIME" |
        "ABS" | "CEIL" | "CEILING" | "FLOOR" | "ROUND" | "TRUNCATE" | "MOD" |
        "POW" | "POWER" | "SQRT" | "SIGN" | "GREATEST" | "LEAST" |
        "COALESCE" | "IF" | "IFNULL" | "NULLIF" |
        "CONCAT" | "CONCAT_WS" | "LOWER" | "UPPER" | "LCASE" | "UCASE" |
        "LENGTH" | "CHAR_LENGTH" | "CHARACTER_LENGTH" | "OCTET_LENGTH" |
        "TRIM" | "LTRIM" | "RTRIM" | "SUBSTR" | "SUBSTRING" | "LEFT" | "RIGHT" |
        "REPLACE" | "REVERSE" | "LOCATE" | "INSTR" | "LPAD" | "RPAD" |
        "ASCII" | "ORD" | "HEX" | "UNHEX" | "MD5" | "SHA" | "SHA1" | "SHA2" |
        "CURRENT_TIMESTAMP" | "CURRENT_DATE" | "CURRENT_TIME" |
        "LOCALTIME" | "LOCALTIMESTAMP" | "NOW" | "UTC_TIMESTAMP" | "UTC_DATE" | "UTC_TIME" |
        "DATE" | "YEAR" | "MONTH" | "DAY" | "DAYOFMONTH" | "DAYOFWEEK" | "DAYOFYEAR" |
        "HOUR" | "MINUTE" | "SECOND" | "MICROSECOND" | "EXTRACT" |
        "DATE_ADD" | "DATE_SUB" | "ADDDATE" | "SUBDATE" | "DATEDIFF" | "TIMESTAMPDIFF" |
        "DATE_FORMAT" | "STR_TO_DATE" | "UNIX_TIMESTAMP" | "FROM_UNIXTIME" |
        "JSON_VALID" | "JSON_EXTRACT" | "JSON_UNQUOTE" | "JSON_LENGTH" | "JSON_TYPE" |
        "JSON_OBJECT" | "JSON_ARRAY" | "JSON_VALUE" | "UUID")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn permits_common_server_expressions_without_inspecting_literal_contents() {
        for expression in [
            "`id` * 2", "7", "'arbitrary_udf()'", "NULL",
            "concat(upper(`name`), ' unknown()')",
            "cast(`amount` as decimal(12,2))",
            "case when (`id` > 0) then (abs(`id`)) else (0) end",
            "`id` in (1,2) and (json_valid(`document`))",
            "current_timestamp(6)",
        ] {
            assert!(validate(expression,Mode::default()).is_ok(),"{expression}");
        }
    }
    #[test]
    fn rejects_unvalidated_and_disguised_calls() {
        for expression in [
            "external_write(id)", "abs(external_write(id))", "`abs`(id)",
            "evil.abs(id)", "abs /* hidden */ (id)", "/*! external_write(id) */",
            "nextval(seq)", "next value for seq", "get_lock('name', 10)", "load_file('/tmp/data')",
        ] {
            assert!(validate(expression,Mode::default()).is_err(),"{expression}");
        }
        assert!(validate("\"abs\"(id)",Mode { ansi_quotes:true,..Mode::default() }).is_err());
        assert!(validate("'safe\\' external_write(id)'",Mode::default()).is_ok());
        assert!(validate("'safe\\' external_write(id)'",Mode { no_backslash_escapes:true,..Mode::default() }).is_err());
    }
}
