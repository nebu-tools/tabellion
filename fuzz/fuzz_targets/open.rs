//! Any bytes: opening, reading every part and saving must not panic, and a
//! document saved with no change is the bytes it was read from.
#![no_main]

use libfuzzer_sys::fuzz_target;
use nebu_docx::{Document, Limits};

fuzz_target!(|data: &[u8]| {
    let limits = Limits {
        max_part_size: 1 << 20,
        max_total_size: 8 << 20,
        ..Limits::default()
    };
    let Ok(mut doc) = Document::from_bytes_with_limits(data, limits.clone()) else {
        return;
    };
    assert!(doc.to_bytes().is_ok_and(|b| b == data));
    let parts: Vec<String> = doc.parts().map(String::from).collect();
    for p in &parts {
        let _ = doc.read_part(p);
        let _ = doc.xml(p);
        let _ = doc.relationships(p);
        let _ = doc.content_type(p);
    }
    // Parsing parts is no change either.
    assert!(doc.to_bytes().is_ok_and(|b| b == data));
    // A changed document reads back, with the change.
    let main = doc.main_part().to_string();
    let Ok(t) = doc.xml(&main) else { return };
    let root = t.root_element();
    if t.set_attribute(root, None, "fuzzed", "1").is_err() {
        return;
    }
    if let Ok(out) = doc.to_bytes() {
        match Document::from_bytes_with_limits(out, limits) {
            Ok(mut again) => {
                let t = again.xml(&main).expect("the changed part parses");
                let root = t.root_element();
                assert_eq!(
                    t.attribute(root, None, "fuzzed").unwrap().as_deref(),
                    Some("1")
                );
            }
            Err(nebu_docx::Error::Limit { .. }) => {}
            Err(e) => panic!("the saved document does not open: {e}"),
        }
    }
});
