//! Streams a real job to the GRBL emulator (tests/fake_grbl.py on a pty) through the app's connection layer.
use laserforge_app::connection::{Connection, Link};
use laserforge_app::project::{process_image, Doc, JobOpts};
use laserforge_app::shapes::Shape;
use laserforge_core::model::{CutMode, MachineSettings};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn wait(c: &Connection, secs: u64, f: impl Fn(&laserforge_app::connection::Shared) -> bool) -> bool {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(secs) {
        if f(&c.shared.lock().unwrap()) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn streams_a_job_to_emulated_grbl() {
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../tools/fake_grbl_server.py");
    let mut py = Command::new("python3").arg(script).env("FAKE_DELAY", "0").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().expect("python3");
    let mut out = BufReader::new(py.stdout.take().unwrap());
    let mut path = String::new();
    out.read_line(&mut path).unwrap();
    let c = Connection::open(path.trim(), 115200, false).expect("open pty");
    // $$ answered and status polled
    assert!(wait(&c, 5, |s| s.settings.get(&32) == Some(&1.0)), "no $$ reply");
    assert!(wait(&c, 5, |s| s.status.state == "Idle"), "no status");

    let m = MachineSettings { workspace_x: 300.0, workspace_y: 300.0, ..Default::default() };
    let mut d = Doc::default();
    d.shapes.push(Shape::rect(1, 0, 10.0, 10.0, 50.0, 30.0));
    d.shapes.push(Shape::circle(2, 1, 125.0, 120.0, 25.0, 20.0));
    d.shapes.push(Shape::rect(3, 2, 20.0, 200.0, 20.0, 15.0));
    d.layers[2].cut_mode = CutMode::Fill;
    d.layers[2].line_interval = 0.5;
    let job = d.build_job(&m, &JobOpts { start_from: "absolute".into(), ..Default::default() }, &mut |s, i, p| process_image(s, i, p)).0;
    let g = laserforge_core::generate(&job);
    let expected: Vec<String> = g.send.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty() && !l.starts_with(';')).collect();
    assert!(expected.len() > 100);
    c.stream(g.send.clone());
    assert!(wait(&c, 60, |s| !s.busy), "job did not finish");
    {
        let s = c.shared.lock().unwrap();
        assert_eq!(s.acked, expected.len());
        assert_eq!(s.total, expected.len());
        assert_ne!(s.link, Some(Link::Alarm));
        assert!(s.console.iter().any(|l| l.text == "Lavoro completato"));
    }
    // jog, then a stop in the middle of a second job
    c.send("$J=G91 G21 X5.000 Y5.000 F3000".into());
    c.stream(g.send.clone());
    std::thread::sleep(Duration::from_millis(30));
    c.stop();
    assert!(wait(&c, 5, |s| !s.busy), "stop did not end the job");
    c.close();
    std::thread::sleep(Duration::from_millis(100));

    drop(py.stdin.take());
    let mut rest = String::new();
    out.read_to_string(&mut rest).unwrap();
    let _ = py.wait();
    let log: Vec<String> = serde_json::from_str(rest.trim()).unwrap();
    assert_eq!(log[0], "$$");
    // every job line arrived in order, nothing lost or reordered by the character-counting streamer
    assert_eq!(&log[1..1 + expected.len()], expected.as_slice());
    assert_eq!(log[1 + expected.len()], "$J=G91 G21 X5.000 Y5.000 F3000");
    let _ = std::io::stdout().flush();
}

fn fake() -> (std::process::Child, BufReader<std::process::ChildStdout>, String) {
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../tools/fake_grbl_server.py");
    let mut py = Command::new("python3").arg(script).env("FAKE_DELAY", "0.003").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().expect("python3");
    let mut out = BufReader::new(py.stdout.take().unwrap());
    let mut path = String::new();
    out.read_line(&mut path).unwrap();
    (py, out, path.trim().to_string())
}

fn long_job() -> String {
    let m = MachineSettings { workspace_x: 300.0, workspace_y: 300.0, ..Default::default() };
    let mut d = Doc::default();
    d.shapes.push(Shape::rect(1, 0, 20.0, 20.0, 100.0, 100.0));
    d.layers[0].cut_mode = CutMode::Fill;
    d.layers[0].line_interval = 0.2;
    laserforge_core::generate(&d.build_job(&m, &JobOpts { start_from: "absolute".into(), ..Default::default() }, &mut |s, i, p| process_image(s, i, p)).0).send
}

/// The controller restarting mid-job (power glitch, reset button) ends the job and settings are re-read.
#[test]
fn controller_restart_mid_job() {
    let (mut py, mut out, path) = fake();
    let c = Connection::open(&path, 115200, false).unwrap();
    assert!(wait(&c, 5, |s| s.settings.contains_key(&32)));
    c.stream(long_job());
    assert!(wait(&c, 5, |s| s.acked > 20));
    py.stdin.as_mut().unwrap().write_all(b"banner\n").unwrap();
    py.stdin.as_mut().unwrap().flush().unwrap();
    assert!(wait(&c, 5, |s| !s.busy), "restart did not end the job");
    let (acked, total) = { let s = c.shared.lock().unwrap(); (s.acked, s.total) };
    assert!(acked < total);
    assert!(c.shared.lock().unwrap().console.iter().any(|l| l.text.contains("riavviato")));
    // and the connection keeps working
    c.send("G0 X1 Y1".into());
    std::thread::sleep(Duration::from_millis(300));
    drop(c);
    drop(py.stdin.take());
    let mut rest = String::new();
    out.read_to_string(&mut rest).unwrap();
    let _ = py.wait();
    let log: Vec<String> = serde_json::from_str(rest.trim()).unwrap();
    assert!(log.iter().filter(|l| *l == "$$").count() >= 2, "settings not re-read after the restart");
    assert_eq!(log.last().unwrap(), "G0 X1 Y1");
}

/// Disconnect then connect again to the same port.
#[test]
fn reconnect_same_port() {
    let (mut py, _out, path) = fake();
    for _ in 0..2 {
        let c = Connection::open(&path, 115200, false).expect("reopen");
        assert!(wait(&c, 5, |s| s.status.state == "Idle"));
        c.close();
        assert!(wait(&c, 3, |s| s.link == Some(Link::Disconnected)));
    }
    drop(py.stdin.take());
    let _ = py.wait();
}

/// GRBL over the network (ESP32 / WiFi boards): a tiny controller on a local TCP socket.
#[test]
fn streams_over_tcp() {
    use std::net::TcpListener;
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let srv = std::thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        let mut r = BufReader::new(s.try_clone().unwrap());
        let mut got = Vec::new();
        let mut buf = Vec::new();
        let mut b = [0u8; 1];
        while r.read(&mut b).unwrap_or(0) == 1 {
            match b[0] {
                b'?' => s.write_all(b"<Idle|WPos:0.000,0.000,0.000>\r\n").unwrap(),
                0x18 => s.write_all(b"Grbl 1.1h ['$' for help]\r\n").unwrap(),
                b'\n' => {
                    let line = String::from_utf8_lossy(&buf).trim().to_string();
                    buf.clear();
                    if line == "$$" {
                        s.write_all(b"$32=1\r\nok\r\n").unwrap();
                    } else {
                        got.push(line);
                        s.write_all(b"ok\r\n").unwrap();
                    }
                }
                c => buf.push(c),
            }
        }
        got
    });
    let c = Connection::open(&format!("tcp://127.0.0.1:{port}"), 0, false).unwrap();
    assert!(wait(&c, 5, |s| s.settings.get(&32) == Some(&1.0)));
    let g = long_job();
    let expected: Vec<String> = g.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty() && !l.starts_with(';')).collect();
    c.stream(g);
    assert!(wait(&c, 30, |s| !s.busy));
    assert_eq!(c.shared.lock().unwrap().acked, expected.len());
    c.close();
    assert!(wait(&c, 3, |s| s.link == Some(Link::Disconnected)));
    drop(c);
    let got = srv.join().unwrap();
    assert_eq!(got, expected);
}

#[test]
fn synchronous_transfer_sends_one_line_at_a_time() {
    let (mut py, mut out, path) = fake();
    let c = Connection::open(&path, 115200, true).expect("open pty");
    assert!(wait(&c, 5, |s| s.settings.get(&32) == Some(&1.0)), "no $$ reply");
    let job: String = (0..60).map(|i| format!("G1 X{}.000 Y{}.000 F1000 S100\n", i % 10, i / 10)).collect();
    c.stream(job.clone());
    let mut max_ahead = 0;
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(30) {
        let (busy, sent, acked) = { let s = c.shared.lock().unwrap(); (s.busy, s.sent, s.acked) };
        max_ahead = max_ahead.max(sent.saturating_sub(acked));
        if !busy {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!c.shared.lock().unwrap().busy, "job did not finish");
    assert!(max_ahead <= 1, "synchronous mode had {max_ahead} lines in flight");
    c.close();
    std::thread::sleep(Duration::from_millis(100));
    drop(py.stdin.take());
    let mut rest = String::new();
    out.read_to_string(&mut rest).unwrap();
    let _ = py.wait();
    let log: Vec<String> = serde_json::from_str(rest.trim()).unwrap();
    let want: Vec<&str> = job.lines().collect();
    assert_eq!(&log[1..1 + want.len()], want.as_slice());
}

#[test]
fn simulator_port_runs_a_job_and_explains_errors() {
    use laserforge_app::connection::explain_grbl;
    let c = Connection::open("SIM", 115200, false).expect("open SIM");
    assert!(wait(&c, 5, |s| s.banner.contains("Simulator")), "no banner");
    assert!(wait(&c, 5, |s| s.settings.get(&32) == Some(&1.0)), "no $$ reply");
    c.send("$J=G91 X5 Y-2 F1000".into());
    assert!(wait(&c, 5, |s| (s.status.x - 5.0).abs() < 1e-6 && (s.status.y + 2.0).abs() < 1e-6), "jog not tracked");
    c.cancel_jog();
    c.stream("G90\nG0 X20 Y30\nG1 X40 F600\n".into());
    assert!(wait(&c, 5, |s| !s.busy && s.acked == 3), "job not finished");
    assert!(wait(&c, 5, |s| (s.status.x - 40.0).abs() < 1e-6 && (s.status.y - 30.0).abs() < 1e-6));
    assert!(explain_grbl("ALARM:2").contains("soft limit"));
    assert!(explain_grbl("error:22").contains("velocità F"));
    assert_eq!(explain_grbl("error:99"), "vedi la documentazione GRBL");
}

#[test]
fn marlin_gets_no_reset_no_polling_and_one_line_at_a_time() {
    use std::net::TcpListener;
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let srv = std::thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        s.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
        let mut got = Vec::new();
        let t = Instant::now();
        let mut buf = [0u8; 256];
        let mut answered = 0;
        while t.elapsed() < Duration::from_secs(3) {
            if let Ok(n) = s.read(&mut buf) {
                got.extend_from_slice(&buf[..n]);
            }
            // answer one ok per received line, slowly: synchronous mode must never have two lines pending
            let lines = got.iter().filter(|&&b| b == b'\n').count();
            if lines > answered + 1 {
                panic!("two lines in flight");
            }
            if lines > answered {
                std::thread::sleep(Duration::from_millis(50));
                s.write_all(b"ok\n").unwrap();
                answered += 1;
            }
        }
        got
    });
    let c = Connection::open_flavor(&format!("tcp://127.0.0.1:{port}"), 0, false, "marlin").unwrap();
    std::thread::sleep(Duration::from_millis(500));
    c.stream("G0 X1\nG1 X2 F600\nG1 Y3\n".into());
    assert!(wait(&c, 3, |s| !s.busy && s.acked == 3), "job not finished");
    drop(c);
    let got = srv.join().unwrap();
    assert!(!got.contains(&b'?'), "status polled on Marlin");
    assert!(!got.contains(&0x18), "soft reset sent to Marlin");
    assert!(!String::from_utf8_lossy(&got).contains("$$"));
}
