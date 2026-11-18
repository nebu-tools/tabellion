//! Random edit sequences on random documents: the output must parse, mean
//! what the tree says, and keep every untouched node's bytes. The checks
//! are the property tests' own, in `tests/support/edits.rs`.
#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../../crates/nebu-docx/tests/support/edits.rs"]
mod edits;

fuzz_target!(|data: &[u8]| {
    let Some((head, ops)) = data.split_first_chunk::<9>() else {
        return;
    };
    let docs = edits::BOUNDARY_DOCUMENTS;
    let xml = match head[0] as usize % (docs.len() + 2) {
        k if k < docs.len() => docs[k].to_string(),
        _ => edits::document(u64::from_le_bytes(head[1..].try_into().unwrap())),
    };
    edits::run(&xml, ops);
});
