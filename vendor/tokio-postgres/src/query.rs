use crate::client::{InnerClient, Responses};
use crate::codec::FrontendMessage;
use crate::connection::RequestMessages;
use crate::prepare::get_type;
use crate::types::{BorrowToSql, IsNull};
use crate::{Column, Error, Portal, Row, Statement};
use bytes::{Bytes, BytesMut};
use fallible_iterator::FallibleIterator;
use futures_util::Stream;
use log::{Level, debug, log_enabled};
use pin_project_lite::pin_project;
use postgres_protocol::message::backend::{CommandCompleteBody, DataRowBody, Message};
use postgres_protocol::message::frontend;
use postgres_types::Type;
use std::collections::VecDeque;
use std::fmt;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, ready};

struct BorrowToSqlParamsDebug<'a, T>(&'a [T]);

impl<T> fmt::Debug for BorrowToSqlParamsDebug<'_, T>
where
    T: BorrowToSql,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.0.iter().map(|x| x.borrow_to_sql()))
            .finish()
    }
}

pub async fn query<P, I>(
    client: &InnerClient,
    statement: Statement,
    params: I,
) -> Result<RowStream, Error>
where
    P: BorrowToSql,
    I: IntoIterator<Item = P>,
    I::IntoIter: ExactSizeIterator,
{
    let buf = if log_enabled!(Level::Debug) {
        let params = params.into_iter().collect::<Vec<_>>();
        debug!(
            "executing statement {} with parameters: {:?}",
            statement.name(),
            BorrowToSqlParamsDebug(params.as_slice()),
        );
        encode(client, &statement, params)?
    } else {
        encode(client, &statement, params)?
    };
    let responses = start(client, buf).await?;
    Ok(RowStream {
        statement,
        responses,
        rows_affected: None, buffered: VecDeque::new(), complete: false, truncated: false,
    })
}

pub async fn query_typed<P, I>(
    client: &Arc<InnerClient>,
    query: &str,
    params: I,
) -> Result<RowStream, Error>
where
    P: BorrowToSql,
    I: IntoIterator<Item = (P, Type)>,
{
    query_typed_with_prefix(client, query, params, &[]).await
}

pub async fn query_typed_with_prefix<P, I>(
    client: &Arc<InnerClient>,
    query: &str,
    params: I,
    prefix: &[&str],
) -> Result<RowStream, Error>
where
    P: BorrowToSql,
    I: IntoIterator<Item = (P, Type)>,
{
    // Resolve result types before Execute. Looking up an unknown OID while
    // Execute's rows fill the bounded response channel deadlocks the connection:
    // the lookup response is queued behind rows nobody is consuming yet.
    static NEXT_TYPED_ID: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let name = format!("rt{}", NEXT_TYPED_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let (buf, execution_buf) = {
        let params = params.into_iter().collect::<Vec<_>>();
        let buf = client.with_buf(|buf| {
            // Only these idempotent session safeguards need to govern Parse too.
            // Arbitrary caller-supplied prefix commands must execute exactly once.
            let parse_safeguards = prefix.iter().copied().filter(|command| matches!(*command,
                "SET LOCAL standard_conforming_strings = on" | "SET LOCAL transaction_read_only = on"
            )).collect::<Vec<_>>();
            encode_prefix(&parse_safeguards, buf)?;
            frontend::parse(&name, query, params.iter().map(|(_, t)| t.oid()), buf).map_err(Error::parse)?;
            frontend::describe(b'S', &name, buf).map_err(Error::encode)?;
            frontend::sync(buf);
            Ok(buf.split().freeze())
        })?;
        let execution_buf = client.with_buf(|buf| {
            encode_prefix(prefix, buf)?;
            encode_bind_raw(&name, params, "", buf)?;
            // The bound portal retains the plan. Close before Execute so even
            // an execution error cannot skip cleanup, and share its Sync.
            frontend::close(b'S', &name, buf).map_err(Error::encode)?;
            frontend::execute("", 0, buf).map_err(Error::encode)?;
            frontend::sync(buf);
            Ok(buf.split().freeze())
        })?;
        (buf, execution_buf)
    };
    let mut responses = client.send(RequestMessages::Single(FrontendMessage::Raw(buf)))?;
    let mut prefix_remaining = prefix.iter().filter(|command| matches!(**command,
        "SET LOCAL standard_conforming_strings = on" | "SET LOCAL transaction_read_only = on"
    )).count();
    let mut columns = vec![];
    loop {
        match responses.next().await? {
            Message::ParseComplete | Message::BindComplete | Message::ParameterDescription(_) => {}
            Message::CommandComplete(_) if prefix_remaining > 0 => { prefix_remaining -= 1; }
            Message::NoData if prefix_remaining == 0 => {}
            Message::RowDescription(description) if prefix_remaining == 0 => {
                let mut fields = description.fields();
                while let Some(field) = fields.next().map_err(Error::parse)? {
                    columns.push((field.name().to_owned(), field.table_oid(), field.column_id(), field.type_modifier(), field.type_oid()));
                }
            }
            Message::ReadyForQuery(_) if prefix_remaining == 0 => break,
            _ => return Err(Error::unexpected_message()),
        }
    }
    // Keep ownership even if resolving a type fails so the named statement closes.
    let mut prepared = Statement::new(client, name, vec![], vec![]);
    let mut resolved = Vec::with_capacity(columns.len());
    for (name, table_oid, column_id, type_modifier, oid) in columns {
        resolved.push(Column {
            name, table_oid: Some(table_oid).filter(|n| *n != 0),
            column_id: Some(column_id).filter(|n| *n != 0), type_modifier,
            r#type: get_type(client, oid).await?,
        });
    }
    // SET LOCAL from description expires at Sync; execution repeats safeguards.
    let mut responses = client.send(RequestMessages::Single(FrontendMessage::Raw(execution_buf)))?;
    let mut prefix_remaining = prefix.len();
    loop {
        match responses.next().await? {
            Message::ParseComplete => {}
            Message::CommandComplete(_) if prefix_remaining > 0 => { prefix_remaining -= 1; }
            Message::BindComplete if prefix_remaining > 0 => {}
            Message::BindComplete => {}
            Message::CloseComplete if prefix_remaining == 0 => {
                prepared.disarm_close();
                break;
            }
            _ => return Err(Error::unexpected_message()),
        }
    }
    // The statement is already closed; the portal owns execution and RowStream
    // only needs the resolved column metadata.
    Ok(RowStream {
        statement: Statement::unnamed(vec![], resolved), responses, rows_affected: None, buffered: VecDeque::new(), complete: false, truncated: false,
    })
}

/// Parse, describe and execute in one exchange. Unknown types are resolved only
/// after draining execution, so catalog queries cannot block behind unread rows.
pub async fn query_typed_bounded_with_prefix<P, I>(
    client: &Arc<InnerClient>, query: &str, params: I, prefix: &[&str],
    max_rows: usize, max_bytes: usize,
) -> Result<RowStream, Error>
where
    P: BorrowToSql,
    I: IntoIterator<Item = (P, Type)>,
{
    let params = params.into_iter().collect::<Vec<_>>();
    let buf = client.with_buf(|buf| {
        encode_prefix(prefix, buf)?;
        frontend::parse("", query, params.iter().map(|(_, t)| t.oid()), buf).map_err(Error::parse)?;
        frontend::describe(b'S', "", buf).map_err(Error::encode)?;
        encode_bind_raw("", params, "", buf)?;
        frontend::execute("", 0, buf).map_err(Error::encode)?;
        frontend::sync(buf);
        Ok(buf.split().freeze())
    })?;
    let mut responses = client.send(RequestMessages::Single(FrontendMessage::Raw(buf)))?;
    let mut prefix_remaining = prefix.len();
    let mut columns = Vec::new();
    loop {
        match responses.next().await? {
            Message::ParseComplete | Message::ParameterDescription(_) => {}
            Message::CommandComplete(_) if prefix_remaining > 0 => { prefix_remaining -= 1; }
            Message::BindComplete if prefix_remaining > 0 => {}
            Message::NoData if prefix_remaining == 0 => {}
            Message::RowDescription(description) if prefix_remaining == 0 => {
                let mut fields = description.fields();
                while let Some(field) = fields.next().map_err(Error::parse)? {
                    columns.push((field.name().to_owned(), field.table_oid(), field.column_id(), field.type_modifier(), field.type_oid()));
                }
            }
            Message::BindComplete => break,
            _ => return Err(Error::unexpected_message()),
        }
    }
    let cached = columns.iter().map(|column| {
        Type::from_oid(column.4).or_else(|| client.type_(column.4))
    }).collect::<Option<Vec<_>>>();
    let mut buffered = VecDeque::new();
    let mut rows_affected = None;
    let mut truncated = false;
    let complete = cached.is_none();
    if complete {
        let mut bytes = 0usize;
        loop {
            match responses.next().await? {
                Message::DataRow(body) => {
                    let size = body.buffer().len().saturating_add(std::mem::size_of::<DataRowBody>());
                    if truncated || buffered.len() >= max_rows || size > max_bytes.saturating_sub(bytes) {
                        truncated = true;
                    } else {
                        bytes += size;
                        buffered.push_back(body);
                    }
                }
                Message::CommandComplete(body) => { rows_affected = Some(extract_row_affected(&body)?); }
                Message::EmptyQueryResponse => {}
                Message::ReadyForQuery(_) => break,
                _ => return Err(Error::unexpected_message()),
            }
        }
    }
    let types = match cached {
        Some(types) => types,
        None => {
            let mut types = Vec::with_capacity(columns.len());
            for column in &columns { types.push(get_type(client, column.4).await?); }
            types
        }
    };
    let resolved = columns.into_iter().zip(types).map(|((name, table_oid, column_id, type_modifier, _), r#type)| Column {
        name, table_oid: Some(table_oid).filter(|n| *n != 0),
        column_id: Some(column_id).filter(|n| *n != 0), type_modifier, r#type,
    }).collect();
    Ok(RowStream {
        statement: Statement::unnamed(vec![], resolved), responses, rows_affected,
        buffered, complete, truncated,
    })
}

fn encode_prefix(prefix: &[&str], buf: &mut BytesMut) -> Result<(), Error> {
    for command in prefix {
        frontend::parse("", command, std::iter::empty(), buf).map_err(Error::parse)?;
        encode_bind_raw("", std::iter::empty::<(&dyn crate::types::ToSql, Type)>(), "", buf)?;
        frontend::execute("", 0, buf).map_err(Error::encode)?;
    }
    Ok(())
}

pub async fn execute_typed<P, I>(
    client: &Arc<InnerClient>,
    query: &str,
    params: I,
) -> Result<u64, Error>
where
    P: BorrowToSql,
    I: IntoIterator<Item = (P, Type)>,
{
    let buf = {
        let params = params.into_iter().collect::<Vec<_>>();
        let param_oids = params.iter().map(|(_, t)| t.oid()).collect::<Vec<_>>();

        client.with_buf(|buf| {
            frontend::parse("", query, param_oids, buf).map_err(Error::parse)?;
            encode_bind_raw("", params, "", buf)?;
            frontend::describe(b'S', "", buf).map_err(Error::encode)?;
            frontend::execute("", 0, buf).map_err(Error::encode)?;
            frontend::sync(buf);

            Ok(buf.split().freeze())
        })?
    };

    let mut responses = client.send(RequestMessages::Single(FrontendMessage::Raw(buf)))?;

    let mut rows = 0;

    loop {
        match responses.next().await? {
            Message::ParseComplete
            | Message::BindComplete
            | Message::ParameterDescription(_)
            | Message::RowDescription(_) => {}
            Message::NoData => {
                rows = 0;
            }

            Message::DataRow(_) => {}
            Message::CommandComplete(body) => {
                rows = extract_row_affected(&body)?;
            }

            Message::EmptyQueryResponse => rows = 0,
            Message::ReadyForQuery(_) => return Ok(rows),
            _ => {
                return Err(Error::unexpected_message());
            }
        }
    }
}

pub async fn query_portal(
    client: &InnerClient,
    portal: &Portal,
    max_rows: i32,
) -> Result<RowStream, Error> {
    let buf = client.with_buf(|buf| {
        frontend::execute(portal.name(), max_rows, buf).map_err(Error::encode)?;
        frontend::sync(buf);
        Ok(buf.split().freeze())
    })?;

    let responses = client.send(RequestMessages::Single(FrontendMessage::Raw(buf)))?;

    Ok(RowStream {
        statement: portal.statement().clone(),
        responses,
        rows_affected: None, buffered: VecDeque::new(), complete: false, truncated: false,
    })
}

/// Extract the number of rows affected from [`CommandCompleteBody`].
pub fn extract_row_affected(body: &CommandCompleteBody) -> Result<u64, Error> {
    let rows = body
        .tag()
        .map_err(Error::parse)?
        .rsplit(' ')
        .next()
        .unwrap()
        .parse()
        .unwrap_or(0);
    Ok(rows)
}

pub async fn execute<P, I>(
    client: &InnerClient,
    statement: Statement,
    params: I,
) -> Result<u64, Error>
where
    P: BorrowToSql,
    I: IntoIterator<Item = P>,
    I::IntoIter: ExactSizeIterator,
{
    let buf = if log_enabled!(Level::Debug) {
        let params = params.into_iter().collect::<Vec<_>>();
        debug!(
            "executing statement {} with parameters: {:?}",
            statement.name(),
            BorrowToSqlParamsDebug(params.as_slice()),
        );
        encode(client, &statement, params)?
    } else {
        encode(client, &statement, params)?
    };
    let mut responses = start(client, buf).await?;

    let mut rows = 0;
    loop {
        match responses.next().await? {
            Message::DataRow(_) => {}
            Message::CommandComplete(body) => {
                rows = extract_row_affected(&body)?;
            }
            Message::EmptyQueryResponse => rows = 0,
            Message::ReadyForQuery(_) => return Ok(rows),
            _ => return Err(Error::unexpected_message()),
        }
    }
}

async fn start(client: &InnerClient, buf: Bytes) -> Result<Responses, Error> {
    let mut responses = client.send(RequestMessages::Single(FrontendMessage::Raw(buf)))?;

    match responses.next().await? {
        Message::BindComplete => {}
        _ => return Err(Error::unexpected_message()),
    }

    Ok(responses)
}

pub fn encode<P, I>(client: &InnerClient, statement: &Statement, params: I) -> Result<Bytes, Error>
where
    P: BorrowToSql,
    I: IntoIterator<Item = P>,
    I::IntoIter: ExactSizeIterator,
{
    client.with_buf(|buf| {
        encode_bind(statement, params, "", buf)?;
        frontend::execute("", 0, buf).map_err(Error::encode)?;
        frontend::sync(buf);
        Ok(buf.split().freeze())
    })
}

pub fn encode_bind<P, I>(
    statement: &Statement,
    params: I,
    portal: &str,
    buf: &mut BytesMut,
) -> Result<(), Error>
where
    P: BorrowToSql,
    I: IntoIterator<Item = P>,
    I::IntoIter: ExactSizeIterator,
{
    let params = params.into_iter();
    if params.len() != statement.params().len() {
        return Err(Error::parameters(params.len(), statement.params().len()));
    }

    encode_bind_raw(
        statement.name(),
        params.zip(statement.params().iter().cloned()),
        portal,
        buf,
    )
}

fn encode_bind_raw<P, I>(
    statement_name: &str,
    params: I,
    portal: &str,
    buf: &mut BytesMut,
) -> Result<(), Error>
where
    P: BorrowToSql,
    I: IntoIterator<Item = (P, Type)>,
    I::IntoIter: ExactSizeIterator,
{
    let (param_formats, params): (Vec<_>, Vec<_>) = params
        .into_iter()
        .map(|(p, ty)| (p.borrow_to_sql().encode_format(&ty) as i16, (p, ty)))
        .unzip();

    let mut error_idx = 0;
    let r = frontend::bind(
        portal,
        statement_name,
        param_formats,
        params.into_iter().enumerate(),
        |(idx, (param, ty)), buf| match param.borrow_to_sql().to_sql_checked(&ty, buf) {
            Ok(IsNull::No) => Ok(postgres_protocol::IsNull::No),
            Ok(IsNull::Yes) => Ok(postgres_protocol::IsNull::Yes),
            Err(e) => {
                error_idx = idx;
                Err(e)
            }
        },
        Some(1),
        buf,
    );
    match r {
        Ok(()) => Ok(()),
        Err(frontend::BindError::Conversion(e)) => Err(Error::to_sql(e, error_idx)),
        Err(frontend::BindError::Serialization(e)) => Err(Error::encode(e)),
    }
}

pin_project! {
    /// A stream of table rows.
    #[project(!Unpin)]
    pub struct RowStream {
        statement: Statement,
        responses: Responses,
        rows_affected: Option<u64>,
        buffered: VecDeque<DataRowBody>,
        complete: bool,
        truncated: bool,
    }
}

impl Stream for RowStream {
    type Item = Result<Row, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();
        if let Some(body) = this.buffered.pop_front() {
            return Poll::Ready(Some(Row::new(this.statement.clone(), body)));
        }
        if *this.complete { return Poll::Ready(None); }
        loop {
            match ready!(this.responses.poll_next(cx)?) {
                Message::DataRow(body) => {
                    return Poll::Ready(Some(Ok(Row::new(this.statement.clone(), body)?)));
                }
                Message::CommandComplete(body) => {
                    *this.rows_affected = Some(extract_row_affected(&body)?);
                }
                Message::EmptyQueryResponse | Message::PortalSuspended => {}
                Message::ReadyForQuery(_) => return Poll::Ready(None),
                _ => return Poll::Ready(Some(Err(Error::unexpected_message()))),
            }
        }
    }
}

impl RowStream {
    /// Whether rows were discarded while draining uncached custom types.
    pub fn truncated(&self) -> bool { self.truncated }

    /// Returns the result columns, including for an empty result set.
    pub fn columns(&self) -> &[Column] {
        self.statement.columns()
    }

    /// Returns the number of rows affected by the query.
    ///
    /// This function will return `None` until the stream has been exhausted.
    pub fn rows_affected(&self) -> Option<u64> {
        self.rows_affected
    }
}

pub async fn sync(client: &InnerClient) -> Result<(), Error> {
    let buf = Bytes::from_static(b"S\0\0\0\x04");
    let mut responses = client.send(RequestMessages::Single(FrontendMessage::Raw(buf)))?;

    match responses.next().await? {
        Message::ReadyForQuery(_) => Ok(()),
        _ => Err(Error::unexpected_message()),
    }
}
