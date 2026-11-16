/*
 * [Input] Optional loopback WebSocket connections from allow-listed web pages that host a
 *         virtual (browser-simulated) device; binary or text messages carry raw device bytes.
 * [Output] A serial-port-shaped byte pipe per connection, listed beside physical USB serial
 *          ports, so the unchanged probe/reader/writer path speaks the same JSON Lines to it.
 * [Pos] Optional transport beneath usb_serial.rs. Off unless the user enables it; binds
 *       127.0.0.1 only; one virtual device at a time (a newer page replaces the older one).
 * [Sync] If this file changes, update `pc/.folder.md`.
 */

use super::{connection_handle::SerialPortPairResult, UsbDeviceInfo};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::{
    self,
    handshake::server::{ErrorResponse, Request, Response},
    http::StatusCode,
    Message,
};

pub const DEFAULT_PORT: u16 = 23340;
pub const PATH: &str = "/virtual-device";
pub const DEFAULT_ALLOWED_ORIGINS: [&str; 3] = ["https://thorb.com", "http://localhost:*", "http://127.0.0.1:*"];
const PORT_PREFIX: &str = "virtual-device-";
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const SOCKET_POLL: Duration = Duration::from_millis(5);
const ACCEPT_POLL: Duration = Duration::from_millis(50);

#[derive(Clone, Debug)]
pub struct Config {
    pub port: u16,
    pub allowed_origins: Vec<String>,
}

pub fn is_virtual_port(port_name: &str) -> bool {
    port_name.starts_with(PORT_PREFIX)
}

// ── Origin allow-list ───────────────────────────────────────────────────────
// A rule is `scheme://host` or `scheme://host:port`; `:*` allows any port on
// that host (local development servers). Paths, credentials and queries are
// never part of a browser Origin, so rules containing them are rejected.

struct OriginParts<'a> { scheme: &'a str, host: &'a str, port: Option<&'a str> }

fn split_origin(value: &str) -> Option<OriginParts<'_>> {
    let (scheme, rest) = value.split_once("://")?;
    if !matches!(scheme, "http" | "https") || rest.is_empty()
        || rest.contains(|c: char| matches!(c, '/' | '?' | '#' | '@') || c.is_whitespace()) {
        return None;
    }
    let (host, port) = if rest.starts_with('[') {
        let (host, tail) = rest.split_at(rest.find(']')? + 1);
        (host, if tail.is_empty() { None } else { Some(tail.strip_prefix(':')?) })
    } else {
        match rest.rsplit_once(':') { Some((host, port)) => (host, Some(port)), None => (rest, None) }
    };
    if host.is_empty() || !host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '[' | ']' | ':')) {
        return None;
    }
    if let Some(port) = port {
        if port != "*" && port.parse::<u16>().ok().filter(|p| *p != 0).is_none() { return None; }
    }
    Some(OriginParts { scheme, host, port })
}

/// Validate and canonicalize one allow-list entry from settings.
pub fn normalize_origin_rule(raw: &str) -> Result<String, String> {
    let rule = raw.trim().trim_end_matches('/').to_ascii_lowercase();
    split_origin(&rule).map(|_| rule.clone())
        .ok_or_else(|| format!("来源格式无效：{}（示例 https://thorb.com 或 http://localhost:*）", raw.trim()))
}

pub fn origin_allowed(rules: &[String], origin: &str) -> bool {
    let origin = origin.trim().to_ascii_lowercase();
    let Some(page) = split_origin(&origin).filter(|page| page.port != Some("*")) else { return false; };
    rules.iter().any(|rule| split_origin(rule).is_some_and(|rule| {
        rule.scheme == page.scheme && rule.host == page.host && (rule.port == Some("*") || rule.port == page.port)
    }))
}

// ── Byte pipe shaped like a serial port ─────────────────────────────────────

struct Link {
    name: String,
    origin: String,
    inbound: Mutex<VecDeque<u8>>,
    arrived: Condvar,
    outbound: mpsc::Sender<Vec<u8>>,
    closed: AtomicBool,
}

impl Link {
    fn push(&self, bytes: &[u8]) {
        if let Ok(mut inbound) = self.inbound.lock() { inbound.extend(bytes); }
        self.arrived.notify_all();
    }
    fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.arrived.notify_all();
    }
    fn is_closed(&self) -> bool { self.closed.load(Ordering::Acquire) }
}

#[derive(Clone)]
struct VirtualPort { link: Arc<Link>, baud: u32, timeout: Duration }

impl Read for VirtualPort {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() { return Ok(0); }
        let deadline = Instant::now() + self.timeout;
        let mut inbound = self.link.inbound.lock().map_err(|_| std::io::Error::other("virtual device link poisoned"))?;
        loop {
            if !inbound.is_empty() {
                let count = buf.len().min(inbound.len());
                for (slot, byte) in buf.iter_mut().zip(inbound.drain(..count)) { *slot = byte; }
                return Ok(count);
            }
            // Same contract as a serial driver: EOF only once the peer is gone.
            if self.link.is_closed() { return Ok(0); }
            let now = Instant::now();
            if now >= deadline { return Err(std::io::ErrorKind::TimedOut.into()); }
            inbound = self.link.arrived.wait_timeout(inbound, deadline - now)
                .map_err(|_| std::io::Error::other("virtual device link poisoned"))?.0;
        }
    }
}

impl Write for VirtualPort {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.link.is_closed() || self.link.outbound.send(bytes.to_vec()).is_err() {
            return Err(std::io::ErrorKind::BrokenPipe.into());
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

impl serialport::SerialPort for VirtualPort {
    fn name(&self) -> Option<String> { Some(self.link.name.clone()) }
    // A WebSocket has no line rate; accept whatever the probe selects.
    fn baud_rate(&self) -> serialport::Result<u32> { Ok(self.baud) }
    fn data_bits(&self) -> serialport::Result<serialport::DataBits> { Ok(serialport::DataBits::Eight) }
    fn flow_control(&self) -> serialport::Result<serialport::FlowControl> { Ok(serialport::FlowControl::None) }
    fn parity(&self) -> serialport::Result<serialport::Parity> { Ok(serialport::Parity::None) }
    fn stop_bits(&self) -> serialport::Result<serialport::StopBits> { Ok(serialport::StopBits::One) }
    fn timeout(&self) -> Duration { self.timeout }
    fn set_baud_rate(&mut self, baud_rate: u32) -> serialport::Result<()> { self.baud = baud_rate; Ok(()) }
    fn set_data_bits(&mut self, _: serialport::DataBits) -> serialport::Result<()> { Ok(()) }
    fn set_flow_control(&mut self, _: serialport::FlowControl) -> serialport::Result<()> { Ok(()) }
    fn set_parity(&mut self, _: serialport::Parity) -> serialport::Result<()> { Ok(()) }
    fn set_stop_bits(&mut self, _: serialport::StopBits) -> serialport::Result<()> { Ok(()) }
    fn set_timeout(&mut self, timeout: Duration) -> serialport::Result<()> { self.timeout = timeout; Ok(()) }
    fn write_request_to_send(&mut self, _: bool) -> serialport::Result<()> { Ok(()) }
    fn write_data_terminal_ready(&mut self, _: bool) -> serialport::Result<()> { Ok(()) }
    fn read_clear_to_send(&mut self) -> serialport::Result<bool> { Ok(true) }
    fn read_data_set_ready(&mut self) -> serialport::Result<bool> { Ok(true) }
    fn read_ring_indicator(&mut self) -> serialport::Result<bool> { Ok(false) }
    fn read_carrier_detect(&mut self) -> serialport::Result<bool> { Ok(true) }
    fn bytes_to_read(&self) -> serialport::Result<u32> {
        Ok(self.link.inbound.lock().map(|inbound| inbound.len() as u32).unwrap_or(0))
    }
    fn bytes_to_write(&self) -> serialport::Result<u32> { Ok(0) }
    fn clear(&self, buffer: serialport::ClearBuffer) -> serialport::Result<()> {
        if matches!(buffer, serialport::ClearBuffer::Input | serialport::ClearBuffer::All) {
            if let Ok(mut inbound) = self.link.inbound.lock() { inbound.clear(); }
        }
        Ok(())
    }
    fn try_clone(&self) -> serialport::Result<Box<dyn serialport::SerialPort>> { Ok(Box::new(self.clone())) }
    fn set_break(&self) -> serialport::Result<()> { Ok(()) }
    fn clear_break(&self) -> serialport::Result<()> { Ok(()) }
}

// ── Host: listener lifecycle plus the current link ──────────────────────────

struct Server { addr: SocketAddr, stop: Arc<AtomicBool> }

#[derive(Default)]
pub(super) struct Host {
    current: Mutex<Option<Arc<Link>>>,
    server: Mutex<Option<Server>>,
    next_link: AtomicU64,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub listening: Option<String>,
    pub connected_origin: Option<String>,
}

impl Host {
    pub(super) fn devices(&self) -> Vec<UsbDeviceInfo> {
        let current = self.current.lock().ok().and_then(|current| current.clone());
        current.filter(|link| !link.is_closed()).map(|link| UsbDeviceInfo {
            port_name: link.name.clone(),
            vid: 0,
            pid: 0,
            serial_number: String::new(),
            manufacturer: "Virtual device".into(),
            product: link.origin.clone(),
        }).into_iter().collect()
    }

    pub(super) fn open(&self, port_name: &str, baud: u32) -> SerialPortPairResult {
        let link = self.current.lock().ok().and_then(|current| current.clone())
            .filter(|link| link.name == port_name && !link.is_closed())
            .ok_or_else(|| format!("virtual device {port_name} is no longer connected"))?;
        let port = VirtualPort { link, baud, timeout: Duration::from_millis(100) };
        Ok((Box::new(port.clone()), Box::new(port)))
    }

    pub(super) fn status(&self) -> Status {
        Status {
            listening: self.server.lock().ok().and_then(|s| s.as_ref().map(|s| s.addr.to_string())),
            connected_origin: self.devices().into_iter().next().map(|device| device.product),
        }
    }

    /// Stop any running listener and its link; start a new one when `config` is given.
    pub(super) fn apply(self: &Arc<Self>, config: Option<Config>) -> Result<Option<SocketAddr>, String> {
        let mut server = self.server.lock().map_err(|_| "virtual device host unavailable")?;
        if let Some(old) = server.take() { old.stop.store(true, Ordering::Release); }
        self.replace_link(None);
        let Some(config) = config else { return Ok(None); };
        let rules = config.allowed_origins.iter().map(|rule| normalize_origin_rule(rule))
            .collect::<Result<Vec<_>, _>>()?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, config.port))
            .map_err(|error| format!("虚拟设备端口 127.0.0.1:{} 无法监听：{error}", config.port))?;
        listener.set_nonblocking(true).map_err(|error| error.to_string())?;
        let addr = listener.local_addr().map_err(|error| error.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        let (host, accept_stop) = (Arc::clone(self), Arc::clone(&stop));
        thread::spawn(move || host.accept_loop(listener, rules, accept_stop));
        *server = Some(Server { addr, stop });
        Ok(Some(addr))
    }

    fn replace_link(&self, next: Option<Arc<Link>>) {
        let previous = self.current.lock().ok().and_then(|mut current| std::mem::replace(&mut *current, next));
        if let Some(previous) = previous { previous.close(); }
    }

    fn accept_loop(self: Arc<Self>, listener: TcpListener, rules: Vec<String>, stop: Arc<AtomicBool>) {
        let rules = Arc::new(rules);
        while !stop.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let (host, rules, stop) = (Arc::clone(&self), Arc::clone(&rules), Arc::clone(&stop));
                    thread::spawn(move || host.serve(stream, &rules, stop));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(ACCEPT_POLL),
                Err(error) => { eprintln!("[virtual-device] accept failed: {error}"); thread::sleep(ACCEPT_POLL); }
            }
        }
    }

    fn serve(&self, stream: TcpStream, rules: &[String], stop: Arc<AtomicBool>) {
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_nodelay(true);
        let _ = stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT));
        let mut origin = String::new();
        let check = |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
            let reject = |status: StatusCode| {
                let mut response = ErrorResponse::new(None);
                *response.status_mut() = status;
                response
            };
            if request.uri().path() != PATH { return Err(reject(StatusCode::NOT_FOUND)); }
            let page = request.headers().get("origin").and_then(|value| value.to_str().ok()).unwrap_or("");
            if !origin_allowed(rules, page) { return Err(reject(StatusCode::FORBIDDEN)); }
            origin = page.to_string();
            Ok(response)
        };
        let mut socket = match tungstenite::accept_hdr(stream, check) {
            Ok(socket) => socket,
            Err(_) => return, // rejected origin/path or not a WebSocket: nothing is registered
        };
        if stop.load(Ordering::Acquire) { return; }
        let _ = socket.get_ref().set_read_timeout(Some(SOCKET_POLL));
        let (outbound, outgoing) = mpsc::channel();
        let link = Arc::new(Link {
            name: format!("{PORT_PREFIX}{}", self.next_link.fetch_add(1, Ordering::Relaxed) + 1),
            origin,
            inbound: Mutex::new(VecDeque::new()),
            arrived: Condvar::new(),
            outbound,
            closed: AtomicBool::new(false),
        });
        eprintln!("[virtual-device] {} connected from {}", link.name, link.origin);
        self.replace_link(Some(Arc::clone(&link)));
        pump(&mut socket, &link, &outgoing, &stop);
        link.close();
        let _ = socket.close(None);
        let _ = socket.flush();
        if let Ok(mut current) = self.current.lock() {
            if current.as_ref().is_some_and(|c| Arc::ptr_eq(c, &link)) { *current = None; }
        }
        eprintln!("[virtual-device] {} disconnected", link.name);
    }
}

fn pump(socket: &mut tungstenite::WebSocket<TcpStream>, link: &Link, outgoing: &mpsc::Receiver<Vec<u8>>, stop: &AtomicBool) {
    loop {
        if link.is_closed() || stop.load(Ordering::Acquire) { return; }
        while let Ok(bytes) = outgoing.try_recv() {
            if socket.send(Message::binary(bytes)).is_err() { return; }
        }
        match socket.read() {
            Ok(Message::Binary(bytes)) => link.push(&bytes),
            Ok(Message::Text(text)) => link.push(text.as_bytes()),
            Ok(Message::Close(_)) => return,
            Ok(_) => {}
            Err(tungstenite::Error::Io(error))
                if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| normalize_origin_rule(value).unwrap()).collect()
    }

    #[test]
    fn origin_allow_list_matches_exact_origins_and_wildcard_ports_only() {
        let defaults = rules(&DEFAULT_ALLOWED_ORIGINS);
        for allowed in ["https://thorb.com", "http://localhost:4321", "http://127.0.0.1:4180", "HTTPS://THORB.COM"] {
            assert!(origin_allowed(&defaults, allowed), "{allowed}");
        }
        for rejected in ["", "null", "http://thorb.com", "https://thorb.com.evil.example", "https://evil.example",
            "https://thorb.com:8443", "http://localhost.evil.example:80", "http://192.168.1.2:4321", "file://"] {
            assert!(!origin_allowed(&defaults, rejected), "{rejected}");
        }
    }

    #[test]
    fn origin_rules_reject_paths_credentials_and_bad_ports() {
        assert_eq!(normalize_origin_rule(" https://Shop.Example/ ").unwrap(), "https://shop.example");
        assert_eq!(normalize_origin_rule("http://[::1]:*").unwrap(), "http://[::1]:*");
        for bad in ["thorb.com", "ftp://thorb.com", "https://thorb.com/path", "https://user@thorb.com",
            "https://thorb.com:0", "https://thorb.com:99999", "https://", "https://thorb.com?x=1"] {
            assert!(normalize_origin_rule(bad).is_err(), "{bad}");
        }
    }
}
