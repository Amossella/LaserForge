//! GRBL protocol pieces that don't depend on a serial port: status reports (with WCO), `$n=v` settings,
//! the controller checks, and the buffered (character-counting) streaming logic.

use std::collections::{BTreeMap, VecDeque};

pub const RX_BUFFER: usize = 127;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    pub state: String,
    /// Work position (MPos − WCO when the controller reports machine positions).
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub feed: Option<f64>,
    pub spindle: Option<i64>,
}

/// Keeps the last WCO, which GRBL only sends every few reports.
#[derive(Debug, Default)]
pub struct StatusParser {
    wco: Option<[f64; 3]>,
}

impl StatusParser {
    pub fn parse(&mut self, line: &str) -> Option<Status> {
        let line = line.trim();
        if !(line.starts_with('<') && line.ends_with('>')) {
            return None;
        }
        let inner = &line[1..line.len() - 1];
        let parts: Vec<&str> = inner.split('|').collect();
        let mut st = Status { state: parts.first()?.to_string(), ..Default::default() };
        for p in &parts[1..] {
            if let Some(v) = p.strip_prefix("WCO:") {
                let n: Vec<f64> = v.split(',').filter_map(|s| s.parse().ok()).collect();
                if !n.is_empty() {
                    self.wco = Some([n[0], *n.get(1).unwrap_or(&0.0), *n.get(2).unwrap_or(&0.0)]);
                }
            }
        }
        for p in &parts[1..] {
            let (is_m, v) = if let Some(v) = p.strip_prefix("MPos:") {
                (true, v)
            } else if let Some(v) = p.strip_prefix("WPos:") {
                (false, v)
            } else if let Some(v) = p.strip_prefix("FS:") {
                let f: Vec<&str> = v.split(',').collect();
                st.feed = f.first().and_then(|s| s.parse().ok());
                st.spindle = f.get(1).and_then(|s| s.parse().ok());
                continue;
            } else {
                continue;
            };
            let mut c: Vec<f64> = v.split(',').filter_map(|s| s.parse().ok()).collect();
            if is_m {
                let w = self.wco.unwrap_or([0.0; 3]);
                for (i, val) in c.iter_mut().enumerate() {
                    *val -= *w.get(i).unwrap_or(&0.0);
                }
            }
            st.x = *c.first().unwrap_or(&0.0);
            st.y = *c.get(1).unwrap_or(&0.0);
            st.z = *c.get(2).unwrap_or(&0.0);
        }
        Some(st)
    }
}

/// A `$n=value` line from `$$`.
pub fn parse_setting(line: &str) -> Option<(u32, f64)> {
    let l = line.trim().strip_prefix('$')?;
    let (k, v) = l.split_once('=')?;
    let v: String = v.chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-').collect();
    Some((k.parse().ok()?, v.parse().ok()?))
}

/// The checks made on a controller's settings after connecting.
pub fn problems(settings: &BTreeMap<u32, f64>, max_power: i64) -> Vec<String> {
    let g = |k: u32| settings.get(&k).copied();
    let mut out = Vec::new();
    if g(32) == Some(0.0) {
        out.push("Modalità laser spenta ($32=0): il laser resta acceso negli spostamenti e rallenta agli angoli. Imposta $32=1.".into());
    }
    if g(20) == Some(1.0) && g(22) == Some(0.0) {
        out.push("I limiti software ($20) non funzionano senza homing ($22=0).".into());
    }
    if g(13) == Some(1.0) {
        out.push("GRBL riporta la posizione in pollici ($13=1): imposta $13=0.".into());
    }
    if let Some(s) = g(30) {
        if s != 0.0 && (s - max_power as f64).abs() > 0.5 {
            out.push(format!("Valore S max diverso: GRBL $30={}, LaserForge {}. Le potenze sarebbero sbagliate.", fmt_g(s), max_power));
        }
    }
    out
}

fn fmt_g(v: f64) -> String {
    if v.fract() == 0.0 { format!("{}", v as i64) } else { format!("{}", v) }
}

/// Character-counting streamer: keeps GRBL's serial buffer full without overflowing it.
/// `$` system commands (except `$J=`) go alone, after everything before them has been acknowledged.
#[derive(Debug, Default)]
pub struct Streamer {
    queue: VecDeque<String>,
    inflight: VecDeque<(String, usize, bool)>,
    pub sent: usize,
    pub acked: usize,
}

impl Streamer {
    pub fn push_program(&mut self, gcode: &str) {
        for l in gcode.lines() {
            let t = l.trim();
            if !t.is_empty() && !t.starts_with(';') {
                self.queue.push_back(t.to_string());
            }
        }
    }

    pub fn in_flight_bytes(&self) -> usize {
        self.inflight.iter().map(|x| x.1).sum()
    }

    /// The next line to write now, if it fits.
    pub fn next_to_send(&mut self) -> Option<String> {
        let line = self.queue.front()?;
        let n = line.len() + 1;
        let exclusive = line.starts_with('$') && !line.starts_with("$J=");
        let last_exclusive = self.inflight.back().map(|x| x.2).unwrap_or(false);
        let fits = !exclusive && !last_exclusive && self.in_flight_bytes() + n <= RX_BUFFER;
        if fits || self.inflight.is_empty() {
            let line = self.queue.pop_front()?;
            self.inflight.push_back((line.clone(), n, exclusive));
            self.sent += 1;
            Some(line)
        } else {
            None
        }
    }

    /// An `ok` or `error:N` arrived for the oldest line.
    pub fn on_response(&mut self) -> Option<String> {
        let (l, _, _) = self.inflight.pop_front()?;
        self.acked += 1;
        Some(l)
    }

    /// Soft reset or alarm: GRBL dropped its buffer, nothing more will be acknowledged.
    pub fn reset(&mut self) {
        self.queue.clear();
        self.inflight.clear();
    }

    pub fn done(&self) -> bool {
        self.queue.is_empty() && self.inflight.is_empty()
    }
}
