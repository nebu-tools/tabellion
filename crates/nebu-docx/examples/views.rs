//! Times opening a document, reading its views and rendering its text, as
//! the median of five runs.
//!
//!   cargo run --release -p nebu-docx --example views -- FILE

use std::time::{Duration, Instant};

use nebu_docx::{Document, TextOptions};

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: views FILE");
        std::process::exit(2);
    };
    let bytes = std::fs::read(&path).unwrap_or_else(|e| {
        eprintln!("views: cannot read {path}: {e}");
        std::process::exit(1);
    });
    type Step = (&'static str, fn(&mut Document));
    let steps: [Step; 4] = [
        ("parse the main part", |d| {
            let main = d.main_part().to_string();
            d.xml(&main).unwrap();
        }),
        ("every view", |d| {
            d.views().unwrap();
        }),
        ("the views again", |d| {
            d.views().unwrap();
        }),
        ("text", |d| {
            drop(d.views().unwrap().text(&TextOptions::default()))
        }),
    ];
    let mut times = vec![vec![]; steps.len()];
    for _ in 0..5 {
        let mut d = Document::from_bytes(bytes.clone()).unwrap();
        for (i, (_, f)) in steps.iter().enumerate() {
            let t = Instant::now();
            f(&mut d);
            times[i].push(t.elapsed());
        }
    }
    for ((name, _), t) in steps.iter().zip(times) {
        println!("{name:<20} {:>8.1} ms", median(t).as_secs_f64() * 1000.0);
    }
}
