# tabellion

tabellion builds and edits Word documents from R and Typst. It does not need
Word, and it changes nothing in a document that it does not touch: every part
and every XML node outside an edit keeps its exact bytes.

tabellion is being rebuilt as a Rust engine with a command line and an R
package. [DESIGN.md](DESIGN.md) describes the whole tool. This repository
holds the first part of it.

## What works today

The engine core, `nebu-docx`, opens a .docx file, gives access to the XML
tree of each part, applies edits to the tree and saves. A part that nothing
changed is saved as the same zip entry. In a part that changed, only the
edited nodes are written again. The engine resolves XML namespaces, so a
document that uses an unusual prefix for Word's namespace works like any
other. It has limits on size and depth, so it can open files from people
you do not know.

The command line has one command, `roundtrip`. It opens a document, saves it
unchanged and checks that every zip entry is identical:

```sh
cargo run -p tabellion -- roundtrip samples/report-template.docx /tmp/copy.docx
```

It prints `/tmp/copy.docx: all 11 entries identical`.

## Build and test

You need Rust 1.88 or later.

```sh
cargo build --release
cargo test
```

The tests use the sample template in `samples/` and a large document that
they generate from it. To write that document to a file:

```sh
cargo run --release -p nebu-docx --example generate -- samples/report-template.docx 300 large.docx
```

On a Windows machine with Word, `tools/word/check.sh` opens files in a Word
instance of its own and reports what Word makes of them. See DESIGN.md,
"With Word, where it is available".

## Licence

[GNU Affero General Public License, version 3](LICENSE).
