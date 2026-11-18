# Fuzz targets

Six targets for [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz),
which needs a nightly toolchain. CI runs each for 30 seconds on every
change:

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
- `views` reads every view of `nebu-docx`, the text with every option and
  every kind of locator. An even first byte makes the rest a file to open;
  an odd one makes it XML for the body of the sample template, with the
  usual prefixes bound. Nothing may panic, and the blocks' text must be the
  text of the document, white space aside.

- `content` feeds any bytes to `Content::from_json`, the reader of the
  content model's JSON form. Nothing may panic, and content that reads
  must write JSON that reads as the same content.
- `write` makes content from the bytes with a small generator of the
  content model (tables with any spans, labels and references that may not
  match, raw OOXML that may not fit, notes in notes) and inserts it into
  the sample template before, after or in place of a paragraph the bytes
  choose, or at the end. Nothing may panic; an insertion that fails must
  leave the document as it was, and one that succeeds must give a document
  that opens again and gives every view.

`fuzz/Cargo.lock` is committed, so a fuzz build uses the versions it was
tested with.

```sh
cd fuzz
cargo +nightly fuzz run xml -- -max_total_time=600
cargo +nightly fuzz run edit -- -max_total_time=600
mkdir -p corpus/open && cp ../samples/report-template.docx corpus/open/
cargo +nightly fuzz run open -- -max_total_time=600
mkdir -p corpus/views && for f in ../corpus/*.docx; do { printf '\0'; cat "$f"; } > "corpus/views/$(basename "$f")"; done
cargo +nightly fuzz run views -- -max_total_time=600
mkdir -p corpus/content && cp ../samples/survey.json corpus/content/
cargo +nightly fuzz run content -- -max_total_time=600
cargo +nightly fuzz run write -- -max_total_time=600
```
