//! Virtual device link: a browser-hosted device that speaks the same JSON Lines
//! over a local WebSocket. Tests drive the manager's public surface with a
//! tungstenite client standing in for the browser page.

use super::*;
use std::net::{SocketAddr, TcpStream};
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest, Message};

type Socket = tungstenite::WebSocket<TcpStream>;

fn config(origins: &[&str]) -> virtual_device::Config {
    virtual_device::Config {
        port: 0, // tests take an ephemeral port; the app uses virtual_device::DEFAULT_PORT
        allowed_origins: origins.iter().map(|origin| origin.to_string()).collect(),
    }
}

fn open(addr: SocketAddr, origin: Option<&str>) -> Result<Socket, tungstenite::Error> {
    let mut request = format!("ws://{addr}{}", virtual_device::PATH).into_client_request().unwrap();
    if let Some(origin) = origin {
        request.headers_mut().insert("Origin", origin.parse().unwrap());
    }
    let stream = TcpStream::connect(addr).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    tungstenite::client(request, stream).map(|(socket, _)| socket).map_err(|error| match error {
        tungstenite::HandshakeError::Failure(error) => error,
        tungstenite::HandshakeError::Interrupted(_) => panic!("blocking handshake interrupted"),
    })
}

fn read_line(socket: &mut Socket, pending: &mut Vec<u8>) -> serde_json::Value {
    loop {
        if let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = pending.drain(..=end).collect();
            return serde_json::from_slice(&line).expect("device receives JSON Lines");
        }
        match socket.read().expect("device link stays open") {
            Message::Binary(bytes) => pending.extend_from_slice(&bytes),
            Message::Text(text) => pending.extend_from_slice(text.as_bytes()),
            _ => {}
        }
    }
}

/// Answer the desktop's probe the way the P4 firmware does: bind -> hello.
fn answer_probe(socket: &mut Socket, pending: &mut Vec<u8>, board: &str) {
    loop {
        if read_line(socket, pending)["topic"] == "bind" { break; }
    }
    let hello = serde_json::json!({"topic":"hello","payload":{"boardDeviceId":board,"runtime":"esp-p4","fw":"virtual-test"}});
    socket.send(Message::binary(format!("{hello}\n").into_bytes())).unwrap();
}

fn wait_for_virtual_device(manager: &UsbSerialManager) -> UsbDeviceInfo {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(device) = manager.scan_devices().into_iter().find(|d| virtual_device::is_virtual_port(&d.port_name)) {
            return device;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("virtual device never appeared in the device list");
}

#[test]
fn virtual_device_connects_over_websocket_and_receives_state_pushes() {
    let manager = UsbSerialManager::new();
    manager.set_desktop_device_id("desktop-1");
    let addr = manager.apply_virtual_device_config(Some(config(&["https://thorb.com"]))).unwrap().unwrap();
    let device = thread::spawn(move || {
        let mut socket = open(addr, Some("https://thorb.com")).expect("allowed origin connects");
        let mut pending = Vec::new();
        answer_probe(&mut socket, &mut pending, "p4-virtual-1");
        loop {
            let line = read_line(&mut socket, &mut pending);
            if line["topic"] == "state/claude-code" { return line; }
        }
    });

    let listed = wait_for_virtual_device(&manager);
    manager.connect(&listed.port_name, |_, _| {}).expect("virtual device passes the serial probe");
    let status = manager.status();
    assert!(status.connected);
    assert_eq!(status.board_device_id, "p4-virtual-1");

    manager.send_state("claude-code", &serde_json::json!({"state":"working"})).unwrap();
    let pushed = device.join().unwrap();
    assert_eq!(pushed["payload"]["state"], "working");
}

#[test]
fn turning_the_virtual_device_off_stops_listening_and_drops_the_device() {
    let manager = UsbSerialManager::new();
    assert_eq!(manager.apply_virtual_device_config(None).unwrap(), None, "off by default: nothing listens");
    let addr = manager.apply_virtual_device_config(Some(config(&["https://thorb.com"]))).unwrap().unwrap();
    let _page = open(addr, Some("https://thorb.com")).unwrap();
    wait_for_virtual_device(&manager);

    assert_eq!(manager.apply_virtual_device_config(None).unwrap(), None);
    assert!(manager.virtual_device_status().listening.is_none());
    assert!(!manager.scan_devices().iter().any(|d| virtual_device::is_virtual_port(&d.port_name)));
    let deadline = Instant::now() + Duration::from_secs(2);
    while TcpStream::connect(addr).is_ok() {
        assert!(Instant::now() < deadline, "listener still accepts after being turned off");
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn pages_outside_the_origin_allow_list_are_refused() {
    let manager = UsbSerialManager::new();
    let addr = manager.apply_virtual_device_config(Some(config(&["https://thorb.com"]))).unwrap().unwrap();
    for origin in [Some("https://evil.example"), Some("http://thorb.com"), None] {
        match open(addr, origin) {
            Err(tungstenite::Error::Http(response)) => assert_eq!(response.status(), 403, "{origin:?}"),
            other => panic!("{origin:?} was not refused: {:?}", other.map(|_| ())),
        }
    }
    thread::sleep(Duration::from_millis(50));
    assert!(!manager.scan_devices().iter().any(|d| virtual_device::is_virtual_port(&d.port_name)));
    assert!(manager.virtual_device_status().connected_origin.is_none());
}

#[test]
fn virtual_device_listens_on_the_loopback_address_only() {
    let manager = UsbSerialManager::new();
    let addr = manager.apply_virtual_device_config(Some(config(&["https://thorb.com"]))).unwrap().unwrap();
    assert_eq!(addr.ip(), std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
    // Every non-loopback address of this machine must refuse the port.
    let lan = std::net::UdpSocket::bind("0.0.0.0:0").and_then(|socket| {
        socket.connect("192.0.2.1:9")?; // TEST-NET-1, nothing is sent
        socket.local_addr()
    });
    if let Ok(lan) = lan.map(|local| local.ip()).map(|ip| SocketAddr::new(ip, addr.port())) {
        if !lan.ip().is_loopback() && !lan.ip().is_unspecified() {
            assert!(TcpStream::connect_timeout(&lan, Duration::from_millis(500)).is_err(), "{lan} accepted");
        }
    }
}
