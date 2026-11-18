//! Any bytes: opening, reading every part and saving must not panic.
#![no_main]

use libfuzzer_sys::fuzz_target;
use nebu_docx::{Document, Limits};

fuzz_target!(|data: &[u8]| {
    let limits = Limits { max_part_size: 1 << 20, max_total_size: 8 << 20, ..Limits::default() };
    let Ok(mut doc) = Document::from_bytes_with_limits(data, limits) else { return };
    let parts: Vec<String> = doc.parts().map(String::from).collect();
    for p in &parts {
        let _ = doc.read_part(p);
        let _ = doc.xml(p);
        let _ = doc.relationships(p);
        let _ = doc.content_type(p);
    }
    let _ = doc.to_bytes();
});
