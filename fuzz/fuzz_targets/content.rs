//! The JSON form of the content model on any input: nothing may panic,
//! and content that reads writes back as JSON that reads as the same
//! content.
#![no_main]

use libfuzzer_sys::fuzz_target;
use nebu_docx::{Content, ContentLimits};

fuzz_target!(|data: &[u8]| {
    let limits = ContentLimits {
        max_bytes: 1 << 20,
        max_depth: 16,
        max_items: 10_000,
    };
    if let Ok(c) = Content::from_json_with_limits(data, &limits) {
        let again = Content::from_json(c.to_json().as_bytes()).expect("written JSON reads");
        assert_eq!(again, c);
    }
});
