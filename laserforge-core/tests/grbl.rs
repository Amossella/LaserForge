use laserforge_core::grbl::*;
use std::collections::BTreeMap;

#[test]
fn status_uses_work_coordinates() {
    let mut p = StatusParser::default();
    let s = p.parse("<Idle|MPos:110.000,60.000,0.000|FS:0,0|WCO:100.000,50.000,0.000>").unwrap();
    assert_eq!((s.state.as_str(), s.x, s.y), ("Idle", 10.0, 10.0));
    // WCO is only sent every few reports: it is remembered
    let s = p.parse("<Run|MPos:120.000,70.000,0.000|FS:1000,500>").unwrap();
    assert_eq!((s.x, s.y, s.feed, s.spindle), (20.0, 20.0, Some(1000.0), Some(500)));
    let s = p.parse("<Alarm|WPos:5.000,6.000,0.000>").unwrap();
    assert_eq!((s.state.as_str(), s.x, s.y), ("Alarm", 5.0, 6.0));
    assert!(p.parse("ok").is_none());
}

#[test]
fn settings_and_checks() {
    assert_eq!(parse_setting("$32=0"), Some((32, 0.0)));
    assert_eq!(parse_setting("$110=6000.000 (x max rate)"), Some((110, 6000.0)));
    assert_eq!(parse_setting("ok"), None);
    let mut s = BTreeMap::new();
    for (k, v) in [(32, 0.0), (20, 1.0), (22, 0.0), (13, 1.0), (30, 255.0)] {
        s.insert(k, v);
    }
    let p = problems(&s, 1000);
    assert_eq!(p.len(), 4, "{p:?}");
    s.insert(32, 1.0); s.insert(20, 0.0); s.insert(13, 0.0); s.insert(30, 1000.0);
    assert!(problems(&s, 1000).is_empty());
}

#[test]
fn streamer_fills_but_never_overflows_the_buffer() {
    let mut st = Streamer::default();
    let prog: String = (0..200).map(|i| format!("G1 X{}.5 Y3 F3000 S{}\n", i % 50, i % 9)).collect();
    st.push_program(&format!("; comment\n$X\n{prog}$H\nG0 X0 Y0\n"));
    let mut written = Vec::new();
    let mut max_inflight = 0;
    loop {
        while let Some(l) = st.next_to_send() {
            assert!(st.in_flight_bytes() <= RX_BUFFER || written.is_empty());
            written.push(l);
            max_inflight = max_inflight.max(st.in_flight_bytes());
        }
        if st.on_response().is_none() {
            break;
        }
    }
    assert!(st.done());
    assert_eq!(written.len(), 203);
    assert_eq!(written[0], "$X");
    assert_eq!(written[202], "G0 X0 Y0");
    assert!(max_inflight > 60 && max_inflight <= RX_BUFFER, "{max_inflight}");
}

#[test]
fn system_commands_go_alone() {
    let mut st = Streamer::default();
    st.push_program("G1 X1\n$H\nG1 X2\n");
    assert_eq!(st.next_to_send().as_deref(), Some("G1 X1"));
    assert_eq!(st.next_to_send(), None); // $H waits for the buffer to empty
    st.on_response();
    assert_eq!(st.next_to_send().as_deref(), Some("$H"));
    assert_eq!(st.next_to_send(), None); // nothing joins $H
    st.on_response();
    assert_eq!(st.next_to_send().as_deref(), Some("G1 X2"));
    st.reset();
    assert!(st.done());
}
