//! Optional remote control from a phone on the local network (off by default, PIN protected): a small web page
//! with the laser status and start / pause / resume / stop / frame / home / jog / air. Commands go to the UI
//! thread through a channel. Ported from laserforge/remote.py.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

const PAGE: &str = include_str!("../assets/remote.html");

/// What the page shows; refreshed by the UI each frame.
#[derive(Default, Clone)]
pub struct Snapshot {
    pub state: String,
    pub x: f64,
    pub y: f64,
    pub sent: usize,
    pub total: usize,
    pub checklist: bool,
}

pub struct Remote {
    pub rx: Receiver<(String, f64, f64)>,
    pub status: Arc<Mutex<Snapshot>>,
    stop: Arc<AtomicBool>,
    pub url: String,
}

impl Drop for Remote {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

pub fn local_ip() -> String {
    UdpSocket::bind("0.0.0.0:0").and_then(|s| { s.connect("10.255.255.255:1")?; s.local_addr() }).map(|a| a.ip().to_string()).unwrap_or_else(|_| "127.0.0.1".into())
}

fn send(s: &mut TcpStream, code: u16, body: &str, ctype: &str) {
    let reason = match code { 200 => "OK", 403 => "Forbidden", _ => "Not Found" };
    let _ = write!(s, "HTTP/1.1 {code} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}", body.len());
}

fn handle(mut s: TcpStream, pin: &str, status: &Arc<Mutex<Snapshot>>, tx: &Sender<(String, f64, f64)>) {
    let mut r = BufReader::new(s.try_clone().unwrap());
    let mut line = String::new();
    if r.read_line(&mut line).is_err() {
        return;
    }
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("/").to_string());
    let mut got_pin = String::new();
    let mut len = 0usize;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).is_err() || h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            match k.trim().to_ascii_lowercase().as_str() {
                "x-pin" => got_pin = v.trim().to_string(),
                "content-length" => len = v.trim().parse().unwrap_or(0),
                _ => {}
            }
        }
    }
    if len > 0 {
        let mut buf = vec![0u8; len.min(4096)];
        let _ = r.read_exact(&mut buf);
    }
    let (path, query) = target.split_once('?').map(|(a, b)| (a.to_string(), b.to_string())).unwrap_or((target.clone(), String::new()));
    let ok = !pin.is_empty() && got_pin == pin;
    if method == "GET" && path == "/" {
        return send(&mut s, 200, PAGE, "text/html; charset=utf-8");
    }
    if !ok {
        return send(&mut s, 403, "{\"error\":\"pin\"}", "application/json");
    }
    if method == "GET" && path == "/api/status" {
        let st = status.lock().unwrap().clone();
        let body = serde_json::json!({"state": st.state, "x": st.x, "y": st.y, "sent": st.sent, "total": st.total}).to_string();
        return send(&mut s, 200, &body, "application/json");
    }
    if method == "POST" {
        let action = path.trim_start_matches("/api/").to_string();
        if !["start", "pause", "resume", "stop", "frame", "home", "jog", "air_on", "air_off"].contains(&action.as_str()) {
            return send(&mut s, 404, "{\"error\":\"unknown\"}", "application/json");
        }
        if action == "start" && status.lock().unwrap().checklist {
            return send(&mut s, 200, "{\"message\":\"Checklist attiva: avvia dal computer.\"}", "application/json");
        }
        let q = |k: &str| query.split('&').find_map(|kv| kv.split_once('=').filter(|(a, _)| *a == k).and_then(|(_, v)| v.parse::<f64>().ok())).unwrap_or(0.0);
        let _ = tx.send((action, q("x"), q("y")));
        return send(&mut s, 200, "{\"ok\":true}", "application/json");
    }
    send(&mut s, 404, "{\"error\":\"not found\"}", "application/json")
}

impl Remote {
    pub fn start(port: u16, pin: &str) -> Result<Remote, String> {
        let listener = TcpListener::bind(("0.0.0.0", port)).map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).ok();
        let (tx, rx) = channel();
        let status = Arc::new(Mutex::new(Snapshot::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (st, sp, pin) = (status.clone(), stop.clone(), pin.to_string());
        std::thread::spawn(move || {
            while !sp.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((s, _)) => {
                        s.set_nonblocking(false).ok();
                        s.set_read_timeout(Some(std::time::Duration::from_secs(5))).ok();
                        let (st, tx, pin) = (st.clone(), tx.clone(), pin.clone());
                        std::thread::spawn(move || handle(s, &pin, &st, &tx));
                    }
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(50)),
                }
            }
        });
        Ok(Remote { rx, status, stop, url: format!("http://{}:{port}", local_ip()) })
    }
}
