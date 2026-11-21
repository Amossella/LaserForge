//! Connection to a GRBL controller over USB serial or TCP, with buffered (character-counting) streaming.
//! One worker thread owns the writing side: it writes lines as GRBL's buffer frees up, handles replies and
//! polls status; a reader thread turns the input into lines.

use laserforge_core::grbl::{parse_setting, Status, StatusParser, Streamer};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    Disconnected,
    Connected,
    Alarm,
}

#[derive(Debug, Clone)]
pub struct ConsoleLine {
    pub text: String,
    pub kind: u8, // 0 sent, 1 reply, 2 error, 3 info
}

#[derive(Debug, Default)]
pub struct Shared {
    pub link: Option<Link>,
    pub status: Status,
    pub console: Vec<ConsoleLine>,
    pub settings: BTreeMap<u32, f64>,
    /// The controller's startup banner, once seen.
    pub banner: String,
    pub sent: usize,
    pub acked: usize,
    pub total: usize,
    pub busy: bool,
}

impl Shared {
    fn log(&mut self, text: impl Into<String>, kind: u8) {
        self.console.push(ConsoleLine { text: text.into(), kind });
        if self.console.len() > 2000 {
            self.console.drain(0..500);
        }
    }
}

enum Cmd {
    Hold(bool),
    Line(String),
    Program(String),
    Realtime(u8),
    Stop,
    Close,
}

pub struct Connection {
    tx: Sender<Cmd>,
    pub shared: Arc<Mutex<Shared>>,
    /// Controller family: grbl, grblhal, fluidnc, smoothie, marlin, linuxcnc.
    pub flavor: String,
}

/// GRBL-style controllers: character-counting buffer, `$$` settings, `$J=` jogging, real-time overrides.
pub fn grbl_family(flavor: &str) -> bool {
    matches!(flavor, "grbl" | "grblhal" | "fluidnc")
}

/// Controllers that understand the real-time bytes (soft reset, `!` hold, `~` resume, `?` status).
pub fn realtime_family(flavor: &str) -> bool {
    grbl_family(flavor) || flavor == "smoothie"
}

pub fn list_ports() -> Vec<String> {
    serialport::available_ports()
        .map(|v| v.into_iter().map(|p| p.port_name).filter(|n| !n.contains("Bluetooth") && !n.contains("debug-console")).collect())
        .unwrap_or_default()
}

type Halves = (Box<dyn Read + Send>, Box<dyn Write + Send>, Duration);

fn open_port(address: &str, baud: u32) -> Result<Halves, String> {
    if address.eq_ignore_ascii_case("SIM") {
        let (r, w) = sim::open();
        return Ok((Box::new(r), Box::new(w), Duration::from_millis(50)));
    }
    if let Some(hp) = address.strip_prefix("tcp://") {
        let hp = if hp.contains(':') { hp.to_string() } else { format!("{hp}:23") };
        let s = TcpStream::connect(&hp).map_err(|e| format!("{hp}: {e}"))?;
        s.set_nodelay(true).ok();
        let r = s.try_clone().map_err(|e| e.to_string())?;
        r.set_read_timeout(Some(Duration::from_millis(100))).ok();
        return Ok((Box::new(r), Box::new(s), Duration::from_millis(300)));
    }
    let mut last = String::new();
    // just after a disconnect the old reader may still hold the (exclusive) port for a moment
    for _ in 0..5 {
        match serialport::new(address, baud).timeout(Duration::from_millis(50)).open() {
            Ok(p) => {
                let r = p.try_clone().map_err(|e| e.to_string())?;
                // opening a USB port resets most boards: GRBL needs about 2 s to boot
                return Ok((Box::new(r), Box::new(p), Duration::from_secs(2)));
            }
            Err(e) => {
                // pseudo-terminals and some virtual ports refuse the baud-rate ioctl: use them as plain files
                if let Ok(f) = std::fs::OpenOptions::new().read(true).write(true).open(address) {
                    let r = f.try_clone().map_err(|e| e.to_string())?;
                    return Ok((Box::new(r), Box::new(f), Duration::from_millis(100)));
                }
                last = format!("{address}: {e}");
                thread::sleep(Duration::from_millis(200));
            }
        }
    }
    Err(last)
}

impl Connection {
    /// `address`: a serial device (/dev/cu.usbserial…) or `tcp://host:port`.
    /// `sync`: synchronous transfer (one line, wait for its ok) instead of character counting.
    pub fn open(address: &str, baud: u32, sync: bool) -> Result<Connection, String> {
        Self::open_flavor(address, baud, sync, "grbl")
    }

    /// Like `open` for a given controller family: Marlin / LinuxCNC get no reset and no `?` polling, and every
    /// non-GRBL controller is streamed synchronously (one line, wait for its ok) without asking for `$$`.
    pub fn open_flavor(address: &str, baud: u32, sync: bool, flavor: &str) -> Result<Connection, String> {
        let flavor = if flavor.is_empty() || address.eq_ignore_ascii_case("SIM") { "grbl" } else { flavor };
        let fl = Flavor { reset: realtime_family(flavor), poll: !matches!(flavor, "marlin" | "linuxcnc"), settings: grbl_family(flavor) };
        let sync = sync || !grbl_family(flavor);
        let (reader, writer, boot) = open_port(address, baud)?;
        let shared = Arc::new(Mutex::new(Shared { link: Some(Link::Connected), ..Default::default() }));
        let (tx, rx) = channel::<Cmd>();
        let (ltx, lrx) = channel::<String>();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_r = stop.clone();
        thread::spawn(move || {
            let mut br = BufReader::new(reader);
            let mut line = String::new();
            while !stop_r.load(Ordering::SeqCst) {
                // a timeout keeps the partial line in `line`: only clear it after a whole line
                match br.read_line(&mut line) {
                    Ok(0) => break, // port closed or unplugged
                    Ok(_) => {
                        let t = line.trim().to_string();
                        line.clear();
                        if !t.is_empty() && ltx.send(t).is_err() {
                            break;
                        }
                    }
                    Err(e) if matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted) => continue,
                    Err(_) => break,
                }
            }
        });
        let sh = shared.clone();
        thread::spawn(move || {
            worker(writer, rx, lrx, sh.clone(), boot, sync, fl);
            stop.store(true, Ordering::SeqCst);
            let mut s = sh.lock().unwrap();
            s.link = Some(Link::Disconnected);
            s.busy = false;
        });
        Ok(Connection { tx, shared, flavor: flavor.to_string() })
    }

    /// Feed hold: `!` on GRBL-style controllers; on the others new lines simply stop being sent.
    pub fn pause(&self) {
        let _ = self.tx.send(Cmd::Hold(true));
        if realtime_family(&self.flavor) {
            self.realtime(b'!');
        }
    }
    pub fn resume(&self) {
        let _ = self.tx.send(Cmd::Hold(false));
        if realtime_family(&self.flavor) {
            self.realtime(b'~');
        }
    }

    pub fn send(&self, line: String) {
        let _ = self.tx.send(Cmd::Line(line));
    }
    pub fn stream(&self, gcode: String) {
        {
            // marked busy at once, so the UI (and callers polling) never see the job as already finished
            let mut s = self.shared.lock().unwrap();
            s.busy = true;
            s.total = gcode.lines().filter(|l| !l.trim().is_empty() && !l.trim().starts_with(';')).count();
            s.acked = 0;
            s.sent = 0;
        }
        let _ = self.tx.send(Cmd::Program(gcode));
    }
    /// Jog cancel (real-time 0x85): stops a `$J=` move at once without a reset.
    pub fn cancel_jog(&self) {
        self.realtime(0x85);
    }
    pub fn realtime(&self, b: u8) {
        let _ = self.tx.send(Cmd::Realtime(b));
    }
    /// Soft reset: GRBL stops at once and drops everything queued.
    pub fn stop(&self) {
        let _ = self.tx.send(Cmd::Stop);
    }
    pub fn close(&self) {
        let _ = self.tx.send(Cmd::Close);
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Close);
    }
}

#[derive(Clone, Copy)]
struct Flavor {
    reset: bool,
    poll: bool,
    settings: bool,
}

fn worker(mut w: Box<dyn Write + Send>, rx: Receiver<Cmd>, lines: Receiver<String>, sh: Arc<Mutex<Shared>>, boot: Duration, sync: bool, fl: Flavor) {
    // startup: let the board boot, drop its noise, soft-reset; the banner that follows triggers `$$`
    let t0 = Instant::now();
    while t0.elapsed() < boot {
        while lines.try_recv().is_ok() {}
        thread::sleep(Duration::from_millis(20));
    }
    while lines.try_recv().is_ok() {}
    if fl.reset && w.write_all(&[0x18]).and_then(|_| w.flush()).is_err() {
        sh.lock().unwrap().log("Connessione persa", 2);
        return;
    }
    let reset_at = Instant::now();
    let mut asked_settings = !fl.settings;
    let mut held = false;

    let mut st = Streamer::default();
    let mut parser = StatusParser::default();
    let mut last_poll = Instant::now();
    // a job waits until everything sent before it is acknowledged, so its progress counts only its own lines
    let mut pending: Option<String> = None;
    let mut running = false;
    loop {
        // commands from the UI
        loop {
            match rx.try_recv() {
                Ok(Cmd::Line(l)) => {
                    sh.lock().unwrap().log(format!("> {l}"), 0);
                    st.push_program(&l);
                }
                Ok(Cmd::Program(g)) => pending = Some(g),
                Ok(Cmd::Hold(h)) => held = h,
                Ok(Cmd::Realtime(b)) => {
                    let _ = w.write_all(&[b]).and_then(|_| w.flush());
                }
                Ok(Cmd::Stop) => {
                    let _ = w.write_all(&[0x18]).and_then(|_| w.flush());
                    held = false;
                    st.reset();
                    pending = None;
                    running = false;
                    let mut s = sh.lock().unwrap();
                    s.busy = false;
                    s.log("Stop (reset)", 3);
                }
                Ok(Cmd::Close) | Err(TryRecvError::Disconnected) => return,
                Err(TryRecvError::Empty) => break,
            }
        }
        // no banner after the reset (some clones stay silent): ask for the settings anyway
        if !asked_settings && reset_at.elapsed() > Duration::from_millis(2500) {
            asked_settings = true;
            st.push_program("$$");
        }
        if st.done() {
            if let Some(g) = pending.take() {
                st.acked = 0;
                st.sent = 0;
                st.push_program(&g);
                running = true;
                let mut s = sh.lock().unwrap();
                if st.done() {
                    running = false;
                    s.busy = false;
                } else {
                    let n = s.total;
                    s.log(format!("Lavoro avviato: {n} righe"), 3);
                }
            }
        }
        // write while GRBL's buffer has room
        let mut wrote = false;
        while let Some(l) = if held || (sync && st.sent > st.acked) { None } else { st.next_to_send() } {
            wrote = true;
            if w.write_all(format!("{l}\n").as_bytes()).is_err() {
                sh.lock().unwrap().log("Connessione persa", 2);
                return;
            }
            if sync {
                break;
            }
        }
        if wrote {
            let _ = w.flush();
        }
        // replies
        let mut got_any = false;
        loop {
            let line = match lines.try_recv() {
                Ok(l) => l,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    sh.lock().unwrap().log("Connessione persa", 2);
                    return;
                }
            };
            got_any = true;
            let mut s = sh.lock().unwrap();
            if line.starts_with('<') {
                if let Some(status) = parser.parse(&line) {
                    let alarm = status.state.starts_with("Alarm");
                    if alarm {
                        s.link = Some(Link::Alarm);
                    } else if s.link == Some(Link::Alarm) {
                        s.link = Some(Link::Connected);
                    }
                    s.status = status;
                }
            } else if line == "ok" || line.starts_with("error") {
                st.on_response();
                if running {
                    s.acked = st.acked.min(s.total);
                    s.sent = st.sent;
                    if st.done() {
                        running = false;
                        s.busy = false;
                        s.log("Lavoro completato", 3);
                    }
                }
                if line.starts_with("error") {
                    s.log(format!("{line} — {}", explain_grbl(&line)), 2);
                }
            } else if line.starts_with("ALARM") {
                s.link = Some(Link::Alarm);
                s.log(format!("{line} — {}", explain_grbl(&line)), 2);
                st.reset();
                pending = None;
                running = false;
                s.busy = false;
            } else if line.starts_with("Grbl ") || line.starts_with("GrblHAL ") {
                // the controller (re)started: whatever was queued is gone
                if running || pending.is_some() {
                    s.log("Il controller si è riavviato: lavoro interrotto", 2);
                }
                st.reset();
                pending = None;
                running = false;
                s.busy = false;
                s.banner = line.clone();
                s.log(line.clone(), 1);
                if fl.settings {
                    asked_settings = true;
                    st.push_program("$$");
                }
            } else {
                if let Some((k, v)) = parse_setting(&line) {
                    s.settings.insert(k, v);
                }
                if line.contains("to unlock") {
                    s.link = Some(Link::Alarm);
                }
                s.log(line.clone(), 1);
            }
        }
        if fl.poll && last_poll.elapsed() > Duration::from_millis(250) {
            if w.write_all(b"?").and_then(|_| w.flush()).is_err() {
                sh.lock().unwrap().log("Connessione persa", 2);
                return;
            }
            last_poll = Instant::now();
        }
        if !got_any {
            thread::sleep(Duration::from_millis(3));
        }
    }
}

/// serial_comm.detect_controllers: probe every serial port for a controller banner → (port, flavour, banner).
pub fn detect_controllers(baud: u32) -> Vec<(String, String, String)> {
    let mut found = Vec::new();
    for port in list_ports() {
        let Ok(mut p) = serialport::new(&port, baud).timeout(Duration::from_millis(300)).open() else { continue };
        thread::sleep(Duration::from_millis(1500));
        let _ = p.write_all(b"\r\n\x18");
        thread::sleep(Duration::from_millis(400));
        let _ = p.write_all(b"$I\nM115\nversion\n");
        let end = Instant::now() + Duration::from_millis(2500);
        let mut text = Vec::new();
        let mut buf = [0u8; 256];
        while Instant::now() < end {
            match p.read(&mut buf) {
                Ok(n) if n > 0 => text.extend_from_slice(&buf[..n]),
                _ => {}
            }
        }
        let t = String::from_utf8_lossy(&text).to_string();
        let low = t.to_lowercase();
        let flavor = if low.contains("fluidnc") { "fluidnc" } else if low.contains("grblhal") { "grblhal" } else if low.contains("grbl") { "grbl" }
                     else if low.contains("marlin") { "marlin" } else if low.contains("smoothie") { "smoothie" } else { continue };
        let banner = t.trim().lines().next().unwrap_or(flavor).chars().take(80).collect();
        found.push((port, flavor.to_string(), banner));
    }
    found
}

/// A short explanation of a GRBL `ALARM:n` or `error:n` line, shown next to it in the console.
pub fn explain_grbl(line: &str) -> &'static str {
    let code = line.split_once(':').map(|(_, c)| c.trim()).unwrap_or("");
    if line.starts_with("ALARM") {
        return match code {
            "1" => "finecorsa toccato (hard limit): la macchina si è fermata. Libera la testa e fai Home ($H) o sblocca ($X)",
            "2" => "movimento fuori dall'area della macchina (soft limit): il lavoro o il frame esce dai limiti. Controlla area di lavoro, origine e 'Avvia da', poi sblocca ($X) o fai Home ($H)",
            "3" => "reset durante il movimento: la posizione potrebbe essere persa. Fai Home ($H) o sblocca ($X)",
            "4" => "sonda in stato errato all'inizio del ciclo",
            "5" => "la sonda non ha toccato nulla",
            "6" => "homing interrotto da un reset",
            "7" => "homing fallito: porta di sicurezza aperta",
            "8" => "homing fallito: il finecorsa non si è liberato",
            "9" => "homing fallito: finecorsa non trovato (controlla cablaggio e $130/$131)",
            "10" => "homing fallito: secondo finecorsa (dual axis) non trovato",
            _ => "allarme: sblocca con $X o fai Home ($H)",
        };
    }
    match code {
        "1" => "comando G-code senza lettera",
        "2" => "valore numerico mancante",
        "3" => "comando $ non supportato",
        "8" => "comando $ accettato solo a macchina ferma",
        "9" => "macchina in allarme: sblocca con $X o fai Home ($H)",
        "10" => "soft limit attivo ma la macchina non ha fatto Home",
        "15" => "jog oltre i limiti della macchina",
        "16" => "comando di jog non valido",
        "20" => "comando G-code non supportato",
        "22" => "velocità F non impostata",
        "24" => "due comandi dello stesso gruppo sulla stessa riga",
        "33" => "destinazione dell'arco non valida",
        _ => "vedi la documentazione GRBL",
    }
}

/// Find GRBL-family controllers on the local network through Bonjour (macOS `dns-sd`).
/// grblHAL / FluidNC advertise telnet on port 23; FluidNC also an http page (its G-code stream is still on telnet).
/// Returns (name, host, port).
pub fn discover_network(timeout: Duration) -> Vec<(String, String, u16)> {
    use std::process::{Command, Stdio};
    fn run(args: &[&str], secs: Duration) -> String {
        let Ok(mut child) = Command::new("dns-sd").args(args).stdout(Stdio::piped()).stderr(Stdio::null()).spawn() else { return String::new() };
        let mut out = child.stdout.take();
        let h = thread::spawn(move || {
            let mut s = String::new();
            if let Some(o) = out.as_mut() {
                let _ = o.read_to_string(&mut s);
            }
            s
        });
        thread::sleep(secs);
        let _ = child.kill();
        let _ = child.wait();
        h.join().unwrap_or_default()
    }
    let browse = regex::Regex::new(r"\sAdd\s+\S+\s+\S+\s+\S+\s+(_\w+\._tcp)\.\s+(.+)$").unwrap();
    let reach = regex::Regex::new(r"can be reached at\s+(\S+?)\.?:(\d+)").unwrap();
    let likely = regex::RegexBuilder::new("fluid|grbl|laser|cnc").case_insensitive(true).build().unwrap();
    let mut found: Vec<(String, String, u16)> = Vec::new();
    for stype in ["_telnet._tcp", "_http._tcp"] {
        for ln in run(&["-B", stype, "local."], timeout).lines() {
            let Some(m) = browse.captures(ln) else { continue };
            if &m[1] != stype {
                continue;
            }
            let name = m[2].trim().to_string();
            if (stype == "_http._tcp" && !likely.is_match(&name)) || found.iter().any(|f| f.0 == name) {
                continue;
            }
            let res = run(&["-L", &name, stype, "local."], Duration::from_millis(1500));
            let Some(r) = reach.captures(&res) else { continue };
            let mut host = r[1].to_string();
            let port = if stype == "_http._tcp" { 23 } else { r[2].parse().unwrap_or(23) };
            if let Ok(mut a) = std::net::ToSocketAddrs::to_socket_addrs(&(host.as_str(), port)) {
                if let Some(ip) = a.find(|a| a.is_ipv4()) {
                    host = ip.ip().to_string();
                }
            }
            found.push((name, host, port));
        }
    }
    found
}

/// A simulated GRBL controller (port name "SIM"), to try the app without hardware.
/// It answers every line with `ok`, tracks the position of G0/G1 and jog moves and replies to `?` and `$$`.
pub mod sim {
    use std::io::{Read, Write};
    use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
    use std::thread;
    use std::time::Duration;

    pub struct SimReader {
        rx: Receiver<Vec<u8>>,
        buf: Vec<u8>,
    }
    pub struct SimWriter {
        tx: Sender<Vec<u8>>,
    }

    impl Read for SimReader {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            if self.buf.is_empty() {
                match self.rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(b) => self.buf = b,
                    Err(RecvTimeoutError::Timeout) => return Err(std::io::ErrorKind::TimedOut.into()),
                    Err(RecvTimeoutError::Disconnected) => return Ok(0),
                }
            }
            let n = out.len().min(self.buf.len());
            out[..n].copy_from_slice(&self.buf[..n]);
            self.buf.drain(..n);
            Ok(n)
        }
    }

    impl Write for SimWriter {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.tx.send(b.to_vec()).map_err(|_| std::io::Error::from(std::io::ErrorKind::BrokenPipe))?;
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    pub const BANNER: &str = "Grbl 1.1h [LaserForge Simulator]";

    #[derive(Default)]
    pub struct Machine {
        pub pos: [f64; 3],
        pub relative: bool,
        pub queued: usize,
        pub alarm: bool,
    }

    fn word(part: &str, axis: char) -> Option<f64> {
        part.strip_prefix(axis).and_then(|v| v.parse().ok())
    }

    impl Machine {
        /// The reply lines to one received line (without the newline).
        pub fn line(&mut self, cmd: &str) -> Vec<String> {
            let cmd = cmd.trim();
            if cmd.is_empty() {
                return vec![];
            }
            let up = cmd.to_ascii_uppercase();
            if up == "$$" {
                let mut v: Vec<String> = [(0, "10"), (1, "25"), (10, "1"), (22, "0"), (30, "1000"), (31, "0"), (32, "1"), (100, "80.000"), (101, "80.000"),
                                          (110, "6000.000"), (111, "6000.000"), (120, "500.000"), (121, "500.000"), (130, "400.000"), (131, "400.000")]
                    .iter().map(|(k, v)| format!("${k}={v}")).collect();
                v.push("ok".into());
                return v;
            }
            if up == "$X" || up == "$H" {
                self.alarm = false;
                if up == "$H" {
                    self.pos = [0.0; 3];
                }
                return vec!["ok".into()];
            }
            if up == "$I" {
                return vec![format!("[VER:1.1h.20190825:]"), "[OPT:V,15,128]".into(), "ok".into()];
            }
            let (moving, rel) = if let Some(j) = up.strip_prefix("$J=") {
                (j, j.contains("G91") || self.relative)
            } else {
                if up.contains("G91") {
                    self.relative = true;
                }
                if up.contains("G90") {
                    self.relative = false;
                }
                (up.as_str(), self.relative)
            };
            for part in moving.split_whitespace() {
                for (i, a) in ['X', 'Y', 'Z'].iter().enumerate() {
                    if let Some(v) = word(part, *a) {
                        self.pos[i] = if rel { self.pos[i] + v } else { v };
                    }
                }
            }
            vec!["ok".into()]
        }

        pub fn status(&self) -> String {
            let state = if self.alarm { "Alarm" } else if self.queued > 0 { "Run" } else { "Idle" };
            format!("<{state}|MPos:{:.3},{:.3},{:.3}|FS:0,0>", self.pos[0], self.pos[1], self.pos[2])
        }
    }

    pub fn open() -> (SimReader, SimWriter) {
        let (to_sim, from_app) = channel::<Vec<u8>>();
        let (to_app, from_sim) = channel::<Vec<u8>>();
        thread::spawn(move || {
            let mut m = Machine::default();
            let mut line = Vec::new();
            let send = |s: String| to_app.send(format!("{s}\r\n").into_bytes()).is_ok();
            while let Ok(chunk) = from_app.recv() {
                for b in chunk {
                    match b {
                        b'?' => {
                            if !send(m.status()) {
                                return;
                            }
                        }
                        0x18 => {
                            line.clear();
                            m.queued = 0;
                            if !send(String::new()) || !send(BANNER.into()) {
                                return;
                            }
                        }
                        b'!' | b'~' | 0x85 | 0x80..=0xff => {}
                        b'\n' => {
                            let cmd = String::from_utf8_lossy(&line).to_string();
                            line.clear();
                            thread::sleep(Duration::from_millis(2));
                            for r in m.line(&cmd) {
                                if !send(r) {
                                    return;
                                }
                            }
                        }
                        b'\r' => {}
                        _ => line.push(b),
                    }
                }
            }
        });
        (SimReader { rx: from_sim, buf: Vec::new() }, SimWriter { tx: to_sim })
    }
}
