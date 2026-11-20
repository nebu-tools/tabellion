//! The replacement budget, guarded in release: one tracked
//! replacement under twice the 20 ms budget on the 300-chapter
//! document, and a full sweep that proves the per-story seed walk
//! holds. Run with `cargo test --release -p nebu-docx --test perf --
//! --ignored`; in debug builds the same operations cost several times
//! more, so the suite stays ignored there.

mod support;

use nebu_docx::{Document, Track};

#[test]
#[ignore]
fn tracked_replace_meets_its_budget() {
    let bytes = support::large::generate(&support::sample(), 300).unwrap();
    let rev = Track {
        author: "Bench".into(),
        date: "2026-11-20T12:00:00Z".into(),
    };
    // One single-paragraph replacement: the budget item.
    let mut doc = Document::from_bytes(bytes.clone()).unwrap();
    let views = doc.views().unwrap();
    let para = views
        .paragraphs()
        .iter()
        .find(|p| p.text.contains("habitat"))
        .expect("a match")
        .node;
    let tick = std::time::Instant::now();
    let n = nebu_docx::revise::replace_in_paragraph(&mut doc, para, "habitat", "range", Some(&rev))
        .unwrap();
    assert!(n > 0, "the fixture lost its match");
    let single = tick.elapsed();
    // The whole sweep over every story.
    let mut doc = Document::from_bytes(bytes).unwrap();
    let views = doc.views().unwrap();
    let paras: Vec<_> = views.paragraphs().iter().map(|p| p.node).collect();
    let tick = std::time::Instant::now();
    let report =
        nebu_docx::revise::replace_all(&mut doc, &paras, "habitat", "range", Some(&rev)).unwrap();
    let sweep = tick.elapsed();
    let replaced: usize = report.replaced.iter().map(|(_, n)| n).sum();
    println!("single {single:?}, sweep {sweep:?}, replaced {replaced}");
    assert!(report.refused.is_empty(), "{report:?}");
    assert!(
        single <= std::time::Duration::from_millis(40),
        "single tracked replacement over budget: {single:?}"
    );
    // Generous on absolute time — hardware varies — but tight enough to
    // catch a lost hoisting: an un-hoisted sweep costs minutes here.
    assert!(
        sweep <= std::time::Duration::from_secs(30),
        "sweep over budget: {sweep:?}"
    );
}
