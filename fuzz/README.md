# Fuzz targets

Three targets for [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz),
which needs a nightly toolchain:

- `open` feeds any bytes to `Document::from_bytes`, then reads every part,
  parses it as XML and saves. Nothing may panic, and a document saved with
  no change must be the bytes it was read from.
- `xml` feeds any bytes to `Tree::parse`. Nothing may panic, and a tree that
  parses must write back exactly the bytes it was given.
- `edit` makes a random document from the first bytes and reads the rest as
  a sequence of edits: creations, text and attribute changes, renames,
  insertions, moves, removals, copies, fragments with clashing bindings and
  handles of another tree. The output must parse, roxmltree must read in it
  what the tree reports, and every node no edit touched must keep its bytes.
  The checks are those of the property tests, in
  `crates/nebu-docx/tests/support/edits.rs`.

```sh
cd fuzz
cargo +nightly fuzz run xml -- -max_total_time=600
cargo +nightly fuzz run edit -- -max_total_time=600
mkdir -p corpus/open && cp ../samples/report-template.docx corpus/open/
cargo +nightly fuzz run open -- -max_total_time=600
```
