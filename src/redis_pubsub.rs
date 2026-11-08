//! Dedicated, bounded Pub/Sub subscriptions and channel discovery.
use crate::{redis_pubsub_frame::Decoder, redis_settings::Settings, redis_stream::Control};
use eframe::egui;
use std::{
    io::Write,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};
#[derive(Clone)]
pub(crate) enum Request {
    Subscribe { target: Vec<u8>, pattern: bool },
    Channels(Vec<u8>),
    Count(Vec<u8>),
}
impl Request {
    fn command(&self) -> Result<redis::Cmd, String> {
        let target = match self {
            Self::Subscribe { target, .. } | Self::Channels(target) | Self::Count(target) => target,
        };
        if target.is_empty() || target.len() > 4096 {
            return Err("Channel or pattern must contain 1–4,096 bytes.".into());
        }
        let mut command = match self {
            Self::Subscribe { pattern, .. } => {
                redis::cmd(if *pattern { "PSUBSCRIBE" } else { "SUBSCRIBE" })
            }
            Self::Channels(_) => {
                let mut c = redis::cmd("PUBSUB");
                c.arg("CHANNELS");
                c
            }
            Self::Count(_) => {
                let mut c = redis::cmd("PUBSUB");
                c.arg("NUMSUB");
                c
            }
        };
        command.arg(target);
        Ok(command)
    }
}
pub(crate) struct Message {
    pub channel: Vec<u8>,
    pub preview: Vec<u8>,
    pub bytes: usize,
}
pub(crate) enum Event {
    Message(Message),
    Channels {
        channels: Vec<Vec<u8>>,
        labels: Vec<String>,
    },
    Count(u64),
}
#[derive(Default)]
struct Shared {
    control: Control,
    done: AtomicBool,
    busy: AtomicBool,
    dropped: AtomicU64,
    status: Mutex<String>,
}
pub(crate) struct Worker {
    shared: Arc<Shared>,
    pub receiver: mpsc::Receiver<Event>,
    requests: mpsc::SyncSender<Request>,
    query_worker: bool,
}
impl Worker {
    #[cfg(test)]
    pub fn start(
        settings: Settings,
        password: String,
        request: Request,
        ctx: egui::Context,
    ) -> Self {
        Self::start_with_sentinel(settings, password, String::new(), request, ctx)
    }
    pub fn start_with_sentinel(
        settings: Settings,
        password: String,
        sentinel_password: String,
        request: Request,
        ctx: egui::Context,
    ) -> Self {
        let query_worker = !matches!(request, Request::Subscribe { .. });
        let (requests, jobs) = mpsc::sync_channel(1);
        let password = zeroize::Zeroizing::new(password);
        let sentinel_password = zeroize::Zeroizing::new(sentinel_password);
        let (sender, receiver) = mpsc::sync_channel(32);
        let shared = Arc::new(Shared::default());
        *shared.status.lock().unwrap() = "Connecting…".into();
        shared.busy.store(true, Ordering::Release);
        let worker = shared.clone();
        std::thread::spawn(move || {
            let result = run(&settings, (&password, &sentinel_password), request, &worker, &sender, &jobs, &ctx);
            let status = if worker.control.cancelled() {
                "Stopped".into()
            } else {
                result.unwrap_or_else(|error| format!("Stopped: {error}"))
            };
            *worker.status.lock().unwrap() = status;
            worker.control.release();
            worker.done.store(true, Ordering::Release);
            ctx.request_repaint();
        });
        Self {
            shared,
            receiver,
            requests,
            query_worker,
        }
    }
    pub fn busy(&self) -> bool {
        !self.done() && self.shared.busy.load(Ordering::Acquire)
    }
    pub fn request(&self, request: Request) -> Result<(), String> {
        if !self.query_worker || matches!(request, Request::Subscribe { .. }) {
            return Err("This connection is reserved for subscriptions.".into());
        }
        request.command()?;
        if self.done() || self.shared.control.cancelled() {
            return Err("Channel connection is closed. Refresh to reconnect.".into());
        }
        if self.shared.busy.swap(true, Ordering::AcqRel) {
            return Err("Wait for the current channel query.".into());
        }
        *self.shared.status.lock().unwrap() = "Loading…".into();
        if self.requests.try_send(request).is_err() {
            self.shared.busy.store(false, Ordering::Release);
            return Err("Channel query could not be queued.".into());
        }
        Ok(())
    }
    pub fn stop(&self) {
        self.shared.control.stop();
    }
    pub fn done(&self) -> bool {
        self.shared.done.load(Ordering::Acquire)
    }
    pub fn status(&self) -> String {
        self.shared.status.lock().unwrap().clone()
    }
    pub fn dropped(&self) -> u64 {
        self.shared.dropped.load(Ordering::Relaxed)
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
    }
}
fn run(
    settings: &Settings,
    passwords: (&str, &str),
    request: Request,
    shared: &Shared,
    sender: &mpsc::SyncSender<Event>,
    jobs: &mpsc::Receiver<Request>,
    ctx: &egui::Context,
) -> Result<String, String> {
    let command = request.command()?;
    let mut reader = shared.control.start(settings, passwords.0, passwords.1, &command)?;
    let mut decoder = Decoder::default();
    let first = decoder.next(&mut reader).map_err(|e| e.to_string())?;
    if !matches!(request, Request::Subscribe { .. }) {
        let mut request = request;
        let mut response = first;
        loop {
            let (event, status) = query_reply(&request, response)?;
            sender
                .try_send(event)
                .map_err(|_| "Channel viewer closed")?;
            *shared.status.lock().unwrap() = status.into();
            shared.busy.store(false, Ordering::Release);
            ctx.request_repaint();
            request = loop {
                if shared.control.cancelled() {
                    return Ok("Stopped".into());
                }
                match jobs.recv_timeout(Duration::from_millis(200)) {
                    Ok(request) => break request,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => return Ok("Stopped".into()),
                }
            };
            // Established channel-inspection connection: one exchange, no
            // health probe, reauthentication, SELECT, or automatic retry.
            reader
                .get_mut()
                .write_all(&request.command()?.get_packed_command())
                .map_err(|e| e.to_string())?;
            response = decoder.next(&mut reader).map_err(|e| e.to_string())?;
        }
    }
    match &request {
        Request::Channels(_) | Request::Count(_) => unreachable!(),
        Request::Subscribe { target, pattern } => {
            let expected = if *pattern {
                b"psubscribe".as_slice()
            } else {
                b"subscribe".as_slice()
            };
            if first
                != redis::Value::Array(vec![
                    redis::Value::BulkString(expected.to_vec()),
                    redis::Value::BulkString(target.clone()),
                    redis::Value::Int(1),
                ])
            {
                return Err("Invalid subscription acknowledgement".into());
            }
        }
    }
    *shared.status.lock().unwrap() = "Subscribed".into();
    ctx.request_repaint();
    shared.control.read_timeout(Duration::from_millis(200))?;
    loop {
        if shared.control.cancelled() {
            return Ok("Stopped".into());
        }
        let value = match decoder.next(&mut reader) {
            Ok(value) => value,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::Interrupted
                ) =>
            {
                continue;
            }
            Err(error) => return Err(error.to_string()),
        };
        let message = message(value, &request)?;
        match sender.try_send(Event::Message(message)) {
            Ok(()) => ctx.request_repaint_after(Duration::from_millis(100)),
            Err(mpsc::TrySendError::Full(_)) => {
                shared.dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(mpsc::TrySendError::Disconnected(_)) => return Ok("Stopped".into()),
        }
    }
}
fn query_reply(request: &Request, value: redis::Value) -> Result<(Event, &'static str), String> {
    let redis::Value::Array(values) = value else {
        return Err("Invalid channel query response".into());
    };
    match request {
        Request::Channels(_) => {
            let mut channels = Vec::with_capacity(values.len());
            for value in values {
                let redis::Value::BulkString(channel) = value else {
                    return Err("Invalid channel name".into());
                };
                if channel.len() > 4096 {
                    return Err("A channel name exceeds 4 KiB; narrow the pattern.".into());
                }
                channels.push(channel);
            }
            channels.sort();
            let labels = channels
                .iter()
                .map(|c| crate::redis_commands::quote(c))
                .collect();
            Ok((
                Event::Channels { channels, labels },
                "Channels loaded · active exact subscriptions on this node",
            ))
        }
        Request::Count(channel) => {
            if values.len() != 2 || values[0] != redis::Value::BulkString(channel.clone()) {
                return Err("Invalid subscriber count channel".into());
            }
            let redis::Value::Int(count) = values[1] else {
                return Err("Invalid subscriber count".into());
            };
            Ok((
                Event::Count(u64::try_from(count).map_err(|_| "Invalid subscriber count")?),
                "Count loaded · excludes pattern subscriptions",
            ))
        }
        Request::Subscribe { .. } => Err("Unexpected subscription query".into()),
    }
}

fn message(value: redis::Value, request: &Request) -> Result<Message, String> {
    let redis::Value::Array(mut values) = value else {
        return Err("Invalid Pub/Sub message".into());
    };
    let Request::Subscribe { target, pattern } = request else {
        return Err("Not a subscription".into());
    };
    let kind = if *pattern {
        b"pmessage".as_slice()
    } else {
        b"message".as_slice()
    };
    if values.len() != if *pattern { 4 } else { 3 }
        || !matches!(&values[0], redis::Value::BulkString(v) if v.as_slice() == kind)
        || !matches!(&values[1], redis::Value::BulkString(v) if v == target)
    {
        return Err("Unexpected Pub/Sub subscription message".into());
    }
    let redis::Value::BulkString(payload) = values.pop().unwrap() else {
        return Err("Invalid Pub/Sub payload".into());
    };
    let redis::Value::BulkString(channel) = values.pop().unwrap() else {
        return Err("Invalid Pub/Sub channel".into());
    };
    if channel.len() > 4096 {
        return Err("Pub/Sub channel exceeds 4 KiB; subscription stopped".into());
    }
    let bytes = payload.len();
    let preview = payload[..payload.len().min(4096)].to_vec();
    Ok(Message {
        channel,
        preview,
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn messages_preserve_binary_names_and_cap_previews() {
        let target = b"events:*".to_vec();
        let message = message(
            redis::Value::Array(vec![
                redis::Value::BulkString(b"pmessage".to_vec()),
                redis::Value::BulkString(target.clone()),
                redis::Value::BulkString(b"events:\0\xff".to_vec()),
                redis::Value::BulkString(vec![255; 8192]),
            ]),
            &Request::Subscribe {
                target,
                pattern: true,
            },
        )
        .unwrap();
        assert_eq!(message.channel, b"events:\0\xff");
        assert_eq!(message.preview.len(), 4096);
        assert!(message.preview.capacity() <= 4096);
        assert_eq!(message.bytes, 8192);
        assert!(Request::Channels(vec![0; 4097]).command().is_err());
        assert!(Request::Count(Vec::new()).command().is_err());
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    use std::{io::Read, net::TcpListener, time::Instant};
    fn wait(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(4);
        while !condition() {
            assert!(Instant::now() < deadline, "Pub/Sub worker did not settle");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    fn startup_is_pipelined_queue_is_bounded_and_stop_closes_socket() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let peer = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut parser = redis::Parser::new();
            let auth: Vec<String> =
                redis::from_redis_value(parser.parse_value(&mut socket).unwrap()).unwrap();
            let subscribe: Vec<String> =
                redis::from_redis_value(parser.parse_value(&mut socket).unwrap()).unwrap();
            assert_eq!(auth, ["AUTH", "user", "fixture-only"]);
            assert_eq!(subscribe, ["SUBSCRIBE", "chan"]);
            socket
                .write_all(b"+OK\r\n*3\r\n$9\r\nsubscribe\r\n$4\r\nchan\r\n:1\r\n")
                .unwrap();
            for _ in 0..100 {
                socket
                    .write_all(b"*3\r\n$7\r\nmessage\r\n$4\r\nchan\r\n$3\r\n\0\xffx\r\n")
                    .unwrap();
            }
            let mut byte = [0];
            assert_eq!(socket.read(&mut byte).unwrap(), 0);
        });
        let worker = Worker::start(
            Settings {
                host: "127.0.0.1".into(),
                port,
                tls: false,
                username: "user".into(),
                ..Default::default()
            },
            "fixture-only".into(),
            Request::Subscribe {
                target: b"chan".to_vec(),
                pattern: false,
            },
            egui::Context::default(),
        );
        wait(|| worker.dropped() == 68);
        assert_eq!(worker.receiver.try_iter().count(), 32);
        let start = Instant::now();
        worker.stop();
        wait(|| worker.done());
        assert!(start.elapsed() < Duration::from_millis(500));
        assert_eq!(worker.status(), "Stopped");
        peer.join().unwrap();
    }
    #[test]
    fn channel_queries_reuse_transport_without_preflight_or_reauthentication() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let peer = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut parser = redis::Parser::new();
            let auth: Vec<String> =
                redis::from_redis_value(parser.parse_value(&mut socket).unwrap()).unwrap();
            let channels: Vec<String> =
                redis::from_redis_value(parser.parse_value(&mut socket).unwrap()).unwrap();
            assert_eq!(auth, ["AUTH", "fixture-only"]);
            assert_eq!(channels, ["PUBSUB", "CHANNELS", "events:*"]);
            socket
                .write_all(b"+OK\r\n*1\r\n$11\r\nevents:demo\r\n")
                .unwrap();
            for _ in 0..2 {
                let count: Vec<String> =
                    redis::from_redis_value(parser.parse_value(&mut socket).unwrap()).unwrap();
                assert_eq!(count, ["PUBSUB", "NUMSUB", "events:demo"]);
                socket
                    .write_all(b"*2\r\n$11\r\nevents:demo\r\n:3\r\n")
                    .unwrap();
            }
            let mut byte = [0];
            assert_eq!(socket.read(&mut byte).unwrap(), 0);
        });
        let worker = Worker::start(
            Settings {
                host: "127.0.0.1".into(),
                port,
                tls: false,
                ..Default::default()
            },
            "fixture-only".into(),
            Request::Channels(b"events:*".to_vec()),
            egui::Context::default(),
        );
        wait(|| !worker.busy());
        assert!(!worker.done(), "{}", worker.status());
        assert!(matches!(
            worker.receiver.try_recv().unwrap(),
            Event::Channels { .. }
        ));
        for _ in 0..2 {
            worker
                .request(Request::Count(b"events:demo".to_vec()))
                .unwrap();
            wait(|| !worker.busy());
            assert!(matches!(
                worker.receiver.try_recv().unwrap(),
                Event::Count(3)
            ));
        }
        worker.stop();
        wait(|| worker.done());
        peer.join().unwrap();
    }
    #[test]
    fn publish_guards_run_before_network_and_each_publish_is_one_exchange() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let peer = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut parser = redis::Parser::new();
            loop {
                let args: Vec<Vec<u8>> =
                    redis::from_redis_value(parser.parse_value(&mut stream).unwrap()).unwrap();
                stream
                    .write_all(if args[0] == b"PING" {
                        b"+PONG\r\n"
                    } else {
                        b"+OK\r\n"
                    })
                    .unwrap();
                if args[0] == b"SELECT" {
                    break;
                }
            }
            for _ in 0..2 {
                let args: Vec<Vec<u8>> =
                    redis::from_redis_value(parser.parse_value(&mut stream).unwrap()).unwrap();
                assert_eq!(
                    args,
                    vec![
                        b"PUBLISH".to_vec(),
                        b"c\0\xff".to_vec(),
                        b"\0\r\n\xff".to_vec()
                    ]
                );
                stream.write_all(b":2\r\n").unwrap();
            }
            // A failed third write must never be automatically replayed.
            let args: Vec<Vec<u8>> =
                redis::from_redis_value(parser.parse_value(&mut stream).unwrap()).unwrap();
            assert_eq!(args[0], b"PUBLISH");
        });
        let mut session = crate::redis_session::Session::connect(
            &Settings {
                host: "127.0.0.1".into(),
                port,
                tls: false,
                ..Default::default()
            },
            "",
        )
        .unwrap();
        assert!(
            session
                .publish(b"c", b"x", true)
                .unwrap_err()
                .contains("Read-only")
        );
        assert!(session.publish(b"c", &vec![0; 65537], false).is_err());
        for _ in 0..2 {
            assert_eq!(
                session.publish(b"c\0\xff", b"\0\r\n\xff", false).unwrap(),
                2
            );
        }
        assert!(
            session
                .publish(b"c", b"x", false)
                .unwrap_err()
                .contains("may have completed")
        );
        assert!(!session.is_connected());
        peer.join().unwrap();
    }
    #[test]
    #[ignore = "disposable Redis plain/TLS fixtures via scripts/test-redis.sh"]
    fn live_subscriptions_discovery_publish_and_tls_identity() {
        let password = "rustable-redis-test-only";
        for tls in [false, true] {
            let settings = Settings {
                host: "127.0.0.1".into(),
                port: crate::redis_settings::fixture_port(tls),
                tls,
                root_certificate: Some(
                    std::env::var_os("RUSTABLE_REDIS_TEST_CA")
                        .expect("fixture CA")
                        .into(),
                ),
                ..Default::default()
            };
            let channel = b"rustable:pubsub:test\0\xff".to_vec();
            let exact = Worker::start(
                settings.clone(),
                password.into(),
                Request::Subscribe {
                    target: channel.clone(),
                    pattern: false,
                },
                egui::Context::default(),
            );
            let pattern = Worker::start(
                settings.clone(),
                password.into(),
                Request::Subscribe {
                    target: b"rustable:pubsub:*".to_vec(),
                    pattern: true,
                },
                egui::Context::default(),
            );
            for worker in [&exact, &pattern] {
                wait(|| worker.status() == "Subscribed" || worker.done());
                assert!(!worker.done(), "{}", worker.status());
            }
            let mut session = crate::redis_session::Session::connect(&settings, password).unwrap();
            assert!(session.publish(&channel, b"guarded", true).is_err());
            assert_eq!(session.publish(&channel, b"\0\r\n\xff", false).unwrap(), 2);
            for worker in [&exact, &pattern] {
                let mut received = None;
                wait(|| {
                    received = worker.receiver.try_recv().ok();
                    received.is_some()
                });
                let Event::Message(message) = received.unwrap() else {
                    panic!()
                };
                assert_eq!(message.channel, channel);
                assert_eq!(message.preview, b"\0\r\n\xff");
            }
            let channels = Worker::start(
                settings.clone(),
                password.into(),
                Request::Channels(b"rustable:pubsub:*".to_vec()),
                egui::Context::default(),
            );
            wait(|| !channels.busy());
            let Event::Channels {
                channels: listed, ..
            } = channels.receiver.try_recv().unwrap()
            else {
                panic!()
            };
            assert!(listed.contains(&channel));
            let count = Worker::start(
                settings.clone(),
                password.into(),
                Request::Count(channel.clone()),
                egui::Context::default(),
            );
            wait(|| !count.busy());
            assert!(matches!(
                count.receiver.try_recv().unwrap(),
                Event::Count(1)
            ));
            exact.stop();
            pattern.stop();
            wait(|| exact.done() && pattern.done());
            if tls {
                for config in [
                    Settings {
                        root_certificate: None,
                        ..settings.clone()
                    },
                    Settings {
                        host: "localhost".into(),
                        ..settings.clone()
                    },
                ] {
                    let failed = Worker::start(
                        config,
                        password.into(),
                        Request::Count(channel.clone()),
                        egui::Context::default(),
                    );
                    wait(|| failed.done());
                    assert!(failed.status().contains("TLS"), "{}", failed.status());
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) fn fixture_worker() -> (Worker, mpsc::SyncSender<Event>) {
    let (sender, receiver) = mpsc::sync_channel(32);
    let (requests, _) = mpsc::sync_channel(1);
    let shared = Arc::new(Shared::default());
    *shared.status.lock().unwrap() = "Subscribed".into();
    (
        Worker {
            shared,
            receiver,
            requests,
            query_worker: false,
        },
        sender,
    )
}
