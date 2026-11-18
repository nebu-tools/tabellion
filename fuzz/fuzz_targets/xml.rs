//! Any bytes as XML: parsing must not panic, and a tree that parses must
//! write back the bytes it was parsed from.
#![no_main]

use libfuzzer_sys::fuzz_target;
use nebu_docx::xml::Tree;

fuzz_target!(|data: &[u8]| {
    if let Ok(tree) = Tree::parse(data) {
        assert_eq!(tree.to_bytes(), data);
    }
});
