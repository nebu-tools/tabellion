# Fuzz targets

Two targets for [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz), which
needs a nightly toolchain:

- `open` feeds any bytes to `Document::from_bytes`, then reads every part,
  parses it as XML and saves. Nothing may panic.
- `xml` feeds any bytes to `Tree::parse`. Nothing may panic, and a tree that
  parses must write back exactly the bytes it was given.

```sh
cd fuzz
cargo +nightly fuzz run xml -- -max_total_time=300
mkdir -p corpus/open && cp ../samples/report-template.docx corpus/open/
cargo +nightly fuzz run open -- -max_total_time=300
```
