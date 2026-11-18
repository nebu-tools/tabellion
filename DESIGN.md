# tabellion design

tabellion builds and edits Word documents. It fills a Word template with
content from R, Typst, Markdown or JSON, edits existing documents as tracked
changes, converts between .docx and other formats, and reads tables and text
back out. It works without Word, and it does not change what it does not
touch: every untouched part of a document stays byte-identical.

tabellion is a Rust engine with a command line and R bindings.

## Overview

The code is one Cargo workspace in this repository:

| Crate or package | Path | Owns |
| --- | --- | --- |
| `nebu-docx` | `crates/nebu-docx` | The OOXML package and the document: read, query, patch, write. Content model and its writer. Tracked changes. Field results. Table styles. No tabellion workflow. |
| `tabellion-typst` | `crates/tabellion-typst` | Typst to the content model, and the content model to Typst. Depends on the `typst` crates. |
| `tabellion` | `crates/tabellion` | Packs, edit operations, conversions (Markdown, JSON, Typst), comparison, the `tabellion` command line. |
| R package `tabellion` | `r/tabellion` | The R interface, through extendr. |
| Python package (later) | `py/` | The same entry points through PyO3. |

`nebu-docx` is the organisation's shared .docx library. The collaborative
Typst editor uses it for Word import and export, and the siglum add-in's
tooling uses it to refresh citations in a closed file. So its interface
speaks about documents, blocks, styles and revisions, never about packs,
templates to fill or operations files. `tabellion-typst` depends on
`nebu-docx`; `nebu-docx` does not depend on Typst, so a user of the engine
does not compile the Typst compiler.

## The engine: `nebu-docx`

### How it keeps what it does not touch

Two layers make the round trip exact.

The package layer reads the zip once and keeps the file's bytes. It reads
and writes the zip format itself, as the XML layer owns its XML, because no
zip library keeps an archive's records as they were. It promises two
things:

1. A document saved with no change is the input file, byte for byte, the
   whole file. A part that was parsed but not changed is no change.
2. After a change, every entry that did not change keeps its complete
   record as it was: the local header with its extra fields, the
   compressed data and the data descriptor, and its central directory
   record apart from the offset of the local header. The entries keep their
   order, in the central directory and in the file, and the bytes before the
   first entry and the archive comment stay. Changed and new entries are
   written fresh. A changed entry keeps its name, date, compression method
   (stored stays stored, anything else is deflated), file attributes and
   comment, and loses its extra fields and data descriptor. New entries go
   after all others.

Whatever a file holds that the engine cannot carry through faithfully, it
refuses when the file is opened, with an error that names it, and never
changes silently: encrypted entries, archives split over several disks,
bytes between entries or between the last entry and the central directory,
an end record that is not at the end of the file, a local header that names
another entry or does not match its directory record, a data descriptor
that does not match, and entries that overlap. An archive comment that
contains an end of central directory signature is refused, because a reader
that searches from the end could take the comment for the end record and
read another archive.

Each local header must agree with its central directory record in the name,
the compression method, the encryption, data descriptor and UTF-8 flags, the
CRC and both sizes, with zip64 fields read on both sides. The one difference
allowed is the one the zip format allows: with a data descriptor, the local
header may give zero for the CRC and the sizes. The descriptor must then
agree with the directory record. A descriptor may start with a signature or
not and may hold 4-byte or 8-byte sizes. The engine reads it in each of
these layouts and takes the one whose fields equal the directory record. If
two fit, the next record decides, since records follow each other without
gaps. If it does not decide, the archive is ambiguous and refused. Each of
these errors names the entry and the field.

Content types and relationships are parts like any other; adding an image
adds one `Relationship` element and, if needed, one `Override` element, and
leaves the rest of those files as they were.

The XML layer is a lossless tree. The parser records, for every node, the
byte range it came from. The writer copies that range for every node that
did not change. A changed node is written anew, and only as far as the
change reaches: an element whose children changed still copies its own
start and end tags, and an element with one changed attribute copies its
name and its other attributes as written. Attribute order, quote style,
namespace declarations, references and white space of every untouched node
are kept. New content enters the tree as a parsed fragment and is written as
it was generated. A tracked replacement in a 5 MB `document.xml` therefore
changes a few hundred bytes, and everything before and after them is
identical.

Whatever the edits, the written part is well-formed and parses back to the
same tree. Each piece the writer puts out is well-formed alone, so the only
danger is where two pieces of character data meet that did not meet in the
source: new text next to old text, or two old texts after what stood
between them was removed. Two such pieces can join into `]]>`, which text
may not contain, or into a carriage return and a line feed, which a reader
takes for one line end. The writer escapes the one character that would
join them: a `]` of new text as `&#93;`, or else the `>` of old text as
`&gt;`, and a line feed after a carriage return as `&#10;`. Comments,
processing instructions and CDATA sections are never written anew, so they
cannot gain a `--`, a `?>` or a `]]>`: they enter the tree only through the
parser, and an edit can only move, copy or remove them. Edits that cannot
be written are refused when they are made: a name that is not an XML name,
a character XML forbids in text, a value or a namespace, a prefix bound
twice in a fragment's bindings, and the XML declaration moved or copied into
an element.

A part keeps its encoding. The parser reads UTF-8, with or without a byte
order mark, and UTF-16, which are the encodings Office Open XML allows, and
the writer writes the same encoding back.

### Namespaces

The XML layer resolves namespaces as the XML namespaces recommendation
defines them. A caller asks for an element by namespace and local name, for
example `(W, "p")`, and finds it whatever prefix the file uses. A document
that binds the WordprocessingML namespace to another prefix, or makes it the
default namespace, works like any other.

New elements and attributes are written with a prefix that is bound to their
namespace where they are inserted, so a paragraph added to a file that uses
`x:` for WordprocessingML is written as `x:p`. If no prefix is bound to a
namespace there, the writer declares one on the new element. A parsed node
that moves to a place where the prefixes it uses mean something else is
written again with the prefixes of its new place.

### Untrusted input

The engine also runs on a server, on files that strangers upload. A
document has limits, with defaults that suit documents far larger than
people write and that a caller can raise. They bound it for its whole life,
at open and under edits:

| Limit | Default |
| --- | --- |
| Entries in the zip archive | 10,000 |
| Decompressed size of one part | 256 MiB |
| Decompressed size of all parts read | 1 GiB |
| Nesting depth of an XML part | 256 |
| Nodes in one XML part: elements, text, comments, CDATA sections, processing instructions and the XML declaration | 10 million |

Each limit is checked in one place: the entry count when the archive is
read, the part sizes where a part is decompressed, the node count where the
tree makes a node of any kind, before it makes it, and the depth where the
parser opens an element and where an edit attaches a node. A tree keeps the
limits it was opened with. Its node count covers the nodes that edits
create, copy or parse from a fragment, and an insertion, move or fragment
that would nest elements deeper than the limit is refused. A subtree that is
not attached is held to the depth limit as well. An edit over a limit gives
the same error as an open over it, which names the `Limits` field to raise,
and leaves the tree as it was. A tree parsed on its own has
`Limits::default()` unless the caller passes others.

Decompression stops at the limit, whatever size the zip headers claim, so a
zip bomb costs no more than the limit. When a part is first read, its CRC
and size are checked against the bytes it decompresses to. Parts are
decompressed only when they are read, and an unchanged part is copied
without being decompressed at all. Its local header and descriptor have
been checked against its directory record at open, so the copy says what
the directory says. The parser and the writer use no recursion, so depth
costs no stack. Zip entry names with `..` segments, a leading `/`, a
backslash or control characters, and names that differ only in case, are
refused.

The parser refuses every document type declaration (`<!DOCTYPE`), with or
without an internal subset, as an error at its place; Office Open XML
forbids them. It never reads a DTD, internal or external, so no entity is
ever defined or expanded, and a part cannot grow into more than its own
bytes. The only references it knows are the five predefined entities and
character references, which may have any number of leading zeros and must
name a character XML allows. On the cases of the W3C XML Conformance Test
Suite that apply to such a parser (XML 1.0 fifth edition and Namespaces
1.0, without a DOCTYPE) it agrees with the suite on every one.

No input may make the engine panic. Every malformed file gives an error
that says what is wrong and where. Tests feed the engine truncated and
damaged archives, zip bombs, deep nesting and malformed XML, and four fuzz
targets in `fuzz/` run opening, parsing, random edit sequences and every
view on random bytes.

### The document model

The engine does not copy the document into a separate object model. It
gives read-only views over the tree and keeps everything else as it is.
`Document::views()` reads every view in one walk and returns them
borrowed; each item of a view says where it is (a block id, a part, a
story) and gives what a reader needs (text, cells, codes, authors).

- `Views::blocks()` lists the top-level children of the body: paragraphs,
  tables, block content controls, the final section properties, bookmarks
  between blocks, and anything else (custom XML, an element no view knows).
  Each block has a `BlockId`. A paragraph with a `w14:paraId` gets
  `p:<paraId>`, a table `t:<paraId of its first paragraph>`; these survive
  a save by Word. Paragraph ids are read as Word reads them, as numbers
  below `0x80000000` in up to eight hexadecimal digits of either case, and
  always written as eight capital digits, so `p:4956fe` and `p:004956FE`
  name one block. Other blocks get `#<index>`, which holds only for the
  file they were read from, so the operations format pairs such ids with
  the file's SHA-256 (`Document::sha256()`). Two blocks that share a
  paraId, which copying in some editors produces, both get `#<index>` and
  are marked `shared_para_id`; `tabellion inspect` lists them. A value
  Word would not keep (above `7FFFFFFF`, or not hexadecimal) gives no id.
  A block gives its kind, style, outline level, list, the text a reader
  sees and, for a paragraph, its content.
- A paragraph's content is a flat list: runs of text with their direct
  formatting, the toggle properties that are on after the style cascade
  and the tracked change they are in, and the inline objects between
  them. Fields, hyperlinks, content controls, bookmarks and comment ranges
  come as a start and an end around their content; note references,
  drawings, equations, breaks and unknown elements as single items. A flat
  list needs no recursion to build, read or serialise, whatever the
  nesting. The blocks give it for the paragraphs of the body's top level;
  paragraphs in tables, block content controls and text boxes give their
  text through those views, and their run-level content is not listed yet.
- `Views::tables()` places cells on the grid, resolving `gridSpan`,
  `vMerge`, `hMerge`, `gridBefore` and `gridAfter`. Each cell appears once
  with its row, column and spans and its own text, paragraphs separated by
  line ends; `rows` gives the text of every grid position, with `null`
  where a merged cell covers it. Nested tables are tables of their own in
  the same flat list, with the cell that holds them, and the cell lists
  their indexes. Rows deleted by a tracked change are left out.
- `Views::images()` lists inline and floating pictures, DrawingML and
  VML, with their relationship, part or link, content type, size in points,
  name, title and alternative text. Charts, shapes, SmartArt and text boxes
  are drawings in the paragraph content; a text box's drawing carries its
  text.
- `Views::fields()` lists complex fields (`fldChar` begin, separate, end,
  nested and split over runs and paragraphs) and simple fields
  (`fldSimple`), with code and result and the field they are nested in. A
  field nested in a code appears in that code as `{ code }`, as Word shows
  it with field codes on, and has a result of its own.
- The other lists give bookmarks with the text they enclose; comments with
  author, initials, date, text, the text they are about, the comment they
  answer (from `commentsExtended`) and whether they are resolved; notes with
  their text, the number Word shows and the block that calls them, in the
  order of their references; content controls with kind, tag, title, whether
  they show their placeholder, and their text.
- `Views::revisions()` lists every `w:ins`, `w:del`, `w:moveFrom`,
  `w:moveTo`, the marks on paragraphs, rows and cells, and the property
  changes (`rPrChange`, `pPrChange`, `tblPrChange`, `trPrChange`,
  `tcPrChange`, `sectPrChange`), with author, date and the text they affect.
  They are grouped as Word reports them: neighbours of one kind, author and
  date with nothing between them are one revision, content inside a row
  inserted or deleted as a whole belongs to the row's revision, and the
  property changes of rows and cells belong to the change of their table.
  A deletion over another author's insertion is two revisions over the
  same text.
- `Views::styles()` gives paragraph, character, table and numbering
  styles with id, stored name, the name Word shows in English, `basedOn`,
  `next`, linked style, outline level and list, the last two through the
  styles a style is based on. `Styles::find` takes an id, a stored name or
  the English name Word shows, without case: Word stores built-in names in
  English (`heading 1`) whatever the interface language, and shows
  `Heading 1`. `Styles::find_of` does the same among the styles of one
  kind, because a file may give a paragraph style and a table style one
  name; an optional kind in one method would hide two behaviours behind an
  argument. Every lookup is a hash lookup.
- `Views::numbering()` gives the abstract definitions with their levels
  (start, format, text, style, restart, legal numbering), and instances with
  their level overrides, as read. Computing the number Word shows for a list
  paragraph needs these and the order of the paragraphs; it comes with the
  field update, in a later change.
- `Views::sections()` gives each section's page size and orientation,
  margins, columns, the usable text width, how the section starts, the first
  and last block, and its headers and footers by kind (default, first, even)
  with their text, including those a section takes over from the one before.
- `Views::locate()` finds the one block a `Locator` names: a block id, the
  block where a bookmark starts (names compare without case, as in Word),
  a heading by its text, the paragraph that holds `{{name}}`, or the block
  whose text contains a string. Every kind goes through one function that
  collects all matches; none or more than one is `Error::Invalid`, which
  names the candidates' ids. A `Locator` is made only by parsing or by
  constructors that refuse an empty or malformed name.

What the engine does not model it keeps. A view that meets an element it
does not know reports it as an opaque block or inline item with its name,
and does not read its content; it never drops it from a listing. Edits
refuse to cut through such objects. For example, a tracked replacement
refuses to delete text that contains a field, a drawing or a note
reference, and says so.

#### One walk, one definition of visible text

Every view comes from one walk over the stories: the body, headers and
footers, footnotes, endnotes, comments, and the text boxes inside each of
them, which are walked after the story that holds them. Nothing else reads
the stories.

The walk records each story as a sequence of pieces: runs of text (`w:t`,
tabs, symbols, hyphens, the letters of an equation), breaks, paragraph
ends, the starts and ends of cells and the ends of rows, and note marks.
Each piece carries what decides whether a reader sees it: the revisions
around it (inserted, deleted, moved away, moved here, any of them at once,
and for a paragraph end, a row or a cell the mark on it too), whether it is
hidden, the number of field codes it is in, and the number of tables it is
in. Field codes are not pieces; the code of each field is kept with the
field.

One function turns a range of pieces and a view of revisions into text.
Every text the views give is such a range: a block, a cell, a bookmark, a
comment and its anchor, a note, a revision, a field result, a content
control, a header or footer, a text box, and `Views::text` in every mode.
The function decides:

- field codes are not text; field results are. The result of a field
  nested in a code is not text of the story, but it is its own field's
  result;
- hidden text is not visible, and a paragraph whose mark is hidden runs on
  into the next one, as in Word;
- tracked changes follow a view of revisions: `accepted` (inserted text in,
  deleted and moved-away text out), `rejected` (the other way round) or
  `current` (everything the file holds). A deleted paragraph mark joins its
  paragraph to the next one, and a deleted row leaves the table, under
  `accepted`;
- a range is read from the outermost table level it touches, counting the
  point where it begins. There a paragraph or a break ends a line; the
  cells of a table one level down are separated by tabs, one more for each
  grid column a cell spans and each column a row skips, and its rows end
  lines; paragraphs and breaks inside one of those cells, and everything
  deeper, are joined by single spaces. So a table in the body reads as rows
  of tab-separated cells, a bookmark over two cells reads with a tab
  between them, and a cell's own text has a line for each of its
  paragraphs (without the tables nested in it, which are tables of their
  own).

Of `mc:AlternateContent`, the walk reads the branch Word takes: the first
`mc:Choice` whose required namespaces Word understands, else
`mc:Fallback`, so a text box written in both branches is read once. Tabs
and line breaks are `\t` and `\n`; a non-breaking hyphen is U+2011 and an
optional hyphen U+00AD; a symbol (`w:sym`) is the code its font gives it.
An item's text has no line end at its end; the text of a revision keeps
it, so a deleted row's text ends with one.

#### Toggle properties and the style cascade

Bold, italic, hidden text and the other toggle properties of ECMA-376
part 1, 17.7.3 (`b`, `bCs`, `i`, `iCs`, `caps`, `smallCaps`, `strike`,
`dstrike`, `outline`, `shadow`, `emboss`, `imprint`, `vanish`) are resolved
in one place, in the order of 17.7.2: document defaults, then the table
style, the paragraph style and the character style, then direct
formatting. Each style states a property or says nothing of it; within one
style's `basedOn` chain the nearest style that states it decides. Between
the levels of styles a property toggles: a style that sets it on turns it
over, one that sets it off leaves it as it was. Direct formatting sets it
outright. A missing paragraph or table style is the default of its kind.
The paragraph mark goes through the same cascade from `w:pPr/w:rPr`, which
decides whether the mark is hidden.

Word agrees with this on every case of the corpus (`cascade.docx`, made in
Word, and `edge.docx`, written by hand), which compares the views with
Word's `Range.Font.Hidden`, `.Bold` and `.Italic`: hidden text in a
paragraph style with a character style that says nothing of hiding stays
hidden; a paragraph style and a character style that both hide text, or
both make it bold, cancel out; a table style and a paragraph style that
are both bold cancel out; two styles of one `basedOn` chain that are both
bold give bold; a character style that sets bold off leaves a bold heading
bold; and direct formatting decides over each of these. Document defaults
are not among the cases yet.

The document keeps what it read until an edit changes a tree or a part is
added: each tree counts the edits that reach it, and the views are read
again when a count differs. The first call to `Document::views` costs one
walk over every story; later calls cost nothing. `Views::text` renders
the pieces again for its options, in time in proportion to the text.

The walk and the collectors use no recursion: they keep their own stacks,
so a table nested two thousand deep or three thousand nested fields cost
memory in proportion to the file and no call stack. The ranges of
bookmarks, comment anchors, field results, content controls and revisions
may overlap and nest, so all their texts together may hold no more than
`Limits::max_total_size` bytes; a file that asks for more gives
`Error::Limit`. A table with more grid positions than sixteen per cell (and
more than 65,536 in all) is refused as `Error::Unsupported`. The views are
read together, so such an error comes from `Document::views` for that
file.

The work of the views grows in proportion to the document. Tests count it
without measuring time (`crates/nebu-docx/src/view/linear.rs`): every
access to a node of a tree and every step of the loops over lists in
memory adds to a counter in test builds, and a document of 4n styles,
latent styles, comments, bookmarks, paragraphs, tables, fields or text
boxes may cost no more than five times one of n. Lookups that a loop over
the input makes (styles by id and name, latent styles, default styles,
block ids, open bookmarks and fields) are hash lookups.

#### The JSON shapes

Every view serialises to one JSON shape, which the command line prints with
`--json` and the R package returns. `crates/nebu-docx/schema/views.json`
gives them as a JSON Schema, and a test validates every view of every
corpus document against it, so a field the schema does not name fails the
build. The output of one view is an object with the file's SHA-256 and one
key named after the view:

| Key | Items |
| --- | --- |
| `blocks` | `id`, `shared_para_id`, `index`, `kind`, `style`, `style_name`, `level`, `numbering`, `text`, `element`, `control`, `content` |
| `styles` | `id`, `name`, `display_name`, `local_names`, `kind`, `built_in`, `default`, `hidden`, `based_on`, `next`, `linked`, `outline_level`, `numbering` |
| `tables` | `index`, `id`, `location`, `parent`, `style`, `row_count`, `column_count`, `cells`, `rows` |
| `fields` | `index`, `kind`, `code`, `result`, `simple`, `parent`, `location` |
| `revisions` | `index`, `kind`, `target`, `author`, `date`, `text`, `location` |
| `comments` | `id`, `author`, `initials`, `date`, `text`, `anchor`, `parent`, `done`, `location` |
| `notes` | `kind`, `id`, `number`, `text`, `location` |
| `bookmarks` | `name`, `id`, `hidden`, `text`, `location` |
| `images` | `index`, `placement`, `relationship`, `part`, `link`, `content_type`, `width`, `height`, `name`, `title`, `alt`, `location` |
| `content_controls` | `kind`, `tag`, `title`, `placeholder`, `text`, `location` |
| `sections` | `index`, `first_block`, `last_block`, `start`, `page_width`, `page_height`, `orientation`, `margins`, `columns`, `text_width`, `title_page`, `headers`, `footers` |
| `numbering` | `definitions` (with `levels`) and `instances` (with `overrides`) |

A `location` is the kind of story, the part and the block id. Lengths are
points. An item of `content` has a `type`: `text`, `field_start`,
`field_end`, `hyperlink_start`, `hyperlink_end`, `content_control_start`,
`content_control_end`, `bookmark_start`, `bookmark_end`, `comment_start`,
`comment_end`, `comment_reference`, `note_reference`, `drawing`, `math`,
`break` or `opaque`; a `text` item lists its `toggles` that are on.
Names are plain English, not the file format's.

#### The names of built-in styles

The names come from Word. `tools/word/style-names.ps1` asks a Word for the
name it shows (`Style.NameLocal`) of each of its 376 built-in styles and,
from a file it saves, the name it stores, and writes
`crates/nebu-docx/src/view/names/<language>.tsv`. The engine reads that
file as it is (`include_str!`), and a test checks that the table it uses
is the file, line for line. The table that remains is English (`en`), from
Word 2021 with an English display language: it gives the display name, so
that `annotation text` is `Comment Text` and `Normal Table` is
`Table Normal`.

German and French names are gone. They were typed by hand, and the Word
used here cannot produce them: it has no French display language, and
setting German as `UILanguageTag` in the registry did not make a new Word
start in German; the next step would change Office account settings that
roam, which these scripts do not touch. To add a language, run
`tools/word/style-names.sh TAG` on a Word whose display language is that
language (set in Word under File, Options, Language), and add the file to
`TABLES` in `names.rs`; its names then appear as `local_names` and
`Styles::find` finds them. Styles of a document's `w:latentStyles` that
the table does not have are found by their stored name.

### Writing content: the content model

Every authoring path produces the same content model: blocks (paragraph,
heading, list, table, figure, image, display equation, page break, raw
OOXML) and inlines (text with marks, footnote, cross-reference, link, inline
math, line break, field, image, raw OOXML). The model says what the content
is. It has a JSON form, so the command line and other languages can produce
it directly.

The writer turns the model into WordprocessingML in the target document's
own styles. `Roles` says which style plays which part: body text, headings
by level, caption, list bullet, list number, footnote text and reference,
figure paragraph, table text, default table style, hyperlink, code.
`Roles::detect` finds Word's built-in styles by their stored names; a pack
can name others. The writer:

- writes captions as Word writes them: the label word, a `SEQ` field, the
  separator and the text, with a hidden bookmark around label and number, so
  that a `REF` cross-reference shows "Table 3" and Word's own cross-reference
  dialog lists the caption;
- writes a cross-reference as a `REF … \h` field, and a reference to a
  numbered equation as "Equation" and a `REF` to its "(n)";
- names the bookmark of a label `_Ref_<label>`, hidden as Word's own are, so
  that a label stays findable after Word saves the file and in a later run;
- writes numbered lists as a fresh numbering instance over the list style's
  definition, so each list starts at 1, and nested items at deeper levels;
- writes footnotes into the footnotes part, creating it with its separators
  if the template has none;
- adds images as media parts with relationships and content types, sized
  from their pixel size and resolution and limited to the text width;
- writes Typst math as Office Math;
- writes tables with the grid computed from row and column spans, per-cell
  shading, alignment and borders, repeated header rows and a real table
  style;
- tracks the insertion when asked: runs in `w:ins`, paragraph marks and rows
  marked as inserted, all under one author and date.

After every write the engine brings field results up to date (see "Fields").

### Public interface

The interface is small on purpose. A caller opens a document, reaches a
part's tree, changes nodes and saves:

```rust
use nebu_docx::{Document, Limits, ns};

// Open and save. Untouched parts and nodes are written as they were read.
let mut doc = Document::open("template.docx")?;            // or from_bytes
let mut doc = Document::open_with_limits("upload.docx", Limits::default())?;
doc.save("out.docx")?;                                       // or to_bytes

// Parts, content types and relationships.
let main = doc.main_part().to_string();                      // word/document.xml
let rels = doc.relationships(&main)?;                        // id, type, target, resolved part
doc.add_part("word/media/image1.png", "image/png", png)?;
let id = doc.add_relationship(&main, IMAGE, Target::Part("word/media/image1.png"))?;

// The tree of a part: find by namespace and local name, edit, save.
let tree = doc.xml(&main)?;
let body = tree.child(tree.root_element(), Some(ns::W), "body")?.expect("a body");
let p = tree.parse_fragment("<w:p><w:r><w:t>Hello</w:t></w:r></w:p>", &[("w", ns::W)])?;
tree.insert(body, 0, p[0])?;
```

The document views add one method on top, read-only. It reads every view
in one walk on first use and returns them borrowed until an edit; each
view serialises to the JSON shapes above:

```rust
let v: &Views = doc.views()?;                               // one walk, then kept
let blocks: &[Block] = v.blocks();                         // id, kind, style, level, text, content
let place: &Block = v.locate(&"heading:Results".parse()?)?; // or Locator::heading("Results")?
let styles: &Styles = v.styles();                           // get(id), find(name), find_of(kind, name)
let tables: &[Table] = v.tables();                          // cells on the grid, nested tables flat
let fields = v.fields();      let revisions = v.revisions();
let comments = v.comments();  let notes = v.notes();
let images = v.images();      let bookmarks = v.bookmarks();
let controls = v.content_controls();
let sections = v.sections();  let numbering = v.numbering();
let text: String = v.text(&TextOptions { revisions: Revisions::Rejected, ..Default::default() });
let sha: String = doc.sha256();                             // names the file #index ids belong to
```

The writer adds:

```rust
let opts = WriteOptions { roles: Roles::detect(doc.views()?.styles()), track: Some(Revision::now("Analyst")), table_styles };
doc.insert(Place::Replace(place.node), &blocks, &opts)?;   // Before, After, Replace, End

revise::replace_in_paragraph(&mut doc, para, "old", "new", Some(&rev))?;
revise::settle(&mut doc, /* accept */ true, Some("Analyst"))?;

doc.ensure_table_style(&TableStyle::Defined(def))?;       // returns the style id
fields::update(&mut doc)?;
```

The tree is public so that other tools can do what the high-level calls do
not, but most users never need it. A tree can also be parsed on its own,
with `Tree::parse` for the default limits or `Tree::parse_with_limits`. Where
each node came from and whether the tree changed are kept inside the crate:
the writer uses them, and callers see their effect in the bytes.

A node handle, `NodeId`, carries the identity of the tree that made it, and
every method that takes one checks it: a handle from another part's tree,
or from a clone, which is a tree of its own, gives `Error::ForeignNode`.
Nodes are never freed while their tree lives, so a handle cannot go stale; a
detached node can be attached again. The reading methods return `Result`
for this reason. No public method panics on any argument.

Errors are one enum, and each message says what was wrong with the file or
the call, so that it can be shown to the person who supplied them: a file
that cannot be read, a damaged archive, a file that is not a Word document,
a missing part, a bad part name, an exceeded limit (with the `Limits` field
to raise), malformed XML (with part, line and column), something not
supported, an edit that would make the tree invalid, and a node handle from
another tree. The document views add `Invalid` (a place that matches
nothing or several blocks, a locator or option that names nothing). The
editing calls will add `ForeignRevision` (another author's pending change
in the target, with the authors' names) and use `Invalid` for an unknown
label and a ragged table.

## The command line

`tabellion` is a Word tool for the terminal and for scripts. Every command
reads and writes files; none needs Word. Text output is for people, `--json`
output for programs, and every JSON shape is the same one the R and Python
bindings return.

```sh
# Look inside
tabellion inspect report.docx                 # outline: headings, tables, figures with ids and captions, counts
tabellion inspect report.docx blocks|styles|tables|fields|comments|revisions|images|bookmarks [--json]
tabellion inspect report.docx notes|sections|numbering|content-controls [--json]
tabellion text report.docx [--revisions accepted|rejected|current] [--headers-footers] [--notes] [--comments] [--no-text-boxes]

# Convert, both ways
tabellion convert report.docx report.typ      # Word -> Typst, in the template's terms
tabellion convert report.docx report.md       # Word -> Markdown (pandoc-compatible)
tabellion convert report.docx report.json     # Word -> content model
tabellion convert notes.md out.docx --template report-template.docx
tabellion convert fragment.typ out.docx --template report-template.docx

# Fill a template
tabellion fill report-template.docx out.docx --pack report.pack.toml \
    --set author="A. Lark" --set date=2026-06-30 --at results --typst results.typ

# Insert and edit
tabellion insert in.docx out.docx --before heading:Methods --typst part.typ [--author A]
tabellion replace in.docx out.docx --find "draft" --with "final" [--block p:1A2B3C4D] [--author A]
tabellion apply in.docx ops.json out.docx     # edit operations, tracked by default

# Revisions
tabellion accept in.docx out.docx [--author A]
tabellion reject in.docx out.docx [--author A]
tabellion compare old.docx new.docx diff.docx [--author A]   # one document with tracked changes

# Images and tables
tabellion images in.docx DIR                  # extract all images
tabellion image in.docx out.docx --target fig-trend --file new.png [--author A]
tabellion tables in.docx DIR --format csv     # extract all tables
tabellion table in.docx out.docx --target tab-sites --csv new.csv [--author A]

# Fields
tabellion fields in.docx out.docx             # update field results without Word

# Development
tabellion roundtrip in.docx out.docx          # proves byte identity
tabellion bench in.docx --find … --with …
```

`inspect` without a view prints an outline: the title, headings indented by
level, tables with their caption and size, figures with their caption, each
with its block id, and counts of tables, images, fields, revisions,
comments, notes and bookmarks. A caption is the paragraph next to the table
or figure that is in the Caption style or holds a `SEQ` field: before a
table first, after a figure first. With a view it prints that view in
aligned columns cut to the terminal's width (`COLUMNS`, else 100), or with
`--json` in the documented shape.

`text` prints paragraphs as lines and a table's rows as lines of
tab-separated cells; a merged cell leaves the positions it covers empty,
and a nested table's cells stand in the outer cell, separated by spaces.
Text boxes follow the paragraph that holds them. `--headers-footers` puts a
section's headers before it and its footers after it (the first page's
first); `--notes` marks note references as `[1]` for footnotes and `[i]` for
endnotes, as Word numbers them, and lists the notes at the end;
`--comments` lists the comments at the end with the text they are about.

Every command exits with 0 on success, 1 when `roundtrip` finds that the
copy differs, 2 for a wrong command line, 3 when a file cannot be read or
written or is not a document the engine can read, and 70 for an internal
error. An error is one line on standard error, never a backtrace.

`insert` takes the place in one of four forms: `--before PLACE`,
`--after PLACE`, `--replace PLACE`, or `--at NAME` for a `{{NAME}}`
placeholder to fill. Without one it appends at the end.

`apply` takes the operations format, a JSON list of changes:

```json
{
  "author": "Analyst",
  "sha256": "only needed for #index block ids",
  "operations": [
    {"op": "set_cells", "target": {"label": "tab-sites"}, "table": {"rows": [["North", "12.1"]]}},
    {"op": "replace", "target": {"block": "t:3F2A11C0"}, "content": {"typst": "…"}},
    {"op": "insert", "at": {"placeholder": "results"}, "content": {"table": {"…": "…"}}},
    {"op": "replace_text", "target": {"heading": "Summary"}, "find": "draft", "with": "final"},
    {"op": "set_caption", "target": {"label": "fig-trend"}, "caption": "Trend, 2020 to 2026"},
    {"op": "replace_image", "target": {"label": "fig-trend"}, "file": "trend.png"},
    {"op": "delete", "target": {"block": "p:0A1B2C3D"}},
    {"op": "update_fields"}
  ]
}
```

Each operation checks its target before anything is written; one failed
check stops the run and writes nothing. Changes are tracked unless the file
says `"track": false`. These rules hold: no change inside another author's
pending revision, no edit to a protected content control (see "Packs")
without `"allow_protected": true`, no cut through a field, a formula or a
section break. `set_cells` changes only the cells that differ and reports
the cells it had to leave, with the reason.

`compare` aligns the two documents' blocks by content (a longest common
subsequence over block signatures, then a word-level diff inside changed
paragraphs and changed cells) and writes the new document with the
differences as tracked changes by one author. Untouched blocks of the new
document keep their bytes. Formatting-only changes become `rPrChange` and
`pPrChange` records.

`convert` to Typst writes what the content model holds, in the vocabulary of
the pack: headings, paragraphs in named styles, lists, tables, figures with
captions and labels, footnotes, references, equations. Content that has no
Typst form, such as a content control or a text box, becomes a call that
keeps the block by id (`#keep("p:1A2B3C4D")`), so that a build from that
Typst copies the block from the source document byte for byte.

## The R interface

The R package is for people who build reports from data. Functions take the
document first and return it invisibly, so they pipe. A document is a handle
to the Rust engine; the R side never holds XML.

Every function that writes content takes one `at` argument for both the
place and the position:

| `at` | Effect |
| --- | --- |
| `end()` | At the end of the body. The default. |
| `"results"` | Fills the `{{results}}` placeholder: the content replaces the paragraph that holds it. |
| `before(heading = "Methods")` | Before the place. |
| `after(bookmark = "summary")` | After the place. |
| `replacing(block = "t:3F2A11C0")` | Instead of the place. |

`before()`, `after()` and `replacing()` name the place with exactly one of
`heading`, `bookmark`, `block`, `placeholder` or `text`.

| Function | Effect |
| --- | --- |
| `open_docx(path)` | Open a template or document. |
| `save_docx(doc, path)` | Write it. Untouched parts stay byte-identical. |
| `end()`, `before(...)`, `after(...)`, `replacing(...)` | A place and a position for `at`. |
| `add_table(doc, data, style, at, caption, label, digits, author)` | A data frame as a Word table. |
| `add_plot(doc, plot, width, height, dpi, caption, label, alt, at, …)` | A ggplot object or a base-graphics function as a figure. |
| `add_typst(doc, source, at, author)` | Typst source, often built with `sprintf` or `glue`, as Word content. |
| `add_text(doc, markdown, at, …)` | Markdown as Word content. |
| `replace_text(doc, find, with, at, author)` | A tracked text replacement. |
| `set_cells(doc, label_or_block, data, author)` | New values in an existing table, cell by cell, tracked. |
| `fill_fields(doc, ...)` | Cover-page and header fields named in the pack. |
| `table_style(name, header, band_fill, top, bottom, inside_h, …)` | A table style defined in R. |
| `typst_table_style(file_or_source, name)` | A table style defined in Typst. |
| `rule(width, color)` | A border line for a table style. |
| `docx_blocks(doc)` | Blocks as a data frame: id, kind, style, level, text. |
| `docx_tables(doc)` | All tables as data frames, with typed columns. |
| `docx_text(doc)`, `docx_revisions(doc)`, `docx_comments(doc)` | Reading back. |
| `accept_revisions(doc, author)`, `reject_revisions(doc, author)` | Settle revisions. |

`style` in `add_table` takes the name of a table style the document has (for
example one the template defines), or a definition from `table_style()` or
`typst_table_style()`. Captions are real `SEQ` fields and labels real
bookmarks, so `@tab-counts` in later Typst or `ref("tab-counts")` in later R
text becomes a `REF` field that Word updates.

A script for a field survey report:

```r
library(tabellion)

doc <- open_docx("samples/report-template.docx")
fill_fields(doc, author = "A. Lark", date = "30 June 2026")

counts <- data.frame(
  Site = c("North", "East", "South", "West"),
  Visits = c(12L, 9L, 14L, 7L),
  Pairs = c(41.5, 33.0, 52.5, 28.0),
  Change = c(0.231, -0.021, 0.118, 0.040)
)

# A table style the template already has.
add_table(doc, counts, style = "Report Table", digits = 2, at = "results",
          caption = "Breeding pairs by site, template style", label = "tab-template")

# A table style defined in Typst, shared with Typst documents.
compact <- typst_table_style("styles.typ", "compact")
add_table(doc, counts, style = compact, digits = 2,
          at = before(heading = "Methods"),
          caption = "Breeding pairs by site, style defined in Typst", label = "tab-typst")

# A table style defined in R.
field <- table_style("Field Green",
  header = list(fill = "#385723", color = "#FFFFFF", bold = TRUE),
  band_fill = "#E2EFDA", top = rule(1, "385723"), bottom = rule(1, "385723"),
  inside_h = rule(0.25, "A9D08E"))
add_table(doc, counts, style = field, digits = 1,
          at = before(heading = "Methods"),
          caption = "Breeding pairs by site, style defined in R", label = "tab-r")

add_plot(doc, function() barplot(counts$Pairs, names.arg = counts$Site),
         width = 14, height = 7, caption = "Mean breeding pairs by site", label = "fig-pairs",
         at = before(heading = "Methods"))

best <- counts$Site[which.max(counts$Pairs)]
add_typst(doc, sprintf(
  "The most breeding pairs were counted at *%s* (%s). @tab-template, @tab-typst and @tab-r show the same counts in three styles; @fig-pairs plots them.",
  best, format(max(counts$Pairs), digits = 3)),
  at = before(heading = "Methods"))

# A tracked change by a named author.
add_typst(doc, "_Added for review._", at = after(bookmark = "summary"), author = "Analyst")

save_docx(doc, "survey.docx")

# Reading back
back <- open_docx("survey.docx")
docx_tables(back)[[1]]    # Site chr, Visits int, Pairs num, Change num
```

The text of the generated paragraph reads, before Word opens the file: "The
most breeding pairs were counted at South (52.5). Table 1, Table 2 and
Table 3 show the same counts in three styles; Figure 1 plots them." The
numbers come from the engine's field update and are what Word shows after
its own update.

Number formatting belongs to the pack (`[numbers]`: decimal mark, thousands
mark, text for missing values, interval and p-value forms), so `add_table`
and the Typst helpers format numbers the same way. Packages that format
tables already (gt, flextable) can hand their result over as a content-model
table later; it is not needed for the first release.

## Typst as an authoring path

A Typst fragment or file goes in; Word content in the template's styles
comes out. The Typst can be written by hand or built in R.

Typst is evaluated in-process with the `typst` crates, and the result is
mapped to the content model. Both sides are Rust, so the content never
leaves memory, and lengths, colours and labels keep their types.

Show rules must apply. They are the normal way to style in Typst, and
styling is what this path is for: a document that says
`#show heading: set text(fill: blue)` or `#show "Ltd": [Limited]` must come
out in Word as it does in Typst. So the mapping reads content after Typst
has applied show rules and set rules, not the content as written. How to
get that content is the first open question (see "Open questions").

### What maps to native Word structures

| Typst | Word |
| --- | --- |
| `= Heading` (any depth), with label | Heading style of that level; label as hidden bookmark |
| paragraphs, `parbreak` | paragraphs in the body style, or a named style |
| `*strong*`, `_emph_`, `underline`, `strike`, `sub`, `super`, `smallcaps`, `highlight` | run properties; highlight as shading, so any colour works |
| `text(fill:, weight:, style:, size:)` set or called | run colour, bold, italic, size |
| `raw` | code character style if the template has one, else a monospace font |
| `- item`, `+ item`, nested | List Bullet / List Number with levels; each numbered list restarts at 1 |
| `terms` | paragraphs with the term in bold |
| `quote` | paragraph in the Quote style |
| `table` with `columns`, `table.header`, `table.cell(x:, y:, colspan:, rowspan:, fill:, align:, stroke:)`, a `fill` colour or function, `stroke`, `table.hline`, `table.vline` | Word table on a grid with `gridSpan` and `vMerge`, cells in the positions Typst gives them, cell shading, alignment, repeated header rows, a table style, and cell borders |
| `figure` with image, table or equation, `caption`, label | caption with SEQ field and bookmark, above tables and below figures (pack decides) |
| `image` (PNG, JPEG, GIF), `width` | inline picture, sized |
| `footnote` | Word footnote |
| `@label` | REF field (to a heading, figure, table or numbered equation) |
| `link("https://…")` | hyperlink with a relationship, Hyperlink style |
| `$ … $` inline and block, labelled block | Office Math; a labelled block equation gets a number "(n)" at the right margin |
| math: fractions, scripts, roots, `lr` delimiters, accents, `op`, primes, big operators with limits (sum, product, integral, union), `mat`, `vec`, `cases` | `m:f`, `m:sSub`/`m:sSup`/`m:sSubSup`, `m:rad`, `m:d`, `m:acc`, `m:nary`, `m:m` |
| `pagebreak` | page break |
| `cite` | a `[@key]` marker that the citation tool turns into a citation content control; a warning says so |

A table is mapped from Typst's own resolution of its grid, so every cell has
its final position, span, fill and stroke on each side, whether these came
from explicit `x` and `y`, a fill function, the table's `stroke`, a cell's
`stroke`, or `table.hline` and `table.vline`. Strokes become Word cell
borders: direct formatting on top of the table style, as per-cell shading
is.

### What does not, and what happens to it

Nothing disappears without a warning. The mapping returns warnings with the
source text of the element, and the R function raises them as R warnings.

- **Rendered as a picture:** drawings (`rect`, `circle`, `line`, `polygon`,
  `curve`), `grid`, `stack`, `columns`, `rotate`, `scale`, `move`, `box` or
  `block` with a fill, border or fixed size, and any other element without a
  native form. Typst lays the element out on its own page and the engine
  embeds the result as a PNG at about 290 pixels per inch, with the source
  as alt text. A later version embeds SVG with a PNG fallback, which Word
  2016 and later show sharply at any zoom.
- **Left to Word's styles, with a warning:** vertical spacing (`v`),
  paragraph spacing and indents set in Typst, page setup.
- **Left out, with a warning:** `outline` (insert a Word table of contents,
  which Word fills), `bibliography` (the citation tool writes the reference
  list), `place`, `hide`, `repeat`, and `context` expressions that depend on
  layout.
- **Approximated, with a warning:** a reference's custom supplement (Word
  shows the caption label), a link to a location in the document (kept as
  text), left-side math scripts (placed on the right), a line break inside
  an equation, a stroke that Word borders cannot draw (a dash pattern Word
  lacks becomes the nearest one).

## Table styles

There is one notion of a table style. A table names its style, and the name
resolves in this order:

1. a definition passed with the content (from Typst, R or JSON), by name;
2. a table style the document already has, by id, name or localised name;
3. the pack's default table style, then Word's "Table Grid".

A definition (`TableStyleDef`) holds the font and size, the outer and inner
borders, cell padding, the header row's fill, colour, weight and bottom
border, a band fill for every other body row, and an optional first-column
look. The engine writes it as a real `w:style w:type="table"` with
`tblStylePr` sections for `firstRow`, `band1Horz` and `firstCol`, and sets
the table's `tblLook` so the header and banding apply. Word lists the style in
the table design gallery, and a Word user can change it or apply it to other
tables. If the document already has a style of that name, the engine uses it
and does not overwrite it, so a later run never undoes a Word user's change to
the style.

The same style from three sources:

```r
# From the template: a style the template defines.
add_table(doc, df, style = "Report Table")

# From Typst: a dictionary bound in a .typ file.
add_table(doc, df, style = typst_table_style("styles.typ", "compact"))

# From R.
add_table(doc, df, style = table_style("Field Green",
  header = list(fill = "#385723", color = "#FFFFFF", bold = TRUE),
  band_fill = "#E2EFDA", top = rule(1, "385723"), bottom = rule(1, "385723")))
```

```typst
#import "@tabellion": table-style, styled-table

#let compact = table-style(
  name: "Compact Data", font-size: 9pt,
  header: (fill: rgb("#7f6000"), color: white, bold: true),
  band-fill: rgb("#fff2cc"),
  top: 1.5pt + rgb("#7f6000"), bottom: 1.5pt + rgb("#7f6000"),
)

#styled-table(compact, columns: 3, table.header[Species][Pairs][Share], …)
```

`styled-table` draws the table in that look when the Typst is compiled to
PDF, and marks it for tabellion, so the Typst preview and the Word result
agree. Per-cell shading and borders from Typst are direct formatting on top
of the style, as in Word.

## Packs

A pack says what a template means. It is a small TOML file that a user
supplies next to the template; the repository ships one neutral sample for
`samples/report-template.docx`. Without a pack, tabellion uses Word's
built-in style names, which is enough for a template that uses them.

A pack holds:

- `[pack]`: name, version, language.
- `[[template]]`: the templates it describes, each with an id, a file name
  and an optional SHA-256 that pins the exact file. A template whose hash
  does not match is refused.
- `[styles]`: style roles (body, headings by level, caption, figure, lists,
  footnotes, table text, default table style, table of contents levels). A
  value names a style by id, by name, or by localised built-in name, or gives
  a pattern (`{ match = "…" }`) when a family of styles plays one part. A list
  role can name both a paragraph style and a numbering style, as some
  templates attach list numbering through numbering styles.
- `[[kind]]`: extra block kinds the template has, by style or by text
  pattern, with flags: `protect` (an edit may not replace or remove it),
  `required` (a build stops if one is missing), `fill` (a placeholder to
  fill), `heading` (counts as a heading without a numbered level).
- `[captions]`: the label words per kind (figure, table, equation) in the
  template's language, the separator, the position above or below, the rule
  for which existing table to copy when the template has a model table, and
  whether a missing figure caption style may be derived from the table one.
- `[fields]`: cover, header and footer fields: the key, the placeholder text
  or pattern in the template, and the parts to search.
- `[numbers]`: decimal and thousands marks, the text for missing values,
  forms for intervals and small p-values, default digits.
- `[checks]`: patterns for unfilled placeholders and example text to report
  after a build.
- `[blocks]`: a Typst file of reusable text blocks with parameters.
- `[protect]`: tag prefixes of content controls that edits must leave alone.
- `[tracking]`: the default author of tracked changes.

Protection is a general rule. A content control whose tag starts with a
listed prefix is protected: an edit may not change, replace or remove it, or
cut through it, unless the caller allows it explicitly. The tags of the
siglum add-in, `siglum:cite:` for citations and `siglum:bibliography` for the
reference list, are protected by default, with or without a pack, so that no
edit loses a citation silently.

## Fields

The engine recomputes, without Word, the results Word would store after
"update fields":

- `SEQ` numbers per identifier, with `\r` (reset to a value), `\c` (repeat)
  and `\h` (hidden), in complex and simple fields;
- `REF` to a bookmark: the bookmark's text after the SEQ update, including
  non-breaking hyphens and tabs;
- `NOTEREF`: the footnote's number;
- `REF \r`, `\n`, `\w`: the paragraph number of the bookmarked paragraph, from
  the numbering engine;
- `STYLEREF`: the text or number of the nearest paragraph in a style;
- the entries of a table of contents and a table of figures: text, levels,
  hyperlinks and bookmarks, rebuilt from headings and captions.

A field update that finds nothing to change writes nothing, so the part stays
the same zip entry.

Only Word can compute what needs layout: page numbers (`PAGE`, `NUMPAGES`,
`PAGEREF`, the page column of every table of contents), `SECTIONPAGES`, and
anything conditional on them. Those keep their last result. Word refreshes
them when the user updates fields or prints; `tools/word/check.ps1` shows how
to do it by automation. Two more fields are Word's alone: `DATE`/`TIME`
results that should show the time of opening, and `INCLUDETEXT` or
`INCLUDEPICTURE` that read other files.

## How correctness is checked

### Without Word, in CI

- **Byte identity.** For every test document, `roundtrip` must give back the
  input file byte for byte, and after an edit every other entry must keep
  its complete local and central directory records, compared whole, and
  every byte outside the edited node must stay. Test archives built byte by
  byte carry data descriptors, zip64 records, stored and deflated entries,
  extra fields, entry and archive comments, an unusual entry order and bytes
  before the first entry.
- **Property tests.** Random well-formed XML with namespaces, references,
  comments, CDATA and processing instructions must write back its bytes;
  random edit sequences must give XML that an independent parser reads as
  the edits say; random content-model documents go through write, read and
  write again. A second oracle runs any sequence of creations, text and
  attribute changes, renames, insertions, moves between parents that bind
  prefixes differently, removals, copies, fragments with clashing bindings
  and handles of another tree: the output must parse, roxmltree must read
  the names, namespaces, attributes and text the tree reports, and every
  node no edit touched must keep its bytes. One run in four uses limits a
  little above what the document needs: the tree must stay within them,
  and an edit they refuse must leave the tree as it was. The `edit` fuzz
  target runs the same oracle.
- **Conformance.** `tools/xmlconf.sh` runs the parser over the W3C XML
  Conformance Test Suite's cases that apply to a parser without DTDs, and
  CI runs it too. A disagreement fails the run unless
  `tests/xmlconf-expected.txt` lists it with a reason.
- **Hostile input.** Truncated and damaged archives, zip bombs, deep nesting,
  malformed XML, duplicate and unsafe entry names give errors, never a panic.
  Each limit is tested with an input of exactly its value and one more, at
  open and for every edit that makes nodes or deepens the tree.
- **Schema validation.** Every output is validated against the ECMA-376
  transitional schemas plus Microsoft's extensions with the Open XML SDK's
  validator (a small .NET tool run in CI). Word's tolerance is not the
  standard; the validator catches element order and attribute errors that
  Word may repair silently or refuse.
- **Golden files.** The content model's JSON for each authoring input is
  compared with a stored copy, and so are the generated XML fragments.
- **LibreOffice as a second reader.** `soffice --headless --convert-to pdf`
  and `--convert-to txt` must succeed on every output, and the text must
  contain the expected paragraphs, captions with numbers, and footnotes. A
  second reader finds the files that only Word can open.
- **Field results.** For documents where Word's results are known (stored
  from a Word run), the engine's field update must reproduce them exactly.
- The views are compared with Word. `corpus/` holds documents with invented
  content and, next to each, what Word reports about it (`NAME.word.json`).
  The tests compare the views with those readings in CI, without Word: the
  text of the body as stored, after accepting and after rejecting all
  revisions, paragraph by paragraph and row by row, empty paragraphs,
  tabs and breaks included; each table as cells of text, nested tables
  too; each field's code and result in the body and the notes, and in
  each section's headers and footers kind by kind, in order; each revision
  with its kind, author and text; each comment with author, initials,
  text, the text it is about and the comment it answers; each bookmark's
  text; the notes; the headers and footers of each section; the text of
  text boxes; pictures with alternative text, title and size; content
  controls with kind, tag, title and placeholder; bold, italic and hidden
  text after the style cascade; and the names of the built-in styles in
  use. The corpus has a bird survey (merged cells both ways, a nested
  table, a table of contents whose result spans paragraphs, `SEQ`, `REF`,
  `PAGE` and nested `IF` fields, a field code split over runs, a field in
  a hyperlink, footnotes and endnotes, an inline and a floating picture, a
  text box in both branches of `mc:AlternateContent`, two sections with
  different headers and footers and a first page of its own), a parts
  catalogue (insertions, deletions, a move, formatting and paragraph
  changes, a deleted and an inserted table row, by two invented authors;
  comments with replies; content controls of the kinds Word makes from its
  interface), field notes with changes (a deletion over another author's
  insertion, tracked changes in a header, a footer and a footnote, a
  comment over two cells, content controls around a paragraph, a row and
  words), toggle properties through styles (`cascade.docx`), a recipe
  collection in German and one in French in built-in styles, a document
  from pandoc, and a document written by hand for what Word does not
  write from its interface (a field nested in a field code, hidden text,
  symbols, special hyphens, breaks, a deleted paragraph mark, a choice of
  `mc:AlternateContent` that Word does not understand, an ignorable
  element of an unknown namespace, custom XML, smart tags, an equation, a
  group content control and controls around a row and a cell, a note with
  its own mark, a legacy VML picture, two paragraphs with one paraId, a
  vertical merge into a row with `gridBefore`, and toggles through a
  `basedOn` chain and a table style). `corpus/build.sh` makes each file
  and the readings; the Word documents are made by `corpus/build/word.ps1`
  in a Word instance of its own, under the invented names Wren Ashby and
  Pip Tanager. LibreOffice was not available where the corpus was made, so
  it has no LibreOffice document yet.
- Where the views report a document differently from Word on purpose, the
  test normalises the difference and names it: Word writes control
  characters for paragraph and cell ends, breaks, note and comment marks and
  pictures, where the views write line ends and tabs, and nothing for a mark
  or a picture; Word gives a symbol as `(`, the views the code its font
  gives it; Word gives the letters of an equation in mathematical italic,
  the views as stored; Word lists a hyperlink element as a `HYPERLINK`
  field, the views as a hyperlink; Word counts a note with its own mark in
  its index of notes, the views give such a note no number; and Word's list
  of cells has one cell for a merged cell and none for the grid columns a
  row skips, where the views write a tab for each grid column, so runs of
  tabs compare as one.
- A test reads every file under `corpus/`, `samples/` and `tools/`, the
  tables of style names, the schema and the documentation, every part of
  every .docx among them, and fails on a user's path, a machine name of
  the form Windows gives or the name of the machine the test runs on, an
  e-mail address outside the example domains, the name of the user the
  test runs as or the name git is configured with, and, in the fields Word
  fills in and in the readings, any author, initials, company, manager,
  user id or identity provider outside the invented names. It cannot tell
  a real name in running text from an invented one.
- Every view of every corpus document is validated against
  `crates/nebu-docx/schema/views.json`.
- Tables nested 2,000 deep, 3,000 nested fields, content controls, revisions
  and hyperlinks run on a thread with a 256 KiB stack; thousands of
  bookmarks open at once meet `max_total_size`; an absurd `gridSpan`, broken
  field structure, unknown elements and random bodies built from the
  elements the walk knows give views or errors and never a panic, and the
  blocks and the text agree on what a reader sees.

### With Word, where it is available

`tools/word/check.sh` runs `tools/word/check.ps1` from WSL against the Word
on the Windows host. For each file it:

1. starts a new, invisible `Word.Application` by COM through
   `tools/word/Word.psm1`, which the corpus scripts share: it opens a blank
   document in that instance, takes its window handle and asks Windows
   which process owns the window, and stops at once without touching Word
   if the instance has documents, is visible, or its process ran before;
2. opens the file with alerts off; a file that needs repair either raises an
   error or makes Word wait on a dialog, which the shell script detects by a
   time limit, after which it stops that one process and reports the file as
   failed;
3. records pages, paragraphs, tables and their styles, the document's table
   styles with Word's visibility flag, revisions, footnotes, and every
   field's code and result; with `TABELLION_WORD_READ=1`, also a reading
   of the whole document with all markup shown: the text of every story,
   the headers and footers of each section, tables as cells, fields,
   revisions, comments, bookmarks, notes, text boxes, pictures, content
   controls and the styles in use, and the text and tables again after
   accepting and after rejecting all revisions;
4. updates all fields in all stories and records the results again, so the
   engine's results can be compared with Word's;
5. saves the updated file, then accepts all revisions and saves, reopens,
   rejects all and saves, and optionally saves a PDF;
6. quits its own Word, and stops that one process if it does not end.

`check.ps1` changes no option of Word. `corpus/build/word.ps1` sets the
user name, initials and "always use these values" for the documents it
makes: `Set-WordOption` records each value before the first change, the
`finally` that puts them back is in place before that change, and each is
put back in its own `try`. If the instance dies first, the module starts a
second instance of its own to put them back. Setting `TABELLION_WORD_FAIL`
makes either script fail half-way, and `tools/word/settings.ps1` prints
every option and registry value concerned, to show that a run leaves them
as they were.

Two deliberately broken files show that step 2 works: malformed XML gives
"Word experienced an error trying to open the file", and a table cell
without a paragraph gives "The file appears to be corrupted". An unknown
element in the WordprocessingML namespace, by contrast, opens without
complaint, so Word opening a file is not proof of validity, which is why CI
validates against the schemas.

Run it with `tools/word/check.sh OUTDIR file.docx…`; set
`TABELLION_WORD_PDF=1` for PDFs and `TABELLION_WORD_TIMEOUT` for the time
limit per file. A release runs it over the test corpus on a Windows machine.

## Installing the R package

Users should not need a Rust toolchain.

- **Binaries through R-universe.** R-universe builds packages with Rust for
  Windows, macOS (both architectures) and Linux, and serves binaries for the
  current and previous R versions. The package is published there from this
  repository; `install.packages("tabellion", repos = c("https://nebu-tools.r-universe.dev", getOption("repos")))`
  installs a binary.
- **Source builds.** The source tarball vendors all crates (`cargo vendor`,
  compressed into `src/rust/vendor.tar.xz`) and builds offline, as CRAN
  requires for Rust packages; `src/Makevars` and `Makevars.win` use the
  vendored crates when the tarball has them and the workspace otherwise. In
  this repository the package is a workspace member and shares the
  workspace's target directory.
- **Size.** The Typst compiler and its embedded fonts make the library large
  (a release build with debug symbols is over 250 MB). Release builds strip
  symbols and use `lto = "thin"`; fonts are needed only for elements that
  become pictures and can move behind a Cargo feature that R-universe
  enables, so a build without it still maps Typst content.
- **Dependencies.** The R side uses only base R and jsonlite; extendr's list
  conversions can replace jsonlite later.

## Performance

Targets, for the generated document of 300 chapters (271 pages in Word, 300
tables, 1,200 fields and 300 footnotes, a 5.9 MB `document.xml`), on one
core of a current desktop:

| Step | Target |
| --- | --- |
| Open and parse the main part | < 150 ms |
| Read every view (one walk over all stories) | < 100 ms |
| A view after the first | < 10 ms |
| Write unchanged | < 10 ms |
| One tracked replacement | < 20 ms |
| Field update over the whole body | < 100 ms |
| Write after the change | < 50 ms |
| Insert 300 chapters (headings, tables, captions, references, footnotes, lists) | < 1 s |

For comparison, officer 0.7.8, the R package for Word documents, reads the
same document with `read_docx` in 0.21 s and writes it with `print` in
17.7 s, in R 4.6.1 on the same desktop, each the median of three runs timed
with `system.time`. officer reads lazily and does neither tracked edits nor
field updates, so only its read and write times compare.

Measured on a Ryzen 9 5900X while other work kept the load average near 31
on its 24 threads, each the median of five runs in one process
(`cargo run --release -p nebu-docx --example views -- large.docx`):
parsing the main part takes 70 to 115 ms, reading every view 63 to 77 ms,
each later view under 5 ms, and the text alone 26 to 34 ms. From the
command line, which also reads and decompresses the file, `tabellion
inspect large.docx` takes 225 ms, `tabellion text large.docx` 163 ms and
`tabellion inspect large.docx blocks --json` 157 ms, medians of fifteen
runs. Parsing the XML is the largest part; the walk is the second.

`tabellion bench` measures each step as the median of several runs in one
process, reading the file from disk each time. CI runs it on the generated
document and fails when a step exceeds twice its target, so slow changes are
seen when they are made. The generator
(`cargo run --release -p nebu-docx --example generate -- samples/report-template.docx 300 large.docx`)
makes the document from the sample template, so anyone can repeat the
numbers.

## Out of scope

- Laying out pages. Page numbers and page breaks are Word's.
- Rendering .docx to PDF. Word or LibreOffice do that.
- Parity with officer. Features come from the needs above.
- Spreadsheets and presentations. The package layer could serve them, but
  nothing else here does.
- Editing in a browser. The collaborative editor uses the engine; the editor
  itself is another project.
- Running Word macros or VBA, and keeping macro-enabled files' code signed.
- Legacy .doc files. Convert them with Word or LibreOffice first.
- Encrypted documents and documents with information rights management.
- Strict Open XML documents. Word saves them as transitional documents on
  request.

## Open questions

**How Typst content is realised.** Show rules apply during realisation,
which turns content into styled elements. The lead to examine is to call
Typst's own realisation, as Typst's HTML export does: the `typst-realize`
crate exports `realize`, which takes a `RealizationKind` (`Fragment` for
nested content) and returns the content as a flat list of elements, each
with its style chain, after show rules have run; `typst-html` calls it for
its document and for every nested fragment. The mapping would then walk that
output in place of the content as written, and recurse into containers as
`typst-html` does. To settle: whether the HTML target's rules are the right
ones for Word (they keep figures, footnotes and references as elements,
which Word needs, but some elements may realise into HTML-specific forms),
which elements must stay opaque so that the mapping sees them whole (math,
tables, figures, references), and what it costs.

**How Typst tables are resolved.** `typst-library` resolves a table into a
grid in `layout::grid::resolve::table_to_cellgrid`, which returns a
`CellGrid`: an entry per grid position (a cell with its fill, column span,
row span and stroke per side, or a reference to the cell it is merged with),
the horizontal and vertical lines, and the header and footer rows. Mapping
from that grid gives explicit positions, fill functions, strokes,
`table.hline` and `table.vline` already resolved per cell, as Typst itself
draws them. To settle: how to call it outside layout (it needs an `Engine`
and the table's style chain, which realisation provides), and how Typst's
stroke priorities between adjacent cells translate to Word's border
conflict rules.

**When Word restarts a sequence.** Word restarts a `SEQ … \s 1` sequence
after each Heading 1, but some templates show no restart, apparently because
their headings are not numbered by a list. The numbering engine will decide
it from the heading's numbering, and a test against Word's own results pins
the rule down.
