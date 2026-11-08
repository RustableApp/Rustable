//! Shared, cancellable native transport for dedicated Redis streaming connections.
use crate::redis_settings::Settings;
use std::{
    io::{Read, Write},
    net::{Shutdown, TcpStream, ToSocketAddrs},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
pub(crate) trait Transport: Read + Write + Send {}
impl<T: Read + Write + Send> Transport for T {}
#[derive(Default)]
pub(crate) struct Control {
    cancelled: AtomicBool,
    socket: Mutex<Option<TcpStream>>,
}
impl Control {
    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    pub fn stop(&self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(socket) = self.socket.lock().unwrap().as_ref() {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }
    pub fn release(&self) {
        self.socket.lock().unwrap().take();
    }
    pub fn read_timeout(&self, duration: Duration) -> Result<(), String> {
        self.socket
            .lock()
            .unwrap()
            .as_ref()
            .ok_or("Redis stream is closed")?
            .set_read_timeout(Some(duration))
            .map_err(|e| e.to_string())
    }
    /// Start only read-only streaming operations; never accepts application writes.
    pub fn start(
        &self,
        settings: &Settings,
        password: &str,
        sentinel_password: &str,
        command: &redis::Cmd,
    ) -> Result<std::io::BufReader<Box<dyn Transport>>, String> {
        let name = command.args_iter().next();
        if !matches!(
            name,
            Some(redis::Arg::Simple(
                b"MONITOR" | b"SUBSCRIBE" | b"PSUBSCRIBE" | b"PUBSUB"
            ))
        ) {
            return Err("Unsupported streaming operation".into());
        }
        let open = |resolved: &Settings, verify: bool| {
            use zeroize::Zeroize;
            let mut reader = std::io::BufReader::with_capacity(8192, self.connect(resolved).map_err(|e| format!("Redis startup transport: {e}"))?);
            let authenticated = !password.is_empty() || !resolved.username.is_empty();
            let mut packed = zeroize::Zeroizing::new(Vec::new());
            if authenticated {
                let mut auth = redis::cmd("AUTH");
                if !resolved.username.is_empty() {
                    auth.arg(&resolved.username);
                }
                auth.arg(password);
                packed.extend(auth.get_packed_command());
            }
            if verify {
                packed.extend(redis::cmd("ROLE").get_packed_command());
            }
            packed.extend(command.get_packed_command());
            reader
                .get_mut()
                .write_all(&packed)
                .map_err(|e| format!("Redis startup transport: {e}"))?;
            packed.zeroize();
            if authenticated && startup_reply(&mut reader)? != redis::Value::Okay {
                return Err(String::from("Unexpected authentication response"));
            }
            if verify && !crate::redis_sentinel::is_primary(&startup_reply(&mut reader)?) {
                return Ok(None);
            }
            Ok(Some(reader))
        };
        settings.validate()?;
        if settings.cluster && !settings.cluster_seeds.is_empty() {
            use std::io::BufRead;
            let deadline = Instant::now() + settings.connect_timeout;
            let mut last = String::from("Cluster stream startup deadline reached");
            let addresses = std::iter::once((settings.host.as_str(), settings.port)).chain(settings.cluster_seeds.iter().map(|seed| (seed.host.as_str(), seed.port)));
            for (index, (host, port)) in addresses.enumerate() {
                if self.cancelled() { return Err("Stopped".into()); }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() { break; }
                let budget = remaining / (settings.cluster_seeds.len() + 1 - index) as u32;
                let candidate = Settings { host: host.into(), port, cluster_seeds: vec![], connect_timeout: budget, read_timeout: settings.read_timeout.min(budget), ..settings.clone() };
                let result = open(&candidate, false).and_then(|reader| {
                    let mut reader = reader.ok_or("Redis stream unavailable")?;
                    // Preserve the command reply for the existing parser. A proxy may
                    // accept TCP but close before Redis returns any startup bytes.
                    if reader.fill_buf().map_err(|e| format!("Redis startup transport: {e}"))?.is_empty() {
                        return Err("Redis closed during startup".into());
                    }
                    Ok(reader)
                });
                match result {
                    Ok(reader) => {
                        if self.cancelled() { self.release(); return Err("Stopped".into()); }
                        if Instant::now() >= deadline { self.release(); last = "Cluster stream startup deadline reached after response".into(); break; }
                        let slot = self.socket.lock().unwrap();
                        let socket = slot.as_ref().ok_or("Redis stream is closed")?;
                        socket.set_read_timeout(Some(settings.read_timeout)).map_err(|e| e.to_string())?;
                        socket.set_write_timeout(Some(settings.read_timeout)).map_err(|e| e.to_string())?;
                        return Ok(reader);
                    }
                    Err(error) => {
                        self.release();
                        if error != "Redis closed during startup" && !error.starts_with("Redis startup transport: ") { return Err(error); }
                        last = error;
                    }
                }
            }
            return Err(format!("No configured Cluster stream seed started: {last}"));
        }
        if settings.sentinel.is_some() {
            crate::redis_sentinel::with_primary(
                settings,
                sentinel_password,
                || self.cancelled(),
                |resolved, deadline| {
                    let result = open(resolved, true)?;
                    crate::redis_sentinel::remaining(deadline)?;
                    if result.is_some() {
                        self.socket
                            .lock()
                            .unwrap()
                            .as_ref()
                            .ok_or("Redis stream is closed")?
                            .set_write_timeout(Some(settings.read_timeout))
                            .map_err(|e| e.to_string())?;
                        self.read_timeout(settings.read_timeout)?;
                    }
                    Ok(result)
                },
            )
        } else {
            open(settings, false)?.ok_or_else(|| "Redis stream unavailable".into())
        }
    }
    pub fn connect(&self, settings: &Settings) -> Result<Box<dyn Transport>, String> {

        settings.validate()?;
        if settings.sentinel.is_some() {
            return Err("Sentinel connections require the discovery credential path.".into());
        }
        // DNS and TLS stay on this worker. Stop during setup is remembered; a socket
        // is registered before TLS, so cancellation also interrupts the handshake.
        let addresses = (settings.host.as_str(), settings.port)
            .to_socket_addrs()
            .map_err(|e| e.to_string())?;
        let deadline = Instant::now() + settings.connect_timeout;
        let mut connected = None;
        let mut last_error = "No Redis addresses resolved".to_string();
        for address in addresses {
            if self.cancelled.load(Ordering::Acquire) {
                return Err("Stopped".into());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match TcpStream::connect_timeout(&address, remaining) {
                Ok(socket) => {
                    connected = Some(socket);
                    break;
                }
                Err(error) => last_error = error.to_string(),
            }
        }
        let socket = connected.ok_or(last_error)?;
        socket
            .set_read_timeout(Some(settings.read_timeout))
            .map_err(|e| e.to_string())?;
        socket
            .set_write_timeout(Some(settings.read_timeout))
            .map_err(|e| e.to_string())?;
        {
            let mut slot = self.socket.lock().unwrap();
            *slot = Some(socket.try_clone().map_err(|e| e.to_string())?);
            if self.cancelled.load(Ordering::Acquire) {
                return Err("Stopped".into());
            }
        }
        let stream: Box<dyn Transport> = if settings.tls {
            let mut builder = native_tls::TlsConnector::builder();
            if let Some(path) = &settings.root_certificate {
                let pem = std::fs::read(path)
                    .map_err(|e| format!("Cannot read Redis CA certificate: {e}"))?;
                builder.add_root_certificate(
                    native_tls::Certificate::from_pem(&pem).map_err(|e| e.to_string())?,
                );
            }
            let connector = builder.build().map_err(|e| e.to_string())?;
            Box::new(
                connector
                    .connect(&settings.host, socket)
                    .map_err(|e| format!("Redis TLS: {e}"))?,
            )
        } else {
            Box::new(socket)
        };
        Ok(stream)
    }
}

/// Bounded RESP2 startup parser. Leaves all following ACK/message bytes in reader.
fn startup_reply(reader: &mut impl std::io::BufRead) -> Result<redis::Value, String> {
    fn parse(
        reader: &mut impl std::io::BufRead,
        bytes: &mut usize,
        nodes: &mut usize,
        depth: usize,
    ) -> Result<redis::Value, String> {
        if depth > 4 || *nodes == 0 {
            return Err("Redis startup response exceeds nesting/item limit".into());
        }
        *nodes -= 1;
        let mut line = Vec::new();
        loop {
            let available = reader.fill_buf().map_err(|e| format!("Redis startup transport: {e}"))?;
            if available.is_empty() {
                return Err("Redis closed during startup".into());
            }
            let take = available
                .iter()
                .position(|b| *b == b'\n')
                .map_or(available.len(), |n| n + 1);
            if take > *bytes || line.len() + take > 4096 {
                return Err("Redis startup response exceeds size limit".into());
            }
            line.extend_from_slice(&available[..take]);
            reader.consume(take);
            *bytes -= take;
            if line.last() == Some(&b'\n') {
                break;
            }
        }
        if !line.ends_with(b"\r\n") || line.len() < 3 {
            return Err("Invalid Redis startup response".into());
        }
        let body = &line[1..line.len() - 2];
        let number = || {
            std::str::from_utf8(body)
                .ok()
                .and_then(|s| s.parse::<i64>().ok())
                .ok_or_else(|| "Invalid Redis startup length/integer".to_string())
        };
        match line[0] {
            b'+' if body == b"OK" => Ok(redis::Value::Okay),
            b'+' => Ok(redis::Value::SimpleString(
                String::from_utf8_lossy(body).into_owned(),
            )),
            b'-' => Err(format!("Redis startup: {}", String::from_utf8_lossy(body))),
            b':' => Ok(redis::Value::Int(number()?)),
            b'$' => {
                let n = number()?;
                if n == -1 {
                    return Ok(redis::Value::Nil);
                }
                let n = usize::try_from(n).map_err(|_| "Invalid Redis bulk length")?;
                if n > bytes.saturating_sub(2) {
                    return Err("Redis startup response exceeds size limit".into());
                }
                let mut value = vec![0; n + 2];
                reader.read_exact(&mut value).map_err(|e| format!("Redis startup transport: {e}"))?;
                *bytes -= n + 2;
                if !value.ends_with(b"\r\n") {
                    return Err("Invalid Redis bulk terminator".into());
                }
                value.truncate(n);
                Ok(redis::Value::BulkString(value))
            }
            b'*' => {
                let n = number()?;
                if n == -1 {
                    return Ok(redis::Value::Nil);
                }
                let n = usize::try_from(n).map_err(|_| "Invalid Redis array length")?;
                if n > *nodes {
                    return Err("Redis startup response exceeds item limit".into());
                }
                let mut values = Vec::with_capacity(n);
                for _ in 0..n {
                    values.push(parse(reader, bytes, nodes, depth + 1)?);
                }
                Ok(redis::Value::Array(values))
            }
            _ => Err("Invalid Redis startup response type".into()),
        }
    }
    parse(reader, &mut 65_536, &mut 1000, 0)
}

#[cfg(test)]
mod sentinel_guard_tests {
    use super::*;
    #[test]
    fn cluster_stream_fallback_preserves_startup_pipeline_and_cancellation() {
        use std::{io::Write, net::TcpListener};
        for (name, authenticated) in ["MONITOR", "SUBSCRIBE", "PSUBSCRIBE", "PUBSUB"].into_iter().flat_map(|name| [true, false].map(|auth| (name, auth))) {
            let dead = TcpListener::bind("127.0.0.1:0").unwrap();
            let dead_port = dead.local_addr().unwrap().port();
            let live = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = live.local_addr().unwrap().port();
            let closed_proxy = std::thread::spawn(move || {
                let (mut socket, _) = dead.accept().unwrap();
                socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut parser = redis::Parser::new();
                for _ in 0..(if authenticated { 2 } else { 1 }) { parser.parse_value(&mut socket).unwrap(); }
                // A TCP proxy accepts but Redis cannot supply an AUTH/command reply.
            });
            let peer = std::thread::spawn(move || {
                let (mut socket, _) = live.accept().unwrap();
                socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut parser = redis::Parser::new();
                if authenticated {
                    let auth: Vec<Vec<u8>> = redis::from_redis_value(parser.parse_value(&mut socket).unwrap()).unwrap();
                    assert_eq!(auth, [b"AUTH".to_vec(), b"fixture-password".to_vec()]);
                }
                let command: Vec<Vec<u8>> = redis::from_redis_value(parser.parse_value(&mut socket).unwrap()).unwrap();
                assert_eq!(command[0], name.as_bytes());
                // Both requests must arrive before either reply, with no preflight.
                socket.write_all(if authenticated { b"+OK\r\n+OK\r\n" } else { b"+OK\r\n" }).unwrap();
            });
            let settings = Settings { host: "127.0.0.1".into(), port: dead_port, cluster: true, tls: false,
                cluster_seeds: vec![crate::redis_settings::SentinelSeed { host: "127.0.0.1".into(), port }], ..Default::default() };
            let control = Control::default();
            let mut command = redis::cmd(name);
            if name != "MONITOR" { command.arg(if name == "PUBSUB" { "CHANNELS" } else { "channel" }); }
            let mut reader = control.start(&settings, if authenticated { "fixture-password" } else { "" }, "", &command).unwrap();
            assert_eq!(startup_reply(&mut reader).unwrap(), redis::Value::Okay);
            peer.join().unwrap();
            closed_proxy.join().unwrap();
        }
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let settings = Settings { host: "127.0.0.1".into(), port: listener.local_addr().unwrap().port(), cluster: true, tls: false,
            cluster_seeds: vec![crate::redis_settings::SentinelSeed { host: "127.0.0.1".into(), port: 1 }], ..Default::default() };
        let control = Control::default();
        control.stop();
        assert!(control.start(&settings, "", "", &redis::cmd("MONITOR")).err().unwrap().contains("Stopped"));
        assert_eq!(listener.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
    }
    #[test]
    fn cluster_stream_alternate_never_downgrades_tls() {
        use std::{io::Read, net::TcpListener};
        let dead = TcpListener::bind("127.0.0.1:0").unwrap();
        let dead_port = dead.local_addr().unwrap().port();
        let alternate = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = alternate.local_addr().unwrap().port();
        drop(dead);
        let peer = std::thread::spawn(move || {
            let (mut socket, _) = alternate.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut header = [0; 1];
            socket.read_exact(&mut header).unwrap();
            assert_eq!(header[0], 0x16, "alternate must receive TLS handshake, never plaintext AUTH");
        });
        let settings = Settings { host: "127.0.0.1".into(), port: dead_port, cluster: true, tls: true,
            cluster_seeds: vec![crate::redis_settings::SentinelSeed { host: "127.0.0.1".into(), port }], ..Default::default() };
        assert!(Control::default().start(&settings, "secret", "", &redis::cmd("MONITOR")).err().unwrap().contains("TLS"));
        peer.join().unwrap();
    }
    #[test]
    fn cluster_stream_authentication_errors_do_not_try_other_seeds() {
        use std::{io::Write, net::TcpListener};
        let first = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = first.local_addr().unwrap().port();
        let alternate = TcpListener::bind("127.0.0.1:0").unwrap();
        alternate.set_nonblocking(true).unwrap();
        let other_port = alternate.local_addr().unwrap().port();
        let peer = std::thread::spawn(move || {
            let (mut socket, _) = first.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut parser = redis::Parser::new();
            for _ in 0..2 { parser.parse_value(&mut socket).unwrap(); }
            socket.write_all(b"-WRONGPASS denied\r\n-NOAUTH denied\r\n").unwrap();
        });
        let settings = Settings { host: "127.0.0.1".into(), port, cluster: true, tls: false,
            cluster_seeds: vec![crate::redis_settings::SentinelSeed { host: "127.0.0.1".into(), port: other_port }], ..Default::default() };
        assert!(Control::default().start(&settings, "bad", "", &redis::cmd("MONITOR")).is_err());
        assert_eq!(alternate.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
        peer.join().unwrap();
    }
    #[test]
    fn startup_frames_are_bounded_and_preserve_following_bytes() {
        for input in [
            b"*1001\r\n".as_slice(),
            b"$65535\r\n",
            b"*-2\r\n",
            b"*1\r\n*1\r\n*1\r\n*1\r\n*1\r\n+OK\r\n",
            b"$3\r\nabcXX",
            b"+OK\n",
        ] {
            assert!(startup_reply(&mut std::io::BufReader::new(input)).is_err());
        }
        let mut reader = std::io::BufReader::new(b"+OK\r\n+ACK\r\n".as_slice());
        assert_eq!(startup_reply(&mut reader).unwrap(), redis::Value::Okay);
        assert_eq!(
            startup_reply(&mut reader).unwrap(),
            redis::Value::SimpleString("ACK".into())
        );
        let control = Control::default();
        assert!(
            control
                .start(&Settings::default(), "", "", &redis::cmd("SET"))
                .err()
                .unwrap()
                .contains("Unsupported")
        );
        assert!(control.socket.lock().unwrap().is_none());
    }
    #[test]
    fn cancelled_discovery_opens_no_socket() {
        let control = Control::default();
        control.stop();
        let settings = Settings {
            sentinel: Some(Default::default()),
            ..Default::default()
        };
        assert_eq!(
            control
                .start(&settings, "", "", &redis::cmd("MONITOR"))
                .err()
                .unwrap(),
            "Stopped"
        );
        assert!(control.socket.lock().unwrap().is_none());
    }
    #[test]
    fn direct_stream_path_never_treats_a_sentinel_profile_as_a_data_endpoint() {
        let control = Control::default();
        let settings = Settings {
            host: String::new(),
            port: 0,
            sentinel: Some(Default::default()),
            ..Default::default()
        };
        let error = match control.connect(&settings) {
            Ok(_) => panic!("Direct connection bypassed discovery"),
            Err(error) => error,
        };
        assert!(error.contains("Sentinel connections"));
        assert!(control.socket.lock().unwrap().is_none());
    }
}
