//! `laserforge gcode job.json [-o out.gcode]` : turn a job description into G-code.
//! `laserforge estimate job.json`              : print the estimated job time.

use std::io::Read;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 || !["gcode", "estimate"].contains(&args[1].as_str()) {
        eprintln!("uso: laserforge gcode <job.json|-> [-o file.gcode] [--allow-unported]\n     laserforge estimate <job.json|->");
        std::process::exit(2);
    }
    let mut text = String::new();
    if args[2] == "-" {
        std::io::stdin().read_to_string(&mut text).expect("stdin");
    } else {
        text = std::fs::read_to_string(&args[2]).unwrap_or_else(|e| {
            eprintln!("{}: {e}", args[2]);
            std::process::exit(1)
        });
    }
    let t0 = std::time::Instant::now();
    let job: laserforge_core::Job = serde_json::from_str(&text).unwrap_or_else(|e| {
        eprintln!("job non valido: {e}");
        std::process::exit(1)
    });
    if std::env::var("LF_TIMING").is_ok() { eprintln!("parse: {:?}", t0.elapsed()); }
    let missing = laserforge_core::unported_features(&job);
    if !missing.is_empty() && !args.iter().any(|a| a == "--allow-unported") {
        eprintln!("Questo lavoro usa funzioni non ancora portate in Rust, il G-code sarebbe sbagliato:");
        for m in &missing {
            eprintln!("  - {m}");
        }
        eprintln!("Usa l'app Python, oppure --allow-unported per generarlo comunque (solo per prove).");
        std::process::exit(3);
    }
    let out = laserforge_core::generate(&job);
    if args[1] == "estimate" {
        let s = out.seconds;
        println!("{}m {}s", (s / 60.0) as i64, (s % 60.0) as i64);
        return;
    }
    match args.iter().position(|a| a == "-o").and_then(|i| args.get(i + 1)) {
        Some(path) => std::fs::write(path, &out.send).expect("write"),
        None => println!("{}", out.send),
    }
}
