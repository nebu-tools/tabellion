# Tabellion

<img align="right" src="assets/branding/tabellion-logo.webp" alt="Tabellion logo: a hedgehog with pencils among its quills stamps a sheet of paper tied with a ribbon" width="160" height="160">

Tabellion builds and edits Word documents from R and Typst. It does not need
Word, and it changes nothing in a document that it does not touch: every part
and every XML node outside an edit keeps its exact bytes.

<br clear="right">

Tabellion is being rebuilt as a Rust engine with a command line and an R
package. [DESIGN.md](DESIGN.md) describes the whole tool. This repository
holds the first part of it.

## What works today

The engine core, `nebu-docx`, opens a .docx file, gives access to the XML
tree of each part, applies edits to the tree and saves. A document saved
with no change is the input file, byte for byte. After a change, every zip
entry that did not change keeps its complete record as it was (local header,
extra fields, compressed data, data descriptor, and its central directory
record apart from the offset), and the entries keep their order. In a part
that changed, only the edited nodes are written again, and the result is
always well-formed XML. The engine resolves XML namespaces, so a
document that uses an unusual prefix for Word's namespace works like any
other. It has limits on size, depth and node count, which hold at open and
under edits, so it can open and edit files from people you do not know.

The engine also reads a document. It lists the blocks of the body with
stable ids, styles, heading levels and text, and gives the content of each
paragraph: runs with their formatting, and fields, hyperlinks, bookmarks,
comment ranges, note references, drawings, equations and content controls
between them. It places table cells on the grid with merged cells and
nested tables, and lists fields with code and result, tracked changes with
author and text, comments with their replies, footnotes and endnotes,
bookmarks, pictures, content controls, styles (findable by the name Word
shows in English, German or French), numbering definitions and sections
with their headers and footers. Each view has a JSON shape, given in
[`crates/nebu-docx/schema/views.json`](crates/nebu-docx/schema/views.json).
The views are checked against what Word reports for a corpus of test
documents (see [`corpus/`](corpus)).

The numbering is read as the file stores it. Computing the number Word
shows for a list paragraph ("2.1", "b)") comes later, with the field
update.

### The command line

`tabellion inspect` prints an outline of a document:

```console
$ tabellion inspect samples/report-template.docx
report-template.docx: 9 blocks in 1 section

KIND       ID  TEXT
title      #0  Breeding bird survey
heading 1  #2  Summary
heading 1  #4  Results
heading 1  #6  Methods

0 tables, 0 images, 1 field, 0 revisions, 0 comments, 0 footnotes, 0 endnotes, 1 bookmark
```

The sample has no paragraph ids, so its blocks are numbered (`#2`). A file
that Word saved has them, and tables and figures appear with their
captions:

```console
$ tabellion inspect corpus/birds.docx
birds.docx: 27 blocks in 2 sections

KIND       ID          TEXT
title      p:1C0F4262  Breeding bird survey of the Alder Marshes
heading 1  p:40F87D84  Survey sites
table      t:46016ED2  Table 1: Sites, visits and counts (5 × 4)
heading 1  p:004956FE  Species
figure     p:7574F01D  Figure 1: A wren at the north site
heading 1  p:6C2A1ADB  Methods

2 tables, 2 images, 13 fields, 0 revisions, 0 comments, 1 footnote, 1 endnote, 5 bookmarks
```

With a view it prints that view: `blocks`, `styles`, `tables`, `fields`,
`comments`, `revisions`, `images`, `bookmarks`, `notes`, `sections`,
`numbering` or `content-controls`. Add `--json` for the documented shape.

```console
$ tabellion inspect samples/report-template.docx blocks
#  ID  KIND       STYLE      LEVEL  TEXT
0  #0  paragraph  Title             Breeding bird survey
1  #1  paragraph  Body Text         By {{author}}, {{date}}.
2  #2  paragraph  Heading 1  1      Summary
3  #3  paragraph  Body Text         The summary goes here.
4  #4  paragraph  Heading 1  1      Results
5  #5  paragraph  Body Text         {{results}}
6  #6  paragraph  Heading 1  1      Methods
7  #7  paragraph  Body Text         Describe the survey sites, the counting method and the data her…
8  #8  section

$ tabellion inspect samples/report-template.docx bookmarks --json
{
  "sha256": "858d292ea98cd934c7f8445b5e20baa47239d6088dcd26119b553db5cd1ed105",
  "bookmarks": [
    {
      "name": "summary",
      "id": "1",
      "hidden": false,
      "text": "The summary goes here.",
      "location": {
        "story": "main",
        "part": "word/document.xml",
        "block": "#3"
      }
    }
  ]
}
```

`tabellion text` prints the text as a reader sees it: field results and no
field codes, no hidden text, tracked changes accepted. Tables print as
lines of tab-separated cells. `--revisions rejected` or `current` shows the
text with all changes rejected or with deleted and inserted text both;
`--headers-footers`, `--notes` and `--comments` add those, and
`--no-text-boxes` leaves out text boxes.

```console
$ tabellion text samples/report-template.docx --headers-footers
Breeding bird survey
By {{author}}, {{date}}.
Summary
The summary goes here.
Results
{{results}}
Methods
Describe the survey sites, the counting method and the data here.
Page 1
```

`tabellion roundtrip IN OUT` opens a document, saves it unchanged and checks
that the copy is the same file, byte for byte:

```sh
cargo run -p tabellion -- roundtrip samples/report-template.docx /tmp/copy.docx
```

It prints `/tmp/copy.docx: identical to samples/report-template.docx, all 5611 bytes`.

Every command exits with 0 on success, 1 when `roundtrip` finds that the
copy differs, 2 for a wrong command line, 3 when a file cannot be read or
written or is not a Word document tabellion can read, and 70 for an
internal error. A file that cannot be read gives one line on standard
error. A wrong command line gives several lines on standard error: what is
wrong, the usage of the command or the values an option takes, and a
pointer to `--help`.

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

`tools/xmlconf.sh` downloads the W3C XML Conformance Test Suite and runs
the parser over it.

On a Windows machine with Word, `tools/word/check.sh` opens files in a Word
instance of its own and reports what Word makes of them. See DESIGN.md,
"With Word, where it is available". `corpus/build.sh` makes the test
corpus and Word's readings of it with that script.

`cargo run --release -p nebu-docx --example views -- FILE` times opening a
document and each view.

## Licence

[GNU Affero General Public License, version 3](LICENSE).

The logo is a stand-in sketch until a hand-painted one replaces it.
