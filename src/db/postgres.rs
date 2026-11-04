//! PostgreSQL work stays on the database owner thread. Editor statements use a
//! description followed by execution; rows are streamed within UI limits.
mod array;
mod browse;
mod changes;
mod export;
mod interval;
mod import;
use super::*;
use ::postgres::{
    Client, NoTls, Row, RowIter,
    config::SslMode,
    fallible_iterator::FallibleIterator,
    types::{FromSql, ToSql, Type},
};
use postgres_native_tls::MakeTlsConnector;
use std::{error::Error, time::Instant};

pub(super) struct PostgresDriver {
    client: Client,
    config: crate::connection::PostgresConfig,
    cancel: Arc<AtomicBool>,
    tls: Option<MakeTlsConnector>,
    tunnel: Option<crate::ssh_tunnel::Tunnel>,
    schemas: SchemaCatalog,
    browse_catalog: Vec<(String, Row)>,
}

impl PostgresDriver {
    pub(super) fn open(
        config: &crate::connection::PostgresConfig,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let tunnel = if let Some(ssh) = &config.ssh {
            let mut ssh = ssh.clone();
            // One source of truth for the database destination and TLS identity.
            ssh.remote_host = config.host.clone();
            ssh.remote_port = config.port;
            Some(crate::ssh_tunnel::Tunnel::start(&ssh, &cancel)?)
        } else {
            None
        };
        let mut connection = client_config(config, tunnel.as_ref().map(|t| t.local_port()));
        let tls = if config.tls {
            connection.ssl_mode(SslMode::Require);
            Some(MakeTlsConnector::new(
                native_tls::TlsConnector::new().map_err(|e| e.to_string())?,
            ))
        } else {
            connection.ssl_mode(SslMode::Disable);
            None
        };
        let client = match &tls {
            Some(tls) => connection.connect(tls.clone()),
            None => connection.connect(NoTls),
        }
        .map_err(error)?;
        Ok(Self {
            client,
            config: config.clone(),
            cancel,
            tls,
            tunnel,
            schemas: SchemaCatalog::default(),
            browse_catalog: Vec::new(),
        })
    }

    fn refresh_schemas(&mut self) -> Result<(), String> {
        let rows = self.client.query_typed(
            "SELECT nspname, pg_catalog.current_schema()::text FROM pg_catalog.pg_namespace WHERE pg_catalog.has_schema_privilege(oid,'USAGE') AND nspname NOT LIKE 'pg_toast%' AND nspname NOT LIKE 'pg_temp_%' ORDER BY nspname",
            &[],
        ).map_err(error)?;
        let current: Option<String> = rows.first().and_then(|row| row.get(1));
        let names = rows.into_iter().map(|row| row.get::<_, String>(0)).collect::<Vec<_>>();
        if self.schemas.selected.as_ref().is_none_or(|selected| !names.contains(selected)) {
            self.schemas.selected = preferred_schema(&names, current.as_deref());
        }
        self.schemas.names = names;
        Ok(())
    }

    fn require_idle_transaction(&mut self) -> Result<(), String> {
        if self.client.transaction_status().is_none() {
            // Prepared-statement Close requests and split error responses may
            // still be queued. Sync drains them without changing an explicit
            // transaction, and gives us an authoritative ReadyForQuery state.
            self.client.check_connection().map_err(error)?;
        }
        match self.client.transaction_status() {
            Some(b'I') => Ok(()),
            _ => Err("Finish the SQL transaction with COMMIT or ROLLBACK before saving table changes, importing or exporting.".into()),
        }
    }

    fn check_tunnel(&mut self) -> Result<(), String> {
        if let Some(tunnel) = &mut self.tunnel {
            tunnel.check_alive()?;
        }
        Ok(())
    }

    fn watch(&self) -> CancelWatch {
        CancelWatch::start(
            self.client.cancel_token(),
            self.cancel.clone(),
            self.tls.clone(),
        )
    }

    fn selected_objects(&mut self) -> Result<Vec<Object>, String> {
        let Some(schema) = self.schemas.selected.clone() else {
            return Ok(Vec::new());
        };
        // Relation and attribute rows share one catalog snapshot and one typed
        // exchange. LEFT JOIN retains relations with no user columns. Stream
        // directly into the owned catalog instead of retaining a second row Vec.
        let mut rows = self.client.query_typed_raw(
            "SELECT c.relname,c.relkind::text,a.attname,pg_catalog.format_type(a.atttypid,a.atttypmod),a.attnotnull, \
             COALESCE(pg_catalog.pg_get_expr(d.adbin,d.adrelid),''), \
             EXISTS(SELECT 1 FROM pg_catalog.pg_index i WHERE i.indrelid=c.oid AND i.indisprimary AND a.attnum=ANY(i.indkey)), \
             (a.attgenerated <> '' OR a.attidentity <> '') \
             FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
             LEFT JOIN pg_catalog.pg_attribute a ON a.attrelid=c.oid AND a.attnum>0 AND NOT a.attisdropped \
             LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum \
             WHERE n.nspname=$1 AND c.relkind IN ('r','p','v','m','f') ORDER BY c.relname,a.attnum",
            [(&schema as &(dyn ToSql + Sync), Type::TEXT)],
        ).map_err(error)?;
        let mut objects: Vec<Object> = Vec::new();
        while let Some(row) = rows.next().map_err(error)? {
            if self.cancel.load(Ordering::Relaxed) {
                return Err("Cancelled".into());
            }
            let relation: &str = row.get(0);
            if objects.last().is_none_or(|object| object.display_name() != relation) {
                let kind: &str = row.get(1);
                objects.push(Object {
                    name: format!("{}.{}", quote(&schema), quote(relation)),
                    schema: Some(schema.clone()),
                    display_name: Some(relation.to_owned()),
                    kind: if matches!(kind, "v" | "m") { "view" } else { "table" }.into(),
                    columns: Vec::new(),
                    metadata_error: None,
                    ddl: String::new(),
                });
            }
            if let Some(name) = row.get::<_, Option<String>>(2) {
                objects.last_mut().unwrap().columns.push(Column {
                    name,
                    data_type: row.get(3),
                    not_null: row.get(4),
                    default: row.get(5),
                    pk: row.get(6),
                    generated: row.get(7),
                });
            }
        }
        Ok(objects)
    }

    #[cfg(test)]
    fn columns(&mut self, table: &str) -> Result<Vec<Column>, String> {
        self.client.query(
            "SELECT a.attname, pg_catalog.format_type(a.atttypid,a.atttypmod), a.attnotnull, \
             COALESCE(pg_catalog.pg_get_expr(d.adbin,d.adrelid),''), \
             EXISTS(SELECT 1 FROM pg_catalog.pg_index i WHERE i.indrelid=a.attrelid AND i.indisprimary AND a.attnum=ANY(i.indkey)), \
             (a.attgenerated <> '' OR a.attidentity <> '') \
             FROM pg_catalog.pg_attribute a LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum \
             WHERE a.attrelid=pg_catalog.to_regclass($1) AND a.attnum>0 AND NOT a.attisdropped ORDER BY a.attnum", &[&table]
        ).map_err(error).map(|rows| rows.into_iter().map(|r| Column {
            name:r.get(0), data_type:r.get(1), not_null:r.get(2), default:r.get(3), pk:r.get(4), generated:r.get(5)
        }).collect())
    }
}

impl Driver for PostgresDriver {
    fn ensure_connected(&mut self) -> Result<(), String> {
        let tunnel_lost = self.tunnel.as_mut().is_some_and(|tunnel| tunnel.check_alive().is_err());
        if !self.client.is_closed() && !tunnel_lost {
            return Ok(());
        }
        // Only a new user operation reconnects. Never replay an operation after
        // a transport error: the server may already have committed its writes.
        let mut replacement = Self::open(&self.config, self.cancel.clone())
            .map_err(|error| format!("Could not reconnect to PostgreSQL: {error}. Try again when the connection is available."))?;
        replacement.schemas = self.schemas.clone();
        *self = replacement;
        Ok(())
    }

    fn postgres_control(&self) -> Option<PostgresControl> {
        Some(PostgresControl { config: self.config.clone(), forwarded_port: self.tunnel.as_ref().map(|t| t.local_port()) })
    }

    fn definition(&mut self, name: &str) -> Result<String, String> {
        self.check_tunnel()?;
        let _watch = self.watch();
        let row = self.client.query_opt(include_str!("postgres/definition.sql"), &[&name]).map_err(error)?
            .ok_or("The relation no longer exists")?;
        let definition: Option<String> = row.get(0);
        definition.ok_or_else(|| "DDL preview is not available for inherited, partitioned, or foreign tables yet. Use pg_dump for their complete definition.".into())
    }

    fn databases(&mut self) -> Result<Vec<String>, String> {
        self.check_tunnel()?;
        if self.transaction_state().blocks_replacement() {
            return Err("Finish the current transaction before refreshing databases".into());
        }
        let _watch = self.watch();
        self.client.query("SELECT datname FROM pg_catalog.pg_database WHERE datallowconn AND NOT datistemplate AND pg_catalog.has_database_privilege(oid, 'CONNECT') ORDER BY datname", &[])
            .map(|rows| rows.into_iter().map(|row| row.get(0)).collect()).map_err(error)
    }

    fn transaction_state(&mut self) -> TransactionState {
        if self.client.is_closed() {
            return TransactionState::Disconnected;
        }
        if self.client.transaction_status().is_none() {
            // An error can arrive before ReadyForQuery. Drain the protocol before
            // publishing state so a failed transaction is not mistaken for idle.
            if self.client.check_connection().is_err() {
                return TransactionState::Disconnected;
            }
        }
        match self.client.transaction_status() {
            Some(b'T') => TransactionState::Active,
            Some(b'E') => TransactionState::Failed,
            _ => TransactionState::Idle,
        }
    }
    fn import_csv(&mut self, request: &csv_import::Request, cancel: &AtomicBool) -> Result<u64, String> {
        import::import_csv(self, request, cancel)
    }
    fn engine_name(&self) -> &'static str {
        "PostgreSQL"
    }
    fn schema_catalog(&self) -> SchemaCatalog {
        self.schemas.clone()
    }

    fn select_schema(&mut self, name: &str) -> Result<Vec<Object>, String> {
        self.check_tunnel()?;
        let _watch = self.watch();
        self.refresh_schemas()?;
        if !self.schemas.names.iter().any(|schema| schema == name) {
            return Err("Schema is unavailable or you no longer have access".into());
        }
        let previous = self.schemas.selected.replace(name.to_owned());
        match self.selected_objects() {
            Ok(objects) => Ok(objects),
            Err(error) => {
                self.schemas.selected = previous;
                Err(error)
            }
        }
    }

    fn objects(&mut self) -> Result<Vec<Object>, String> {
        self.check_tunnel()?;
        let _watch = self.watch();
        self.refresh_schemas()?;
        self.selected_objects()
    }

    fn execute(&mut self, sql: &str, safe: bool, cancel: &AtomicBool) -> ScriptResult {
        let started = Instant::now();
        let _watch = self.watch();
        let mut report = ScriptResult::default();
        if let Err(error) = self.check_tunnel() {
            report.error = Some(error);
            return report;
        }
        let statements = match split_statements(sql) {
            Ok(s) => s,
            Err(e) => {
                report.error = Some(e);
                return report;
            }
        };
        if statements.len() > 128 {
            report.error=Some("A run supports at most 128 statements. Select a smaller batch; nothing was executed.".into());
            return report;
        }
        let mut budget = BYTE_LIMIT;
        for (index, statement) in statements.into_iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                report.error = Some(format!(
                    "Cancelled before statement {}. Autocommitted statements remain committed; explicit transactions are unchanged.",
                    index + 1
                ));
                break;
            }
            let result: Result<Grid, String> = (|| {
                check_statement(statement, safe)?;
                let timer = Instant::now();
                // These SET LOCAL commands share the query's implicit
                // transaction and Sync boundary. No explicit BEGIN/COMMIT is
                // needed: PostgreSQL commits normal editor statements itself.
                // Recovery commands must reach an aborted user transaction
                // without an earlier SET being rejected and skipping recovery.
                let recovery = matches!(
                    first_word(statement).as_deref(),
                    Some("COMMIT" | "END" | "ROLLBACK" | "ABORT")
                );
                let prefix: &[&str] = if recovery {
                    &[]
                } else if safe {
                    &[
                        "SET LOCAL standard_conforming_strings = on",
                        "SET LOCAL transaction_read_only = on",
                    ]
                } else {
                    &["SET LOCAL standard_conforming_strings = on"]
                };
                let rows = self
                    .client
                    .query_typed_raw_with_prefix(
                        statement,
                        std::iter::empty::<(&(dyn ToSql + Sync), Type)>(),
                        prefix,
                    )
                    .map_err(|err| {
                        if let Some(::postgres::error::ErrorPosition::Original(position)) = err.as_db_error().and_then(|error| error.position()) {
                            let local = statement.char_indices().nth(position.saturating_sub(1) as usize)
                                .map_or(statement.len(), |(byte, _)| byte);
                            report.error_offset = Some(statement.as_ptr() as usize - sql.as_ptr() as usize + local);
                        }
                        error(err)
                    })?;
                let columns = rows
                    .columns()
                    .iter()
                    .map(|column| column.name().to_owned())
                    .collect();
                let mut grid = collect(rows, columns, QUERY_LIMIT, budget, cancel)?;
                grid.elapsed = timer.elapsed();
                Ok(grid)
            })();
            match result {
                Ok(grid) => {
                    budget = budget
                        .saturating_sub(grid.rows.iter().flatten().map(value_size).sum::<usize>());
                    report.grids.push(Arc::new(grid));
                }
                Err(e) => {
                    let guidance = if self.transaction_state() == TransactionState::Disconnected {
                        "Connection lost; this statement's outcome is unknown. Run a new query or reload to reconnect. Verify saved data before retrying writes. Remaining statements were not run."
                    } else {
                        "Autocommitted statements remain committed; an explicit transaction may need ROLLBACK."
                    };
                    report.error = Some(format!(
                        "Statement {} failed: {e}. {guidance}",
                        index + 1
                    ));
                    break;
                }
            }
        }
        // Errors or early stream termination can leave ReadyForQuery pending.
        // Fully consumed successful queries already include statement cleanup.
        if self.client.transaction_status().is_none()
            && let Err(failure) = self.client.check_connection()
            && report.error.is_none()
        {
            report.error = Some(format!("Connection lost while finishing query: {}. Verify the outcome before retrying writes.", error(failure)));
        }
        report.elapsed = started.elapsed();
        report
    }

    fn browse(&mut self, request: &Browse) -> Result<TableData, String> {
        browse::browse(self, request)
    }

    fn plan(&mut self, changes: &[Change]) -> Result<ChangePlan, String> {
        changes::plan(self, changes)
    }
    fn apply(&mut self, plan: &ChangePlan) -> Result<(), String> {
        changes::apply(self, plan)
    }
    fn export_table(
        &mut self,
        path: &std::path::Path,
        browse: &Browse,
        cancel: &AtomicBool,
        format: ExportFormat,
    ) -> Result<(), String> {
        export::export_table(self, path, browse, cancel, format)
    }
}

fn preferred_schema(names: &[String], current: Option<&str>) -> Option<String> {
    names
        .iter()
        .find(|name| name.as_str() == "public")
        .or_else(|| names.iter().find(|name| Some(name.as_str()) == current))
        .or_else(|| {
            names
                .iter()
                .find(|name| !matches!(name.as_str(), "pg_catalog" | "information_schema"))
        })
        .or_else(|| names.first())
        .cloned()
}

pub(crate) fn client_config(
    config: &crate::connection::PostgresConfig,
    forwarded_port: Option<u16>,
) -> ::postgres::Config {
    async_client_config(config, forwarded_port).into()
}

pub(crate) fn async_client_config(
    config: &crate::connection::PostgresConfig,
    forwarded_port: Option<u16>,
) -> tokio_postgres::Config {
    let mut connection = tokio_postgres::Config::new();
    connection
        .host(&config.host)
        .port(forwarded_port.unwrap_or(config.port))
        .dbname(&config.database)
        .user(&config.user)
        .password(&config.password)
        .application_name("Rustable")
        .options("-c statement_timeout=0 -c lock_timeout=0")
        .keepalives_idle(Duration::from_secs(15))
        .keepalives_interval(Duration::from_secs(5))
        .keepalives_retries(3)
        .connect_timeout(Duration::from_secs(10));
    if forwarded_port.is_some() {
        // hostaddr selects the loopback transport; host remains the remote
        // database hostname used for TLS certificate and hostname verification.
        connection.hostaddr(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
    }
    connection
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
fn error(error: ::postgres::Error) -> String {
    if let Some(db) = error.as_db_error() {
        let mut message = format!(
            "{}: {} (SQLSTATE {})",
            db.severity(),
            db.message(),
            db.code().code()
        );
        if let Some(detail) = db.detail() {
            message.push_str(&format!("\n{detail}"));
        }
        if let Some(hint) = db.hint() {
            message.push_str(&format!("\nHint: {hint}"));
        }
        message
    } else {
        error.to_string()
    }
}

struct CancelWatch {
    done: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
const CANCEL_REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
impl CancelWatch {
    fn start(
        token: ::postgres::CancelToken,
        cancel: Arc<AtomicBool>,
        tls: Option<MakeTlsConnector>,
    ) -> Self {
        let done = Arc::new(AtomicBool::new(false));
        let stopped = done.clone();
        let progress = super::progress::current();
        let thread = std::thread::spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                if cancel.load(Ordering::Relaxed) {
                    let result = match &tls {
                        Some(tls) => token.cancel_query_with_timeout(tls.clone(), CANCEL_REQUEST_TIMEOUT),
                        None => token.cancel_query_with_timeout(NoTls, CANCEL_REQUEST_TIMEOUT),
                    };
                    match result {
                        Ok(true) => break,
                        result => {
                            let error = match result { Err(error) => error.to_string(), _ => "request timed out".into() };
                            if let Some(progress) = &progress {
                                progress.notify(format!("Could not send cancellation: {error}. The query may still be running. Retry cancellation or inspect running queries."));
                            }
                            cancel.store(false, Ordering::Relaxed);
                        }
                    }
                }
                std::thread::park_timeout(Duration::from_millis(20));
            }
        });
        Self {
            done,
            thread: Some(thread),
        }
    }
}
impl Drop for CancelWatch {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            // Wake immediately on completion; cancellation polling must not add
            // a full polling interval to every successful query.
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

fn collect(
    mut rows: RowIter<'_>,
    columns: Vec<String>,
    limit: usize,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Grid, String> {
    let mut grid = Grid {
        columns,
        ..Grid::default()
    };
    let mut bytes = 0usize;
    while let Some(row) = rows.next().map_err(error)? {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        if grid.rows.len() >= limit || grid.truncated {
            grid.truncated = true;
            continue;
        }
        let Some(values) = decode_row_bounded(&row, budget.saturating_sub(bytes))? else {
            grid.truncated = true;
            continue;
        };
        bytes += values.iter().map(value_size).sum::<usize>();
        super::progress::decoded_row(&values);
        grid.rows.push(values);
    }
    grid.affected = rows.rows_affected().unwrap_or(0) as usize;
    Ok(grid)
}
fn value_size(value: &Value) -> usize {
    std::mem::size_of::<Value>()
        + match value {
            Value::Text(s) => s.len(),
            Value::Blob(b) => b.len(),
            _ => 0,
        }
}
#[cfg(test)]
fn decode_row(row: &Row) -> Result<Vec<Value>, String> {
    (0..row.len())
        .map(|i| {
            row.try_get::<_, Option<PgValue>>(i)
                .map(|v| v.map_or(Value::Null, |v| v.0))
                .map_err(error)
        })
        .collect()
}

// Borrow protocol-owned bytes without cloning them merely to inspect their size.
struct BorrowedValue<'a>(&'a [u8]);
impl<'a> FromSql<'a> for BorrowedValue<'a> {
    fn from_sql(_: &Type, raw: &'a [u8]) -> Result<Self, Box<dyn Error + Sync + Send>> {
        Ok(Self(raw))
    }
    fn accepts(_: &Type) -> bool {
        true
    }
}
fn decode_row_bounded(row: &Row, budget: usize) -> Result<Option<Vec<Value>>, String> {
    let Some(mut remaining) =
        budget.checked_sub(row.len().saturating_mul(std::mem::size_of::<Value>()))
    else {
        return Ok(None);
    };
    let mut values = Vec::with_capacity(row.len());
    for (index, column) in row.columns().iter().enumerate() {
        let raw = row
            .try_get::<_, Option<BorrowedValue<'_>>>(index)
            .map_err(error)?;
        let value = if let Some(raw) = raw {
            let Some(value) = PgValue::decode_bounded(column.type_(), raw.0, remaining)
                .map_err(|error| error.to_string())?
            else {
                return Ok(None);
            };
            remaining -= value_size(&value) - std::mem::size_of::<Value>();
            value
        } else {
            Value::Null
        };
        values.push(value);
    }
    Ok(Some(values))
}

#[derive(Debug)]
struct PgValue(Value);
fn native_display_type(ty: &Type) -> bool {
    use ::postgres::types::Kind;
    match ty.kind() {
        Kind::Domain(base) => native_display_type(base),
        Kind::Enum(_) => true,
        _ => matches!(
            *ty,
            Type::BOOL
                | Type::INT2
                | Type::INT4
                | Type::INT8
                | Type::OID
                | Type::FLOAT4
                | Type::FLOAT8
                | Type::TEXT
                | Type::VARCHAR
                | Type::BPCHAR
                | Type::NAME
                | Type::UNKNOWN
                | Type::JSON
                | Type::XML
                | Type::JSONB
                | Type::NUMERIC
                | Type::DATE
                | Type::TIMESTAMP
                | Type::TIMESTAMPTZ
                | Type::TIME
                | Type::INTERVAL
                | Type::UUID
                | Type::BYTEA
        ),
    }
}

#[cfg(test)]
#[test]
fn native_display_decodes_only_known_wire_formats() {
    use ::postgres::types::Kind;
    let enumeration = Type::new(
        "mood".into(),
        50000,
        Kind::Enum(vec!["happy".into()]),
        "public".into(),
    );
    let domain = Type::new(
        "positive".into(),
        50001,
        Kind::Domain(Type::INT4),
        "public".into(),
    );
    assert!(native_display_type(&enumeration));
    assert!(native_display_type(&domain));
    assert_eq!(
        PgValue::from_sql(&enumeration, b"happy").unwrap().0,
        Value::Text("happy".into())
    );
    assert_eq!(
        PgValue::from_sql(&domain, &42_i32.to_be_bytes()).unwrap().0,
        Value::Integer(42)
    );
    assert!(!native_display_type(&Type::TEXT_ARRAY));
    assert!(native_display_type(&Type::INTERVAL));
    assert_eq!(
        PgValue::from_sql(&Type::POINT, b"printable but not text")
            .unwrap()
            .0,
        Value::Blob(b"printable but not text".to_vec())
    );
}
impl PgValue {
    fn decode_bounded(
        ty: &Type,
        raw: &[u8],
        budget: usize,
    ) -> Result<Option<Value>, Box<dyn Error + Sync + Send>> {
        use ::postgres::types::Kind;
        match ty.kind() {
            Kind::Domain(base) => return Self::decode_bounded(base, raw, budget),
            Kind::Array(element) if native_display_type(element) => {
                return Ok(array::decode_bounded(element, raw, budget)?.map(Value::Text));
            }
            _ => {}
        }
        if *ty == Type::NUMERIC {
            return Ok(numeric_bounded(raw, budget)?.map(Value::Text));
        }
        // Scalar binary decoders allocate at most a small fixed representation.
        // All other encodings copy raw payload, except the JSONB version byte.
        let scalar = matches!(
            *ty,
            Type::BOOL
                | Type::INT2
                | Type::INT4
                | Type::INT8
                | Type::OID
                | Type::FLOAT4
                | Type::FLOAT8
                | Type::DATE
                | Type::TIMESTAMP
                | Type::TIMESTAMPTZ
                | Type::TIME
                | Type::INTERVAL
                | Type::UUID
        );
        let payload = raw
            .len()
            .saturating_sub(usize::from(*ty == Type::JSONB && raw.first() == Some(&1)));
        if (!scalar || raw.len() > 16) && payload > budget {
            return Ok(None);
        }
        let value = Self::from_sql(ty, raw)?.0;
        Ok((value_size(&value) - std::mem::size_of::<Value>() <= budget).then_some(value))
    }
}
impl<'a> FromSql<'a> for PgValue {
    fn from_sql(ty: &Type, raw: &'a [u8]) -> Result<Self, Box<dyn Error + Sync + Send>> {
        // These wire encodings are explicitly defined by PostgreSQL; unknown
        // binary formats must never be guessed to contain UTF-8.
        match ty.kind() {
            ::postgres::types::Kind::Array(element) if native_display_type(element) => {
                return Ok(Self(Value::Text(array::decode(element, raw)?)));
            }
            ::postgres::types::Kind::Domain(base) => return Self::from_sql(base, raw),
            ::postgres::types::Kind::Enum(_) => {
                return Ok(Self(Value::Text(std::str::from_utf8(raw)?.into())));
            }
            _ => {}
        }
        let value = match *ty {
            Type::BOOL => Value::Text(
                if bool::from_sql(ty, raw)? {
                    "true"
                } else {
                    "false"
                }
                .into(),
            ),
            Type::INT2 => Value::Integer(i16::from_sql(ty, raw)? as i64),
            Type::INT4 => Value::Integer(i32::from_sql(ty, raw)? as i64),
            Type::INT8 => Value::Integer(i64::from_sql(ty, raw)?),
            Type::OID => Value::Integer(u32::from_sql(ty, raw)? as i64),
            Type::FLOAT4 => Value::Real(f32::from_sql(ty, raw)? as f64),
            Type::FLOAT8 => Value::Real(f64::from_sql(ty, raw)?),
            Type::TEXT
            | Type::VARCHAR
            | Type::BPCHAR
            | Type::NAME
            | Type::UNKNOWN
            | Type::JSON
            | Type::XML => Value::Text(std::str::from_utf8(raw)?.into()),
            Type::JSONB if raw.first() == Some(&1) => {
                Value::Text(std::str::from_utf8(&raw[1..])?.into())
            }
            Type::NUMERIC => Value::Text(numeric(raw)?),
            Type::INTERVAL => Value::Text(interval::decode(raw)?),
            Type::DATE if raw.len() == 4 => {
                let days = i32::from_be_bytes(raw.try_into()?);
                Value::Text(match days {
                    i32::MAX => "infinity".into(),
                    i32::MIN => "-infinity".into(),
                    _ => date(days as i64),
                })
            }
            Type::TIMESTAMP | Type::TIMESTAMPTZ if raw.len() == 8 => {
                let micros = i64::from_be_bytes(raw.try_into()?);
                Value::Text(match micros {
                    i64::MAX => "infinity".into(),
                    i64::MIN => "-infinity".into(),
                    _ => format!(
                        "{} {}{}",
                        date(micros.div_euclid(86_400_000_000)),
                        time(micros.rem_euclid(86_400_000_000)),
                        if *ty == Type::TIMESTAMPTZ { "+00" } else { "" }
                    ),
                })
            }
            Type::TIME if raw.len() == 8 => Value::Text(time(i64::from_be_bytes(raw.try_into()?))),

            Type::UUID if raw.len() == 16 => Value::Text(format!(
                "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
                raw[0],
                raw[1],
                raw[2],
                raw[3],
                raw[4],
                raw[5],
                raw[6],
                raw[7],
                raw[8],
                raw[9],
                raw[10],
                raw[11],
                raw[12],
                raw[13],
                raw[14],
                raw[15]
            )),
            _ => Value::Blob(raw.to_vec()),
        };
        Ok(Self(value))
    }
    fn accepts(_: &Type) -> bool {
        true
    }
}
// Gregorian calendar conversion for PostgreSQL's days since 2000-01-01.
fn date(days: i64) -> String {
    let z = days + 10_957 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    if year <= 0 {
        format!("{:04}-{month:02}-{day:02} BC", 1 - year)
    } else {
        format!("{year:04}-{month:02}-{day:02}")
    }
}
fn time(micros: i64) -> String {
    let seconds = micros / 1_000_000;
    let fraction = micros % 1_000_000;
    let mut text = format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        (seconds / 60) % 60,
        seconds % 60
    );
    if fraction != 0 {
        text.push_str(format!(".{fraction:06}").trim_end_matches('0'));
    }
    text
}

fn numeric(raw: &[u8]) -> Result<String, Box<dyn Error + Sync + Send>> {
    Ok(numeric_bounded(raw, usize::MAX)?.expect("unbounded numeric"))
}
fn numeric_bounded(
    raw: &[u8],
    budget: usize,
) -> Result<Option<String>, Box<dyn Error + Sync + Send>> {
    if raw.len() < 8 || !raw.len().is_multiple_of(2) {
        return Err("Invalid PostgreSQL numeric".into());
    }
    let count = u16::from_be_bytes([raw[0], raw[1]]) as usize;
    let weight = i16::from_be_bytes([raw[2], raw[3]]) as i32;
    let sign = u16::from_be_bytes([raw[4], raw[5]]);
    let scale = u16::from_be_bytes([raw[6], raw[7]]) as usize;
    match sign {
        0xc000 => return Ok((budget >= 3).then(|| "NaN".into())),
        0xd000 => return Ok((budget >= 8).then(|| "Infinity".into())),
        0xf000 => return Ok((budget >= 9).then(|| "-Infinity".into())),
        0 | 0x4000 => (),
        _ => return Err("Invalid numeric sign".into()),
    }
    if raw.len() != 8 + count * 2 {
        return Err("Invalid numeric digits".into());
    }
    let digits = raw[8..].as_chunks::<2>().0;
    if digits.iter().any(|d| u16::from_be_bytes(*d) >= 10000) {
        return Err("Invalid numeric digit".into());
    }
    let group = |power: i32| -> u16 {
        let i = weight - power;
        if i < 0 {
            0
        } else {
            digits
                .get(i as usize)
                .map(|d| u16::from_be_bytes(*d))
                .unwrap_or(0)
        }
    };
    let integer_length = if weight < 0 {
        1
    } else {
        let first = group(weight);
        usize::from(first >= 10)
            + usize::from(first >= 100)
            + usize::from(first >= 1000)
            + 1
            + weight as usize * 4
    };
    let length = usize::from(sign == 0x4000) + integer_length + usize::from(scale > 0) + scale;
    if length > budget {
        return Ok(None);
    }
    let mut out = String::with_capacity(length);
    if sign == 0x4000 {
        out.push('-');
    }
    if weight < 0 {
        out.push('0');
    } else {
        out.push_str(&group(weight).to_string());
        for p in (0..weight).rev() {
            out.push_str(&format!("{:04}", group(p)));
        }
    }
    if scale > 0 {
        out.push('.');
        let start = out.len();
        for n in 1..=scale.div_ceil(4) {
            let chunk = format!("{:04}", group(-(n as i32)));
            out.push_str(&chunk[..(start + scale - out.len()).min(4)]);
        }
        out.truncate(start + scale);
    }
    Ok(Some(out))
}

fn split_statements(sql: &str) -> Result<Vec<&str>, String> {
    crate::sql::postgres_statements(sql)
        .map(|ranges| ranges.into_iter().map(|range| &sql[range]).collect())
}
fn comment_end(bytes: &[u8], mut i: usize) -> Result<usize, String> {
    let mut depth = 1;
    i += 2;
    while i < bytes.len() {
        if bytes.get(i..i + 2) == Some(b"/*") {
            depth += 1;
            i += 2;
        } else if bytes.get(i..i + 2) == Some(b"*/") {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return Ok(i);
            }
        } else {
            i += 1;
        }
    }
    Err("Unterminated SQL comment".into())
}
fn first_word(mut s: &str) -> Option<String> {
    loop {
        s = s.trim_start();
        if let Some(rest) = s.strip_prefix('(') {
            s = rest;
            continue;
        }
        if s.starts_with("--") {
            s = s.split_once('\n')?.1;
        } else if s.starts_with("/*") {
            s = &s[comment_end(s.as_bytes(), 0).ok()?..];
        } else {
            let end = s.bytes().take_while(|c| c.is_ascii_alphabetic()).count();
            return (end > 0).then(|| s[..end].to_ascii_uppercase());
        }
    }
}
fn check_statement(sql: &str, safe: bool) -> Result<(), String> {
    let word = first_word(sql).ok_or("Expected an SQL statement")?;
    let recovery = matches!(word.as_str(), "COMMIT" | "END" | "ROLLBACK" | "ABORT")
        && sqlparser::parser::Parser::parse_sql(&sqlparser::dialect::PostgreSqlDialect {}, sql)
            .is_ok_and(|statements| matches!(statements.as_slice(),
                [sqlparser::ast::Statement::Commit { modifier: None, .. }]
                | [sqlparser::ast::Statement::Rollback { .. }]));
    if safe && !recovery
        && !matches!(
            word.as_str(),
            "SELECT" | "WITH" | "VALUES" | "TABLE" | "SHOW" | "EXPLAIN"
        )
    {
        return Err(
            "Safe mode only allows read queries, enforced by PostgreSQL transaction_read_only"
                .into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires disposable PostgreSQL on port 54329"]
    fn ddl_preview_reads_live_catalog() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let mut driver = PostgresDriver::open(&crate::connection::PostgresConfig {
            host: "127.0.0.1".into(), port: 54329, database: "rustable_qa".into(),
            user: "postgres".into(), password: "rustable_qa_only".into(), tls: false, ssh: None,
        }, Arc::new(AtomicBool::new(false))).unwrap();
        driver.client.batch_execute("BEGIN; CREATE SCHEMA rustable_ddl_qa; CREATE TABLE rustable_ddl_qa.example(id integer GENERATED ALWAYS AS IDENTITY PRIMARY KEY, label text NOT NULL DEFAULT 'hello', calculated integer GENERATED ALWAYS AS (id+1) STORED, CONSTRAINT positive CHECK (id>0)); CREATE INDEX label_idx ON rustable_ddl_qa.example(label); CREATE VIEW rustable_ddl_qa.visible AS SELECT label FROM rustable_ddl_qa.example").unwrap();
        let ddl = driver.definition("rustable_ddl_qa.example").unwrap();
        for expected in ["CREATE TABLE rustable_ddl_qa.example", "GENERATED ALWAYS AS IDENTITY", "NOT NULL", "DEFAULT 'hello'", "GENERATED ALWAYS AS", "STORED", "PRIMARY KEY", "CONSTRAINT positive CHECK", "CREATE INDEX label_idx"] {
            assert!(ddl.contains(expected), "Missing {expected}: {ddl}");
        }
        let view = driver.definition("rustable_ddl_qa.visible").unwrap();
        assert!(view.contains("CREATE VIEW rustable_ddl_qa.visible AS"));
        assert!(view.ends_with(';'));
        driver.client.batch_execute("ALTER TABLE rustable_ddl_qa.example ADD COLUMN fresh boolean").unwrap();
        assert!(driver.definition("rustable_ddl_qa.example").unwrap().contains("fresh boolean"));
        assert!(driver.definition("rustable_ddl_qa.missing").is_err());
        // The preview should itself parse as DDL for a regular table.
        driver.client.batch_execute("DROP SCHEMA rustable_ddl_qa CASCADE; CREATE SCHEMA rustable_ddl_qa").unwrap();
        driver.client.batch_execute(&ddl).unwrap();
        driver.client.batch_execute("ROLLBACK").unwrap();
    }

    #[test]
    fn postgres_boundaries_preserve_dollar_quotes_and_nested_comments() {
        let sql = "SELECT $$a;b$$; /* outer /* inner ; */ still */ SELECT E'a\\\';b'; SELECT $tag$日本;$tag$;";
        let s = split_statements(sql).unwrap();
        assert_eq!(s.len(), 3);
        assert!(s[1].contains("inner"));
        assert!(split_statements("SELECT $tag$broken").is_err());
        assert!(split_statements("SELECT 'broken").is_err());
        assert!(split_statements("/* broken").is_err());
    }
    #[test]
    fn safe_mode_allows_session_recovery_but_not_transaction_configuration() {
        for sql in [
            "BEGIN",
            "-- comment\nSET transaction_read_only=off",
            "PREPARE TRANSACTION 'x'",
            "COMMIT PREPARED 'x'",
            "ROLLBACK PREPARED 'x'",
            "COMMIT; DELETE FROM x",
        ] {
            assert!(check_statement(sql, true).is_err());
            assert!(check_statement(sql, false).is_ok());
        }
        for sql in ["/* recovery */ COMMIT", "END", "ROLLBACK", "ABORT", "ROLLBACK TO SAVEPOINT x", "COMMIT AND CHAIN"] {
            assert!(check_statement(sql, true).is_ok(), "{sql}");
        }
        assert!(check_statement("SELECT 1", true).is_ok());
        assert!(check_statement("(/* comment */ (SELECT 1))", true).is_ok());
        assert!(check_statement("DELETE FROM x", true).is_err());
        assert!(check_statement("DELETE FROM x", false).is_ok());
    }
    #[test]
    fn numeric_preserves_decimal_precision() {
        assert_eq!(
            numeric(&[0, 2, 0, 0, 0, 0, 0, 4, 0, 12, 13, 128]).unwrap(),
            "12.3456"
        );
        assert_eq!(
            numeric(&[0, 1, 255, 255, 64, 0, 0, 5, 0, 12]).unwrap(),
            "-0.00120"
        );
        assert_eq!(
            numeric(&[0, 1, 0, 2, 0, 0, 0, 0, 0, 1]).unwrap(),
            "100000000"
        );
        assert_eq!(numeric(&[0, 0, 0, 0, 192, 0, 0, 0]).unwrap(), "NaN");
    }
    #[test]
    fn temporal_values_keep_microseconds_and_calendar_boundaries() {
        assert_eq!(date(0), "2000-01-01");
        assert_eq!(date(-1), "1999-12-31");
        assert_eq!(date(59), "2000-02-29");
        assert_eq!(date(60), "2000-03-01");
        assert_eq!(date(-10957), "1970-01-01");
        assert_eq!(time(3_661_123_400), "01:01:01.1234");
        assert_eq!(time(86_400_000_000), "24:00:00");
    }
    #[test]
    fn ssh_transport_keeps_remote_tls_identity() {
        let config = crate::connection::PostgresConfig {
            host: "db.internal.example".into(),
            port: 5432,
            ..Default::default()
        };
        let forwarded = client_config(&config, Some(12345));
        assert_eq!(
            forwarded.get_hosts(),
            &[::postgres::config::Host::Tcp("db.internal.example".into())]
        );
        assert_eq!(
            forwarded.get_hostaddrs(),
            &[std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)]
        );
        assert_eq!(forwarded.get_ports(), &[12345]);
        let direct = client_config(&config, None);
        assert!(direct.get_hostaddrs().is_empty());
        assert_eq!(direct.get_ports(), &[5432]);
    }
    #[test]
    fn identifiers_are_quoted_as_single_components() {
        assert_eq!(quote("a.b\"c"), "\"a.b\"\"c\"");
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    #[test]
    #[ignore = "requires opted-in disposable PostgreSQL QA server on port 54329"]
    fn editor_autocommit_and_explicit_transactions_follow_postgres() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let cancel = Arc::new(AtomicBool::new(false));
        let mut driver = PostgresDriver::open(
            &crate::connection::PostgresConfig {
                host: "127.0.0.1".into(),
                port: 54329,
                database: "rustable_qa".into(),
                user: "postgres".into(),
                password: "rustable_qa_only".into(),
                tls: false,
                ssh: None,
            },
            cancel.clone(),
        )
        .unwrap();
        let setup = driver.execute(
            "CREATE TEMP TABLE autocommit_qa(id integer)",
            false,
            &cancel,
        );
        assert!(setup.error.is_none(), "{:?}", setup.error);
        let ids = driver.execute(
            "SELECT txid_current(); SELECT txid_current()",
            false,
            &cancel,
        );
        assert!(ids.error.is_none(), "{:?}", ids.error);
        assert_ne!(ids.grids[0].rows[0], ids.grids[1].rows[0]);
        let explicit = driver.execute("BEGIN; SELECT txid_current(); SELECT txid_current(); INSERT INTO autocommit_qa VALUES (1)", false, &cancel);
        assert!(explicit.error.is_none(), "{:?}", explicit.error);
        assert_eq!(explicit.grids[1].rows[0], explicit.grids[2].rows[0]);
        assert_eq!(driver.client.transaction_status(), Some(b'T'));
        assert!(
            changes::apply(&mut driver, &ChangePlan::default())
                .unwrap_err()
                .contains("Finish the SQL transaction")
        );
        let output = tempfile::tempdir().unwrap();
        let destination = output.path().join("refused.csv");
        assert!(
            export::export_table(
                &mut driver,
                &destination,
                &Browse {
                    table: "autocommit_qa".into(),
                    ..Default::default()
                },
                &cancel, ExportFormat::Csv)
            .unwrap_err()
            .contains("Finish the SQL transaction")
        );
        assert!(!destination.exists());
        assert_eq!(driver.client.transaction_status(), Some(b'T'));
        let failed = driver.execute("SELECT 1/0", false, &cancel);
        assert!(failed.error.is_some());
        let still_aborted = driver.execute("SELECT 1", false, &cancel);
        assert!(
            still_aborted
                .error
                .as_deref()
                .is_some_and(|e| e.contains("25P02")),
            "{:?}",
            still_aborted.error
        );
        // A split ErrorResponse/ReadyForQuery packet may still be pending;
        // both unknown and the server's E status must conservatively refuse.
        assert_ne!(driver.client.transaction_status(), Some(b'I'));
        assert!(changes::apply(&mut driver, &ChangePlan::default()).is_err());
        let rolled_back = driver.execute(
            "ROLLBACK; SELECT count(*) FROM autocommit_qa",
            false,
            &cancel,
        );
        assert!(rolled_back.error.is_none(), "{:?}", rolled_back.error);
        assert_eq!(rolled_back.grids[1].rows[0], [Value::Integer(0)]);
        assert_eq!(driver.client.transaction_status(), Some(b'I'));
        let committed = driver.execute("BEGIN; INSERT INTO autocommit_qa VALUES (2); SAVEPOINT a; INSERT INTO autocommit_qa VALUES (3); ROLLBACK TO a; COMMIT; SELECT count(*) FROM autocommit_qa", false, &cancel);
        assert!(committed.error.is_none(), "{:?}", committed.error);
        assert_eq!(committed.grids[6].rows[0], [Value::Integer(1)]);
        assert_eq!(driver.client.transaction_status(), Some(b'I'));
        let autocommit = driver.execute(
            "INSERT INTO autocommit_qa VALUES (4); SELECT 1/0",
            false,
            &cancel,
        );
        assert!(autocommit.error.is_some());
        let count = driver.execute("SELECT count(*) FROM autocommit_qa", false, &cancel);
        assert_eq!(count.grids[0].rows[0], [Value::Integer(2)]);
        let settings = driver.execute("SET default_transaction_read_only = off", false, &cancel);
        assert!(settings.error.is_none(), "{:?}", settings.error);
        let safe = driver.execute(
            "SELECT current_setting('transaction_read_only')",
            true,
            &cancel,
        );
        assert!(safe.error.is_none(), "{:?}", safe.error);
        assert_eq!(safe.grids[0].rows[0], [Value::Text("on".into())]);
    }

    #[test]
    #[ignore = "requires opted-in disposable PostgreSQL QA server on port 54329"]
    fn implicit_safe_prefix_probe() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let mut client = Client::connect(
            "host=127.0.0.1 port=54329 user=postgres password=rustable_qa_only dbname=rustable_qa",
            NoTls,
        )
        .unwrap();
        let mut rows = client.query_typed_raw_with_prefix(
            "SELECT current_setting('transaction_read_only'), current_setting('standard_conforming_strings')",
            std::iter::empty::<(&(dyn ToSql + Sync), Type)>(),
            &["SET LOCAL standard_conforming_strings = on", "SET LOCAL transaction_read_only = on"],
        ).unwrap();
        let row = rows.next().unwrap().unwrap();
        assert_eq!(row.get::<_, String>(0), "on");
        assert_eq!(row.get::<_, String>(1), "on");
        assert!(rows.next().unwrap().is_none());
        drop(rows);
        assert_eq!(
            client
                .query_one("SHOW transaction_read_only", &[])
                .unwrap()
                .get::<_, String>(0),
            "off"
        );
        let denied = client.query_typed_raw_with_prefix(
            "SELECT set_config('transaction_read_only','off',true)",
            std::iter::empty::<(&(dyn ToSql + Sync), Type)>(),
            &["SET LOCAL transaction_read_only = on"],
        );
        if let Ok(mut rows) = denied {
            assert!(rows.next().is_err(), "safe flag can be disabled");
        }
    }

    #[test]
    #[ignore = "requires opted-in disposable PostgreSQL QA server on port 54329"]
    fn pipelined_setup_failure_skips_query_and_cancellation_rolls_back() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let cancel = Arc::new(AtomicBool::new(false));
        let mut driver = PostgresDriver::open(
            &crate::connection::PostgresConfig {
                host: "127.0.0.1".into(),
                port: 54329,
                database: "rustable_qa".into(),
                user: "postgres".into(),
                password: "rustable_qa_only".into(),
                tls: false,
                ssh: None,
            },
            cancel.clone(),
        )
        .unwrap();
        driver
            .client
            .batch_execute("CREATE TEMP TABLE pipeline_qa(id integer)")
            .unwrap();
        for prefix in [
            ["INVALID SET", "SET LOCAL standard_conforming_strings = on"],
            [
                "SET LOCAL standard_conforming_strings = on",
                "SET LOCAL rustable_invalid_setting = on",
            ],
        ] {
            assert!(
                driver
                    .client
                    .query_typed_raw_with_prefix(
                        "INSERT INTO pipeline_qa VALUES (1) RETURNING id",
                        std::iter::empty::<(&(dyn ToSql + Sync), Type)>(),
                        &prefix,
                    )
                    .is_err()
            );
            let count: i64 = driver
                .client
                .query_one("SELECT count(*) FROM pipeline_qa", &[])
                .unwrap()
                .get(0);
            assert_eq!(count, 0, "query executed despite failed prefix");
        }
        let mut rows = driver.client.query_typed_raw_with_prefix(
            "SELECT count(*) FROM pipeline_qa",
            std::iter::empty::<(&(dyn ToSql + Sync), Type)>(),
            &["INSERT INTO pipeline_qa VALUES (9)"],
        ).unwrap();
        assert_eq!(rows.next().unwrap().unwrap().get::<_, i64>(0), 1, "prefix must execute exactly once");
        assert!(rows.next().unwrap().is_none());
        drop(rows);
        driver.client.batch_execute("DELETE FROM pipeline_qa").unwrap();
        let request_cancel = cancel.clone();
        let canceller = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            request_cancel.store(true, Ordering::Relaxed);
        });
        let cancelled = driver.execute(
            "WITH inserted AS (INSERT INTO pipeline_qa VALUES (4) RETURNING id) SELECT id, pg_sleep(1) FROM inserted",
            false, &cancel,
        );
        canceller.join().unwrap();
        assert!(cancelled.error.is_some());
        cancel.store(false, Ordering::Relaxed);
        let count: i64 = driver
            .client
            .query_one("SELECT count(*) FROM pipeline_qa", &[])
            .unwrap()
            .get(0);
        assert_eq!(count, 0, "server-cancelled CTE INSERT was committed");
        let recovered = driver.execute("SELECT 3 AS id", true, &cancel);
        assert!(recovered.error.is_none(), "{:?}", recovered.error);
        assert_eq!(recovered.grids[0].rows[0], [Value::Integer(3)]);
    }

    #[test]
    #[ignore = "requires opted-in disposable PostgreSQL QA server on port 54329"]
    fn editor_query_preserves_headers_safety_and_recovery() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let cancel = Arc::new(AtomicBool::new(false));
        let mut driver = PostgresDriver::open(
            &crate::connection::PostgresConfig {
                host: "127.0.0.1".into(),
                port: 54329,
                database: "rustable_qa".into(),
                user: "postgres".into(),
                password: "rustable_qa_only".into(),
                tls: false,
                ssh: None,
            },
            cancel.clone(),
        )
        .unwrap();
        let result = driver.execute(
            "SELECT 2::integer AS id, 'hello'::text AS name",
            true,
            &cancel,
        );
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(result.grids[0].columns, ["id", "name"]);
        assert_eq!(
            result.grids[0].rows[0],
            [Value::Integer(2), Value::Text("hello".into())]
        );
        let empty = driver.execute(
            "SELECT 2::integer AS id, 'hello'::text AS name WHERE false",
            true,
            &cancel,
        );
        assert!(empty.error.is_none(), "{:?}", empty.error);
        assert_eq!(empty.grids[0].columns, ["id", "name"]);
        assert!(empty.grids[0].rows.is_empty());
        // A temporary fixture is private to this connection; read-only mode
        // must still reject a write to a non-temporary relation.
        let denied = driver.execute("WITH modified AS (DELETE FROM pg_catalog.pg_class WHERE false RETURNING oid) SELECT * FROM modified", true, &cancel);
        assert!(
            denied
                .error
                .as_deref()
                .is_some_and(|error| error.contains("read-only")),
            "{:?}",
            denied.error
        );
        let failed = driver.execute("SELECT 1 / 0", true, &cancel);
        assert!(failed.error.is_some());
        let recovered = driver.execute(
            "SELECT current_setting('transaction_read_only') AS safety",
            true,
            &cancel,
        );
        assert!(recovered.error.is_none(), "{:?}", recovered.error);
        assert_eq!(recovered.grids[0].rows[0], [Value::Text("on".into())]);
        // A SQL function can alter session settings. Reapply canonical literal
        // parsing before the next statement even after that session mutation.
        driver
            .client
            .batch_execute("SET standard_conforming_strings = off")
            .unwrap();
        let literals = driver.execute(
            "SELECT current_setting('standard_conforming_strings')",
            false,
            &cancel,
        );
        assert!(literals.error.is_none(), "{:?}", literals.error);
        assert_eq!(literals.grids[0].rows[0], [Value::Text("on".into())]);
        assert_eq!(
            driver
                .client
                .query_one("SELECT current_setting('standard_conforming_strings')", &[])
                .unwrap()
                .get::<_, String>(0),
            "off"
        );
    }

    #[test]
    #[ignore = "requires opted-in disposable PostgreSQL QA server on port 54329"]
    fn browse_row_totals_use_catalog_estimates_and_observed_final_page() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let mut driver = PostgresDriver::open(
            &crate::connection::PostgresConfig {
                host: "127.0.0.1".into(),
                port: 54329,
                database: "rustable_qa".into(),
                user: "postgres".into(),
                password: "rustable_qa_only".into(),
                tls: false,
                ssh: None,
            },
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        driver.client.batch_execute("CREATE TEMP TABLE total_qa(id integer PRIMARY KEY); INSERT INTO total_qa SELECT generate_series(1,1200);").unwrap();
        let mut request = Browse {
            table: "total_qa".into(),
            ..Default::default()
        };
        assert_eq!(driver.browse(&request).unwrap().grid.total_rows, None);
        driver.client.batch_execute("ANALYZE total_qa;").unwrap();
        assert_eq!(
            driver.browse(&request).unwrap().grid.total_rows,
            Some(RowTotal {
                rows: 1200,
                estimated: true
            })
        );
        request.filter_column = Some("id".into());
        request.filter = "".into();
        assert_eq!(driver.browse(&request).unwrap().grid.total_rows, None);
        request.offset = 1000;
        assert_eq!(
            driver.browse(&request).unwrap().grid.total_rows,
            Some(RowTotal {
                rows: 1200,
                estimated: false
            })
        );
        driver.client.batch_execute("CREATE TEMP TABLE partition_total_qa(id integer) PARTITION BY RANGE(id); CREATE TEMP TABLE partition_total_qa_child PARTITION OF partition_total_qa FOR VALUES FROM (1) TO (2000); INSERT INTO partition_total_qa SELECT generate_series(1,1200); ANALYZE partition_total_qa;").unwrap();
        let partition = Browse {
            table: "partition_total_qa".into(),
            ..Default::default()
        };
        assert_eq!(
            driver.browse(&partition).unwrap().grid.total_rows,
            Some(RowTotal {
                rows: 1200,
                estimated: true
            })
        );
    }

    #[test]
    #[ignore = "requires opted-in disposable PostgreSQL QA server on port 54329"]
    fn browse_complex_types_uses_server_text_and_native_sort() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let cancel = Arc::new(AtomicBool::new(false));
        let mut driver = PostgresDriver::open(
            &crate::connection::PostgresConfig {
                host: "127.0.0.1".into(),
                port: 54329,
                database: "rustable_qa".into(),
                user: "postgres".into(),
                password: "rustable_qa_only".into(),
                tls: false,
                ssh: None,
            },
            cancel.clone(),
        )
        .unwrap();
        driver.client.batch_execute("CREATE TEMP TABLE initialize_temp(id int); CREATE TYPE pg_temp.mood AS ENUM ('z','a'); CREATE DOMAIN pg_temp.positive AS integer CHECK(VALUE>0); CREATE TEMP TABLE complex_browse(id integer PRIMARY KEY, tags text[], mood pg_temp.mood, score pg_temp.positive, duration interval, span int4range, bytes bytea); INSERT INTO complex_browse VALUES (1,ARRAY['a,b','日本',NULL],'z',10,'2 days','[1,5)',decode('ff00','hex')),(2,ARRAY['other'],'a',2,'10 days','[9,12)',NULL)").unwrap();
        let mut request = Browse {
            table: "complex_browse".into(),
            ..Default::default()
        };
        let data = driver.browse(&request).unwrap();
        assert_eq!(
            data.grid.rows[0],
            vec![
                Value::Integer(1),
                Value::Text("{\"a,b\",日本,NULL}".into()),
                Value::Text("z".into()),
                Value::Integer(10),
                Value::Text("+2 days".into()),
                Value::Text("[1,5)".into()),
                Value::Blob(vec![255, 0])
            ]
        );
        assert_eq!(
            data.grid.columns,
            vec!["id", "tags", "mood", "score", "duration", "span", "bytes"]
        );
        driver.client.batch_execute("ALTER TABLE complex_browse ADD CONSTRAINT score_unique UNIQUE(score); CREATE INDEX ordinary_tags ON complex_browse(tags)").unwrap();
        let indexes = driver.browse(&request).unwrap().indexes;
        assert_eq!(
            indexes
                .iter()
                .find(|index| index.name == "score_unique")
                .unwrap()
                .origin,
            "u"
        );
        assert_eq!(
            indexes
                .iter()
                .find(|index| index.name == "ordinary_tags")
                .unwrap()
                .origin,
            "c"
        );
        assert_eq!(
            indexes
                .iter()
                .find(|index| index.name == "complex_browse_pkey")
                .unwrap()
                .origin,
            "pk"
        );
        request.sort = Some(("duration".into(), false));
        assert_eq!(
            driver.browse(&request).unwrap().grid.rows[0][0],
            Value::Integer(1)
        );
        request.sort = Some(("mood".into(), false));
        assert_eq!(
            driver.browse(&request).unwrap().grid.rows[0][0],
            Value::Integer(1)
        );
        request.filter_column = Some("tags".into());
        request.filter = "日本".into();
        assert_eq!(driver.browse(&request).unwrap().grid.rows.len(), 1);
        let result = driver.execute(
            "SELECT 'a'::pg_temp.mood, 3::pg_temp.positive",
            true,
            &cancel,
        );
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(
            result.grids[0].rows[0],
            vec![Value::Text("a".into()), Value::Integer(3)]
        );
    }

    /// Only uses the disposable QA server; never accepts an arbitrary URL.
    #[test]
    #[ignore = "requires explicitly opted-in disposable PostgreSQL QA server on port 54329"]
    fn disposable_postgres_integration() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let cancel = Arc::new(AtomicBool::new(false));
        let config = crate::connection::PostgresConfig {
            host: "127.0.0.1".into(),
            port: 54329,
            database: "rustable_qa".into(),
            user: "postgres".into(),
            password: "rustable_qa_only".into(),
            tls: false,
            ssh: None,
        };
        let mut driver = PostgresDriver::open(&config, cancel.clone()).unwrap();
        let report=driver.execute("DROP SCHEMA IF EXISTS \"rustable.qa\" CASCADE; CREATE SCHEMA \"rustable.qa\"; CREATE TABLE \"rustable.qa\".\"test.table\"(id integer PRIMARY KEY, name text NOT NULL DEFAULT 'hello', amount numeric(30,8), bytes bytea); INSERT INTO \"rustable.qa\".\"test.table\" VALUES (1, 'a%b', 12345678901234567890.12345678, decode('00ff','hex')), (2,'other',NULL,NULL);",false,&cancel);
        assert!(report.error.is_none(), "{:?}", report.error);
        let values=driver.execute("SELECT DATE '1970-01-01', TIMESTAMP '2026-10-31 12:34:56.1234', TIMESTAMPTZ '2026-10-31 12:34:56+02', TIME '23:59:59.5', '123e4567-e89b-12d3-a456-426614174000'::uuid, '{\"a\":1}'::jsonb",true,&cancel);
        assert!(values.error.is_none(), "{:?}", values.error);
        assert_eq!(
            values.grids[0].rows[0],
            vec![
                Value::Text("1970-01-01".into()),
                Value::Text("2026-10-31 12:34:56.1234".into()),
                Value::Text("2026-10-31 10:34:56+00".into()),
                Value::Text("23:59:59.5".into()),
                Value::Text("123e4567-e89b-12d3-a456-426614174000".into()),
                Value::Text("{\"a\": 1}".into())
            ]
        );
        let objects = driver.select_schema("rustable.qa").unwrap();
        let table = objects
            .iter()
            .find(|o| o.name == "\"rustable.qa\".\"test.table\"")
            .unwrap();
        for (column, operator, value, expected) in [("id", ">", "1", 2), ("name", "Contains", "%", 1), ("id", "IN", "[2]", 2), ("Raw SQL", "=", "id = 1 AND name LIKE 'a%'", 1)] {
            let request = Browse { table: table.name.clone(), filters: vec![crate::db::filters::Filter { enabled: true, column: column.into(), operator: operator.into(), value: value.into() }], ..Default::default() };
            let filtered = driver.browse(&request).unwrap();
            assert_eq!(filtered.grid.rows.len(), 1);
            assert_eq!(filtered.grid.rows[0][0], Value::Integer(expected));
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("filtered.json");
            driver.export_table(&path, &request, &cancel, ExportFormat::Json).unwrap();
            let json: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            assert_eq!(json[0]["id"], expected);
            assert_eq!(json.as_array().unwrap().len(), 1);
        }
        assert!(table.columns[0].pk);
        assert!(table.columns[1].not_null);
        assert!(!table.columns[1].default.is_empty());
        let page = driver
            .browse(&Browse {
                table: table.name.clone(),
                filter_column: Some("name".into()),
                filter: "%".into(),
                ..Browse::default()
            })
            .unwrap();
        assert_eq!(page.grid.rows.len(), 1);
        assert_eq!(
            page.grid.rows[0][2],
            Value::Text("12345678901234567890.12345678".into())
        );
        assert_eq!(page.grid.rows[0][3], Value::Blob(vec![0, 255]));
        let report=driver.execute("SELECT amount,bytes FROM \"rustable.qa\".\"test.table\" WHERE id=2; SELECT generate_series(1,6000); SELECT $$semi;colon$$",true,&cancel);
        assert!(report.error.is_none(), "{:?}", report.error);
        assert_eq!(report.grids.len(), 3);
        assert_eq!(report.grids[0].rows[0], vec![Value::Null, Value::Null]);
        assert!(report.grids[1].truncated);
        assert_eq!(report.grids[1].rows.len(), QUERY_LIMIT);
        assert_eq!(report.grids[2].rows[0][0], Value::Text("semi;colon".into()));
        for sql in [
            "WITH deleted AS (DELETE FROM \"rustable.qa\".\"test.table\" RETURNING *) SELECT * FROM deleted",
            "EXPLAIN ANALYZE DELETE FROM \"rustable.qa\".\"test.table\"",
            "SELECT set_config('transaction_read_only','off',false)",
            "COMMIT; DELETE FROM \"rustable.qa\".\"test.table\"",
        ] {
            let report = driver.execute(sql, true, &cancel);
            assert!(report.error.is_some(), "safe mode accepted {sql}");
        }
        let report = driver.execute(
            "SELECT count(*) FROM \"rustable.qa\".\"test.table\"",
            true,
            &cancel,
        );
        assert!(report.error.is_none(), "{:?}", report.error);
        assert_eq!(report.grids[0].rows[0][0], Value::Integer(2));
        let trigger = cancel.clone();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            trigger.store(true, Ordering::Relaxed);
        });
        let timer = Instant::now();
        let report = driver.execute("SELECT pg_sleep(10)", true, &cancel);
        thread.join().unwrap();
        assert!(report.error.is_some());
        assert!(timer.elapsed() < Duration::from_secs(5));
        cancel.store(false, Ordering::Relaxed);
        let report = driver.execute("SELECT 42", true, &cancel);
        assert!(report.error.is_none(), "{:?}", report.error);
        assert_eq!(report.grids[0].rows[0][0], Value::Integer(42));
        assert!(
            driver
                .execute("DROP SCHEMA \"rustable.qa\" CASCADE", false, &cancel)
                .error
                .is_none()
        );
    }
}

#[cfg(test)]
mod ssh_live_tests {
    use super::*;

    #[test]
    #[ignore = "requires opt-in disposable local sshd and PostgreSQL QA fixtures"]
    fn disposable_postgres_over_ssh() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let directory = std::path::PathBuf::from(
            std::env::var("RUSTABLE_SSH_QA_DIR").expect("temporary local SSH QA key directory"),
        );
        assert!(directory.starts_with("/tmp"));
        let ssh = crate::ssh_tunnel::Config {
            host: "127.0.0.1".into(),
            port: 54328,
            user: std::env::var("USER").unwrap(),
            identity_file: Some(directory.join("client")),
            known_hosts_file: Some(directory.join("known_hosts")),
            config_file: Some("/dev/null".into()),
            remote_host: "127.0.0.1".into(),
            remote_port: 54329,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let config = crate::connection::PostgresConfig {
            host: "127.0.0.1".into(),
            port: 54329,
            database: "rustable_qa".into(),
            user: "postgres".into(),
            password: "rustable_qa_only".into(),
            tls: false,
            ssh: Some(ssh.clone()),
        };
        let mut driver = PostgresDriver::open(&config, cancel.clone()).unwrap();
        let report = driver.execute("SELECT 'through SSH'", true, &cancel);
        assert!(report.error.is_none(), "{:?}", report.error);
        assert_eq!(
            report.grids[0].rows[0][0],
            Value::Text("through SSH".into())
        );
        let trigger = cancel.clone();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            trigger.store(true, Ordering::Relaxed);
        });
        let started = Instant::now();
        let report = driver.execute("SELECT pg_sleep(10)", true, &cancel);
        thread.join().unwrap();
        assert!(report.error.is_some());
        assert!(started.elapsed() < Duration::from_secs(5));
        cancel.store(false, Ordering::Relaxed);
        assert!(driver.execute("SELECT 42", true, &cancel).error.is_none());
        let port = driver.tunnel.as_ref().unwrap().local_port();
        drop(driver);
        assert!(
            std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).is_ok(),
            "dropping driver must close its tunnel listener"
        );
        let spaced_key = directory.join("client %h spaced");
        let spaced_hosts = directory.join("known \"hosts\" %p spaced");
        std::fs::copy(directory.join("client"), &spaced_key).unwrap();
        std::fs::copy(directory.join("known_hosts"), &spaced_hosts).unwrap();
        let mut unusual_paths = ssh.clone();
        unusual_paths.identity_file = Some(spaced_key);
        unusual_paths.known_hosts_file = Some(spaced_hosts);
        drop(
            crate::ssh_tunnel::Tunnel::start(&unusual_paths, &cancel)
                .expect("literal spaces, quotes, and percent signs in key/known-host paths"),
        );
        let unknown = tempfile::NamedTempFile::new().unwrap();
        let mut untrusted = ssh.clone();
        untrusted.known_hosts_file = Some(unknown.path().into());
        let error = crate::ssh_tunnel::Tunnel::start(&untrusted, &cancel)
            .err()
            .expect("unknown SSH host key must fail");
        assert!(error.contains("Host key verification failed"), "{error}");
        let changed = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            changed.path(),
            format!(
                "[127.0.0.1]:54328 {}",
                std::fs::read_to_string(directory.join("client.pub")).unwrap()
            ),
        )
        .unwrap();
        untrusted.known_hosts_file = Some(changed.path().into());
        let error = crate::ssh_tunnel::Tunnel::start(&untrusted, &cancel)
            .err()
            .expect("changed SSH host key must fail");
        assert!(error.contains("HOST IDENTIFICATION HAS CHANGED"), "{error}");
    }
}

#[cfg(test)]
mod schema_tests {
    use super::*;
    #[test]
    fn default_prefers_public_then_current_schema_then_accessible_user_schema() {
        let names = vec![
            "_timescaledb_internal".into(),
            "pg_catalog".into(),
            "private".into(),
            "public".into(),
        ];
        assert_eq!(
            preferred_schema(&names, Some("private")),
            Some("public".into())
        );
        assert_eq!(
            preferred_schema(&names[..3], Some("private")),
            Some("private".into())
        );
        assert_eq!(
            preferred_schema(&["information_schema".into(), "tenant".into()], None),
            Some("tenant".into())
        );
        assert_eq!(preferred_schema(&[], None), None);
    }

    #[test]
    #[ignore = "requires opted-in disposable PostgreSQL server on port54329"]
    fn streamed_catalog_preserves_empty_relations_and_column_metadata() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let cancel = Arc::new(AtomicBool::new(false));
        let mut driver = PostgresDriver::open(&crate::connection::PostgresConfig {
            host: "127.0.0.1".into(), port: 54329, database: "rustable_qa".into(),
            user: "postgres".into(), password: "rustable_qa_only".into(), tls: false, ssh: None,
        }, cancel).unwrap();
        driver.client.batch_execute("BEGIN; CREATE SCHEMA rustable_catalog_stream_qa; CREATE TABLE rustable_catalog_stream_qa.empty(); CREATE TABLE rustable_catalog_stream_qa.typed(id integer GENERATED ALWAYS AS IDENTITY PRIMARY KEY, gone text, name text NOT NULL DEFAULT 'hello', calculated integer GENERATED ALWAYS AS (id+1) STORED); ALTER TABLE rustable_catalog_stream_qa.typed DROP COLUMN gone; CREATE VIEW rustable_catalog_stream_qa.visible AS SELECT id,name FROM rustable_catalog_stream_qa.typed").unwrap();
        let objects = driver.select_schema("rustable_catalog_stream_qa").unwrap();
        assert_eq!(objects.iter().map(Object::display_name).collect::<Vec<_>>(), ["empty", "typed", "visible"]);
        assert!(objects[0].columns.is_empty());
        assert_eq!(objects[0].kind, "table");
        assert_eq!(objects[1].columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["id", "name", "calculated"]);
        assert!(objects[1].columns[0].pk && objects[1].columns[0].generated);
        assert!(objects[1].columns[1].not_null);
        assert!(objects[1].columns[1].default.contains("hello"));
        assert!(objects[1].columns[2].generated);
        assert_eq!(objects[2].kind, "view");
        assert_eq!(objects[2].columns.len(), 2);
        assert_eq!(driver.client.transaction_status(), Some(b'T'));
        driver.client.batch_execute("ALTER TABLE rustable_catalog_stream_qa.typed ADD COLUMN fresh boolean").unwrap();
        let refreshed = driver.objects().unwrap();
        assert_eq!(refreshed[1].columns.last().unwrap().name, "fresh");
        driver.client.batch_execute("ROLLBACK").unwrap();
        driver.objects().unwrap();
        assert!(!driver.schemas.names.iter().any(|s| s == "rustable_catalog_stream_qa"));
    }

    #[test]
    #[ignore = "requires opted-in disposable PostgreSQL server on port54329"]
    fn disposable_schema_navigation_excludes_extension_tables_until_selected() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let cancel = Arc::new(AtomicBool::new(false));
        let config = crate::connection::PostgresConfig {
            host: "127.0.0.1".into(),
            port: 54329,
            database: "rustable_qa".into(),
            user: "postgres".into(),
            password: "rustable_qa_only".into(),
            tls: false,
            ssh: None,
        };
        let mut driver = PostgresDriver::open(&config, cancel.clone()).unwrap();
        let cleanup = "DROP TABLE IF EXISTS public.rustable_schema_qa; DROP SCHEMA IF EXISTS _timescaledb_catalog_rustable_qa CASCADE; DROP SCHEMA IF EXISTS _timescaledb_internal_rustable_qa CASCADE; DROP SCHEMA IF EXISTS \"qa.schema\"\"x\" CASCADE;";
        assert!(driver.execute(cleanup, false, &cancel).error.is_none());
        let setup = "CREATE TABLE public.rustable_schema_qa(public_value integer); INSERT INTO public.rustable_schema_qa VALUES(17); CREATE SCHEMA _timescaledb_catalog_rustable_qa; CREATE TABLE _timescaledb_catalog_rustable_qa.rustable_schema_qa(internal_value text); CREATE SCHEMA _timescaledb_internal_rustable_qa; CREATE TABLE _timescaledb_internal_rustable_qa.chunk_1(secret_value text); CREATE SCHEMA \"qa.schema\"\"x\"; CREATE TABLE \"qa.schema\"\"x\".\"odd.table\"\"name\"(\"odd.column\" text); INSERT INTO \"qa.schema\"\"x\".\"odd.table\"\"name\" VALUES('unusual');";
        let report = driver.execute(setup, false, &cancel);
        assert!(report.error.is_none(), "{:?}", report.error);
        let objects = driver.objects().unwrap();
        let schemas = driver.schema_catalog();
        assert_eq!(schemas.selected.as_deref(), Some("public"));
        assert!(
            schemas
                .names
                .iter()
                .any(|name| name == "_timescaledb_catalog_rustable_qa")
        );
        assert!(
            objects
                .iter()
                .all(|object| object.schema.as_deref() == Some("public"))
        );
        let public = objects
            .iter()
            .find(|object| object.display_name() == "rustable_schema_qa")
            .unwrap();
        assert_eq!(public.name, "\"public\".\"rustable_schema_qa\"");
        assert_eq!(public.columns[0].name, "public_value");
        let public_name = public.name.clone();
        let internal = driver
            .select_schema("_timescaledb_catalog_rustable_qa")
            .unwrap();
        assert_eq!(internal.len(), 1);
        assert_eq!(internal[0].display_name(), public.display_name());
        assert_ne!(internal[0].name, public_name);
        assert_eq!(internal[0].columns[0].name, "internal_value");
        assert_eq!(
            driver
                .browse(&Browse {
                    table: public_name,
                    ..Default::default()
                })
                .unwrap()
                .grid
                .rows[0][0],
            Value::Integer(17)
        );
        let odd = driver.select_schema("qa.schema\"x").unwrap();
        assert_eq!(odd.len(), 1);
        assert_eq!(odd[0].display_name(), "odd.table\"name");
        assert_eq!(odd[0].columns[0].name, "odd.column");
        assert_eq!(
            driver
                .browse(&Browse {
                    table: odd[0].name.clone(),
                    ..Default::default()
                })
                .unwrap()
                .grid
                .rows[0][0],
            Value::Text("unusual".into())
        );
        assert!(driver.select_schema("missing_schema").is_err());
        assert_eq!(
            driver.schema_catalog().selected.as_deref(),
            Some("qa.schema\"x")
        );
        assert!(driver.execute(cleanup, false, &cancel).error.is_none());
        driver.objects().unwrap();
        assert_eq!(driver.schema_catalog().selected.as_deref(), Some("public"));
    }
}

#[cfg(test)]
mod latency_tests;

#[cfg(test)]
mod memory_tests;

#[cfg(test)]
mod transport_tests;

#[cfg(test)]
mod foreign_key_navigation_tests {
    use super::*;
    #[test]
    #[ignore = "requires opted-in disposable PostgreSQL QA server on port 54329"]
    fn live_composite_foreign_key_navigation() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let mut driver = PostgresDriver::open(&crate::connection::PostgresConfig { host:"127.0.0.1".into(),port:54329,database:"rustable_qa".into(),user:"postgres".into(),password:"rustable_qa_only".into(),tls:false,ssh:None },Arc::new(AtomicBool::new(false))).unwrap();
        driver.client.batch_execute("CREATE TEMP TABLE fk_nav_parent(tenant TEXT,id INT,PRIMARY KEY(tenant,id)); CREATE TEMP TABLE fk_nav_child(a TEXT,b INT,FOREIGN KEY(a,b) REFERENCES fk_nav_parent(tenant,id)); INSERT INTO fk_nav_parent VALUES('a',3),('b',3); INSERT INTO fk_nav_child VALUES('a',3)").unwrap();
        for _ in 0..2 {
            let source = driver.browse(&Browse { table:"fk_nav_child".into(),..Default::default() }).unwrap();
            assert_eq!(source.foreign_keys.len(),1);
            let key = &source.foreign_keys[0];
            assert_eq!(key.referenced_columns,vec!["tenant","id"]);
            let filters = key.filters(&source.grid.columns,&source.grid.rows[0]).unwrap();
            let table = format!("{}.{}",quote(key.schema.as_ref().unwrap()),quote(&key.table));
            let target = driver.browse(&Browse { table,filters,..Default::default() }).unwrap();
            assert_eq!(target.grid.rows.len(),1);
            assert_eq!(target.grid.rows[0][0],Value::Text("a".into()));
        }
    }
}

#[cfg(test)]
mod query_lifetime_tests {
    use super::*;
    #[test]
    #[ignore = "requires opted-in disposable PostgreSQL on port 54329"]
    fn queries_have_no_default_statement_or_lock_deadline() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let config = crate::connection::PostgresConfig {
            host: "127.0.0.1".into(), port: 54329, database: "rustable_qa".into(),
            user: "postgres".into(), password: "rustable_qa_only".into(), tls: false, ssh: None,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let mut driver = PostgresDriver::open(&config, cancel.clone()).unwrap();
        for setting in ["statement_timeout", "lock_timeout"] {
            let row = driver.client.query_one("SELECT current_setting($1)", &[&setting]).unwrap();
            assert_eq!(row.get::<_, String>(0), "0");
        }
        let trigger = cancel.clone();
        let cancellation = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            trigger.store(true, Ordering::Relaxed);
        });
        let report = driver.execute("SELECT pg_sleep(15)", false, &cancel);
        cancellation.join().unwrap();
        assert!(report.error.as_ref().is_some_and(|e| e.contains("cancel")), "{:?}", report.error);
        cancel.store(false, Ordering::Relaxed);
        assert!(driver.execute("SELECT 1", false, &cancel).error.is_none());
    }
}

#[cfg(test)]
mod custom_type_stream_tests {
    use super::*;
    #[test]
    #[ignore = "requires opted-in disposable PostgreSQL on port 54329"]
    fn uncached_custom_type_with_large_rows_does_not_stall() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let config = crate::connection::PostgresConfig {
            host: "127.0.0.1".into(), port: 54329, database: "rustable_qa".into(),
            user: "postgres".into(), password: "rustable_qa_only".into(), tls: false, ssh: None,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let mut driver = PostgresDriver::open(&config, cancel.clone()).unwrap();
        driver.client.batch_execute("CREATE TYPE pg_temp.stream_state AS ENUM ('active'); CREATE TEMP TABLE stream_stores AS SELECT i, 'active'::pg_temp.stream_state AS state, repeat('x',100000) AS payload, NULL::text AS col FROM generate_series(1,20) i;").unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let result = driver.execute("SELECT * FROM stream_stores WHERE col IS NULL LIMIT 20", true, &cancel);
            let _ = tx.send(result);
        });
        let report = rx.recv_timeout(Duration::from_secs(5)).expect("custom-type result stream stalled");
        assert!(report.error.is_none(), "{:?}", report.error);
        assert_eq!(report.grids[0].rows.len(), 20);
        worker.join().unwrap();
    }
}
