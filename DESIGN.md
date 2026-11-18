# tabellion design

tabellion builds and edits Word documents. It fills a Word template with
content from R, Typst, Markdown or JSON, edits existing documents as tracked
changes, converts between .docx and other formats, and reads tables and text
back out. It works without Word, and it does not change what it does not
touch: every untouched part of a document stays byte-identical.

This document describes the rebuild: a Rust engine with a command line and R
bindings. The spike in `spike/` and the code in `crates/` and `r/` prove the
risky parts. The R code in `R/` is the current tool and stays until the
rebuild replaces it.

## What the current tool does

The current tool is about 4,700 lines of R and Typst. It is worth knowing
what it does well, because the rebuild keeps all of it.

- `R/scan.R` slices `word/document.xml` into top-level body blocks with a tag
  scanner, so each block keeps its original bytes. `R/build.R` writes a block
  back verbatim when its content did not change and rebuilds it only when it
  did. `R/package.R` copies all other parts of the package unchanged.
- `R/import.R` turns a template into a Typst file: one Typst call per block,
  with the block's id. `R/typst.R` runs `typst eval` with a `query` for labelled metadata, which
  returns the evaluated content of `metadata` elements as JSON. `R/typst.R`
  and `R/para.R` map that JSON to a token list (text with bold, italic,
  underline, sub and superscript; opaque "atoms" for fields, drawings and
  footnote references; bookmarks), and `render_para` writes the tokens as runs
  in the prototype paragraph's style. Lists, headings, figures, tables and
  equations (`R/math.R`, Typst math to OMML) are handled as block tokens. So
  today Typst content reaches Word by way of the content tree that Typst
  evaluates, serialised to JSON and mapped in R. Layout plays no part.
- `R/ops.R` applies JSON operations to an existing document: change table
  values, replace a table, figure or text block, fill a `{{placeholder}}`,
  change a caption title. Each change is a tracked change by a named author.
  It refuses to touch a target that holds another author's pending revision,
  a citation, a formula or a section break. Revision and bookmark ids are
  taken above the highest id in the file.
- `R/resolve.R` recomputes what Word would store after "update fields": SEQ
  numbers, REF and NOTEREF results, paragraph numbers for `REF \r`, and the
  text of every table of contents. Page numbers need layout and stay as they
  were.
- `R/zitate.R` reads the citation content controls of the siglum Word add-in.

The weaknesses are the ones the audit found. The XML edits are string and
regular-expression operations in R, with xml2 for some nodes. Block kinds
are found by German style ids such as `berschrift1` and `Beschriftung`. The API and the JSON keys are German.
The code knows one publisher's templates. And it is slow: reading and parsing
the largest template takes 6 to 7 seconds, one tracked edit on it 11 seconds,
and on a generated 279-page document reading alone takes about a minute (see
"Performance").

## Overview

The rebuild is one Cargo workspace in this repository:

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

The package layer (`package.rs`) reads the zip once. A part that nobody asks
for is copied into the output as the same compressed zip entry, CRC and all
(`ZipWriter::raw_copy_file`). A part that is parsed but not changed is also
copied raw. Only parts with real changes are compressed again. Content types
and relationships are parts like any other; adding an image adds one
`Relationship` element and, if needed, one `Default` element, and leaves the
rest of those files as they were.

The XML layer (`xml.rs`) is a lossless tree. The parser records, for every
node, the byte range it came from. The writer copies that range for every
node that did not change. A changed node is serialised, and so are its
ancestors' start and end tags only where needed: an ancestor whose children
changed still copies its own start tag from the source. Attribute order,
quote style, namespace declarations, entity spelling and whitespace of every
untouched node are kept. New content enters the tree as a parsed fragment
and is written as it was generated.

The spike measured this on a tracked replacement in a 5.3 MB
`document.xml`: the output differs from the input in 22 bytes of the old
file, replaced by 295 new bytes. Everything before and after is identical.

Names are handled as written (`w:p`), which is what Word and every other
producer emit. The engine checks at open that the main part binds `w` to the
WordprocessingML namespace, and it declares a missing prefix it needs (`m`,
`wp`, `r`) on the root element before it writes content that uses it. A
document that binds the standard namespaces to other prefixes is refused
with a clear error in this version. Full prefix resolution is a contained
change in `xml.rs` if such files appear.

### The document model

The engine does not copy the document into a separate object model. It
gives typed views over the tree and keeps everything else as it is.

- **Blocks.** `Document::blocks()` lists the top-level children of the body:
  paragraphs, tables, block content controls, the final section properties,
  and anything else (bookmarks between blocks, custom XML, `mc:AlternateContent`).
  Each block has an id. A paragraph with a `w14:paraId` gets `p:<paraId>`, a
  table `t:<paraId of its first paragraph>`; these survive a save by Word.
  Other blocks get `#<index>`, which holds only for the file they were read
  from, so the operations format pairs such ids with the file's SHA-256, as
  today.
- **Paragraphs and runs.** A paragraph view gives its style, visible text
  (no deleted text, no field codes), runs with their formatting, and the
  inline objects between them: fields, footnote and endnote references,
  drawings, hyperlinks, bookmarks, comment ranges, content controls, math.
- **Tables.** A table view places cells on the grid, resolving `gridSpan`,
  `vMerge`, `gridBefore` and `gridAfter`, so a reader gets rows and columns
  as a person sees them, and a writer can address a cell by row and column.
- **Images.** Inline and floating drawings, with their relationship, part,
  size, alt text and description.
- **Fields.** Complex fields (`fldChar` begin, separate, end, also nested and
  split over runs) and simple fields (`fldSimple`), with code and result.
- **Bookmarks, comments, footnotes, endnotes.** Bookmarks with their range;
  comments with author, date, text and the range they mark; notes with their
  content and the reference that calls them.
- **Revisions.** Every `w:ins`, `w:del`, `w:moveFrom`, `w:moveTo` and the
  property-change elements, with author, date and the text they affect.
- **Styles.** Paragraph, character, table and numbering styles with id, name,
  `basedOn`, outline level and numbering. A style is found by its `w:styleId`,
  by its `w:name`, or by the name Word shows for a built-in style in another
  language: Word stores built-in names in English (`heading 1`) whatever the
  interface language, and the engine carries a table of localised names
  (`Überschrift 1`, `Titre 1`) so that a user can write the name they see.
- **Numbering.** Abstract definitions, instances, level overrides and
  numbering styles, enough to compute the number Word shows for a
  paragraph.
- **Sections.** Page size, margins, columns, headers and footers per
  section, and the usable text width that the writer needs for tables and
  images.

What the engine does not model it keeps. A view that meets an element it
does not know treats it as an opaque inline or block object: it reports it,
moves it with its paragraph, and never drops it. Edits refuse to cut through
such objects. For example, a tracked replacement refuses to delete text that
contains a field, a drawing or a note reference, and says so.

### Writing content: the content model

Every authoring path produces the same content model (`content.rs`): blocks
(paragraph, heading, list, table, figure, image, display equation, page
break, raw OOXML) and inlines (text with marks, footnote, cross-reference,
link, inline math, line break, field, image, raw OOXML). The model says what
the content is. It has a JSON form, so the command line and other languages
can produce it directly.

The writer (`write.rs`) turns the model into WordprocessingML in the target
document's own styles. `Roles` says which style plays which part: body text,
headings by level, caption, list bullet, list number, footnote text and
reference, figure paragraph, table text, default table style, hyperlink,
code. `Roles::detect` finds Word's built-in styles by their stored names; a
pack can name others. The writer:

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
- writes tables with the grid computed from HTML-style row and column spans,
  per-cell shading and alignment, repeated header rows and a real table
  style;
- tracks the insertion when asked: runs in `w:ins`, paragraph marks and rows
  marked as inserted, all under one author and date.

After every write the engine brings field results up to date (see "Fields").

### Public interface

The interface is small on purpose. Most work goes through five types and a
handful of methods.

```rust
// Open and save. Untouched parts and nodes are written as they were read.
let mut doc = Document::open("template.docx")?;          // or from_bytes
doc.save("out.docx")?;                                     // or to_bytes

// Find things.
let blocks: Vec<BlockInfo> = doc.blocks()?;                // id, kind, style, level, text
let place: BlockInfo = doc.locate(&"heading:Results".parse()?)?;
//   Locator: Block(id) | Bookmark(name) | Heading(text) | Placeholder(name) | Text(s)
let styles: &Styles = doc.styles()?;                       // resolve(name or id, kind)

// Write content in the document's styles.
let opts = WriteOptions { roles: Roles::detect(doc.styles()?), track: Some(Revision::now("Analyst")), table_styles };
doc.insert(Place::Replace(place.node), &blocks, &opts)?;   // Before, After, Replace, End

// Edit text, tracked or not, and settle revisions.
revise::replace_in_paragraph(&mut doc, para, "old", "new", Some(&rev))?;
revise::settle(doc.tree(), body, /* accept */ true, Some("Analyst"));

// Table styles and fields.
doc.ensure_table_style(&TableStyle::Defined(def))?;       // returns the style id
fields::update(&mut doc)?;
```

Below that sits `Package` for parts, relationships and content types, and
`xml::Tree` for direct node work. They are public so that other tools can
do what the high-level calls do not, but most users never need them.

Errors are one enum. The ones a user meets are `ForeignRevision` (another
author's pending change in the target, with the authors' names), `Invalid`
(a place that matches nothing or several blocks, an unknown label, a ragged
table) and `Unsupported`.

## The command line

`tabellion` is a Word tool for the terminal and for scripts. Every command
reads and writes files; none needs Word. Text output is for people, `--json`
output for programs, and every JSON shape is the same one the R and Python
bindings return.

```sh
# Look inside
tabellion inspect report.docx                 # outline: headings, tables, figures with ids
tabellion inspect report.docx blocks|styles|tables|fields|comments|revisions|images|bookmarks [--json]
tabellion text report.docx                    # plain text, as a reader sees it

# Convert, both ways
tabellion convert report.docx report.typ      # Word -> Typst, in the template's terms
tabellion convert report.docx report.md       # Word -> Markdown (pandoc-compatible)
tabellion convert report.docx report.json     # Word -> content model
tabellion convert notes.md out.docx --template report-template.docx
tabellion convert fragment.typ out.docx --template report-template.docx

# Fill a template
tabellion fill report-template.docx out.docx --pack report.pack.toml \
    --set client="North Ltd" --at '{{results}}' --typst results.typ

# Insert and edit
tabellion insert in.docx out.docx --at heading:Methods --mode before --typst part.typ [--author A]
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
tabellion table in.docx out.docx --target tab-regions --csv new.csv [--author A]

# Fields
tabellion fields in.docx out.docx             # update field results without Word

# Development
tabellion roundtrip in.docx out.docx          # proves byte identity
tabellion bench in.docx --find … --with …
```

`apply` takes the operations format, a JSON list of changes:

```json
{
  "author": "Analyst",
  "sha256": "only needed for #index block ids",
  "operations": [
    {"op": "set_cells", "target": {"label": "tab-regions"}, "table": {"rows": [["North", "12.1"]]}},
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
says `"track": false`. The rules of today's tool hold: no change inside
another author's pending revision, no deletion of citations without
`"allow_citation_loss": true`, no cut through a field, a formula or a section
break. `set_cells` changes only the cells that differ and reports the cells
it had to leave, with the reason.

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
Typst copies the block from the source document byte for byte. This is how
today's `import` and `build` work, made general.

## The R interface

The R package is the main way the owner works. It is designed for someone who
builds reports from data. Functions take the document first and return it
invisibly, so they pipe. A document is a handle to the Rust engine; the R
side never holds XML.

| Function | Effect |
| --- | --- |
| `open_docx(path)` | Open a template or document. |
| `save_docx(doc, path)` | Write it. Untouched parts stay byte-identical. |
| `at(placeholder =, bookmark =, heading =, block =, text =)` | Name a place. `at()` is the end. |
| `add_table(doc, data, style, at, mode, caption, label, digits, author)` | A data frame as a Word table. |
| `add_plot(doc, plot, width, height, dpi, caption, label, alt, at, …)` | A ggplot object or a base-graphics function as a figure. |
| `add_typst(doc, source, at, mode, author)` | Typst source, often built with `sprintf` or `glue`, as Word content. |
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
bookmarks, so `@tab-rates` in later Typst or `ref("tab-rates")` in later R
text becomes a `REF` field that Word updates.

A realistic script, taken from `spike/r/report.R`, which runs:

```r
library(tabellion)

doc <- open_docx("samples/report-template.docx")

rates <- data.frame(
  Region = c("North", "East", "South", "West"),
  n = c(1204L, 987L, 1432L, 776L),
  Mean = c(12.134, 10.421, 9.807, 11.25),
  Change = c(0.231, -0.021, 0.118, 0.040)
)

# A table style the template already has.
add_table(doc, rates, style = "Report Table", digits = 2,
          at = at(placeholder = "results"),
          caption = "Rates by region, template style", label = "tab-template")

# A table style defined in Typst, shared with Typst documents.
compact <- typst_table_style("spike/typst/styles.typ", "compact")
add_table(doc, rates, style = compact, digits = 2,
          at = at(heading = "Methods"), mode = "before",
          caption = "Rates by region, style defined in Typst", label = "tab-typst")

# A table style defined in R.
house <- table_style("House Green",
  header = list(fill = "#385723", color = "#FFFFFF", bold = TRUE),
  band_fill = "#E2EFDA", top = rule(1, "385723"), bottom = rule(1, "385723"),
  inside_h = rule(0.25, "A9D08E"))
add_table(doc, rates, style = house, digits = 1,
          at = at(heading = "Methods"), mode = "before",
          caption = "Rates by region, style defined in R", label = "tab-r")

add_plot(doc, function() barplot(rates$Mean, names.arg = rates$Region),
         width = 14, height = 7, caption = "Mean rate by region", label = "fig-rates",
         at = at(heading = "Methods"), mode = "before")

best <- rates$Region[which.max(rates$Mean)]
add_typst(doc, sprintf(
  "The highest mean rate was in *%s* (%s). @tab-template, @tab-typst and @tab-r show the same data in three styles; @fig-rates plots it.",
  best, format(max(rates$Mean), digits = 3)),
  at = at(heading = "Methods"), mode = "before")

# A tracked change by a named author.
add_typst(doc, "_Added for review._", at = at(bookmark = "summary"), author = "Analyst")

save_docx(doc, "report.docx")

# Reading back
back <- open_docx("report.docx")
docx_tables(back)[[1]]    # Region chr, n int, Mean num, Change num
```

The text of the generated paragraph reads, before Word opens the file: "The
highest mean rate was in North (12.1). Table 1, Table 2 and Table 3 show the
same data in three styles; Figure 1 plots it." The numbers come from the
engine's field update and are what Word shows after its own update.

Number formatting belongs to the pack (`[numbers]`: decimal mark, thousands
mark, text for missing values, interval and p-value forms), so `add_table`
and the Typst helpers format numbers the same way. Packages that format
tables already (gt, flextable) can hand their result over as a content-model
table later; it is not needed for the first release.

## Typst as an authoring path

A Typst fragment or file goes in; Word content in the template's styles
comes out. The Typst can be written by hand or built in R.

### How: three options

1. **Evaluate in-process and walk the content tree.** The `typst` crates
   evaluate the source to a module whose content is the tree of elements
   before layout: headings, paragraphs, strong and emphasis, lists, tables
   with their cells, figures with captions, footnotes, references, equations
   with their math nodes, and `styled` elements that carry set rules. The
   walker maps this tree to the content model and reads text properties
   (weight, style, fill, size) from the style chain.
2. **Typst's HTML export.** It realises the document for the HTML target and
   applies show rules. But it maps elements to HTML, not to the semantics
   Word needs: a reference becomes a link with text, a footnote a list at the
   end, a caption a `figcaption` with Typst's own numbering, and equations are
   not exported natively. Word content would have to be recovered from HTML,
   which loses exactly what makes the result a Word document: fields,
   numbering, styles.
3. **The current approach.** `typst eval` with `query`, JSON out, mapping in
   R. It is option 1 behind a process boundary and a lossy serialisation.
   Each call costs a process start and a full evaluation, and the JSON loses
   types (lengths, colours, labels become strings).

The rebuild takes option 1. It keeps the semantic tree, which is what Word
needs, and both sides are Rust, so the content never leaves memory. Its one
real gap is show rules: they run during realisation, which the walk does not
do. The walker reports every show rule it meets and keeps the content as
written. Set rules work, because they are only styles. If show rules turn out
to matter, the next step is to run Typst's realisation for the HTML target
and walk its output for the elements that a show rule replaced, keeping
option 1 for the rest; that needs no change to the content model.

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
| `table` with `columns`, `table.header`, `table.cell(colspan:, rowspan:, fill:, align:)`, table `fill` colour | Word table on a grid with `gridSpan` and `vMerge`, cell shading, alignment, repeated header rows, a table style |
| `figure` with image, table or equation, `caption`, label | caption with SEQ field and bookmark, above tables and below figures (pack decides) |
| `image` (PNG, JPEG, GIF), `width` | inline picture, sized |
| `footnote` | Word footnote |
| `@label` | REF field (to a heading, figure, table or numbered equation) |
| `link("https://…")` | hyperlink with a relationship, Hyperlink style |
| `$ … $` inline and block, labelled block | Office Math; a labelled block equation gets a number "(n)" at the right margin |
| math: fractions, scripts, roots, `lr` delimiters, accents, `op`, primes, big operators with limits (sum, product, integral, union), `mat`, `vec`, `cases` | `m:f`, `m:sSub`/`m:sSup`/`m:sSubSup`, `m:rad`, `m:d`, `m:acc`, `m:nary`, `m:m` |
| `pagebreak` | page break |
| `cite` | a `[@key]` marker that the citation tool turns into a citation content control; a warning says so |

### What does not, and what happens to it

Nothing disappears without a warning. The walker returns warnings with the
source text of the element, and the R function raises them as R warnings.

- **Rendered as a picture:** drawings (`rect`, `circle`, `line`, `polygon`,
  `curve`), `grid`, `stack`, `columns`, `rotate`, `scale`, `move`, `box` or
  `block` with a fill, border or fixed size, and any other element without a
  native form. Typst lays the element out on its own page and the engine
  embeds the result as a PNG at about 290 pixels per inch, with the source
  as alt text. A later version embeds SVG with a PNG fallback, which Word
  2016 and later show sharply at any zoom.
- **Left to Word's styles, with a warning:** table strokes and `table.hline`
  or `vline` (the table style draws lines), vertical spacing (`v`),
  paragraph spacing and indents set in Typst, page setup.
- **Left out, with a warning:** `outline` (insert a Word table of contents,
  which Word fills), `bibliography` (the citation tool writes the reference
  list), `place`, `hide`, `repeat`, `context` expressions that depend on
  layout, and show rules (their content is kept as written).
- **Approximated, with a warning:** a reference's custom supplement (Word
  shows the caption label), a link to a location in the document (kept as
  text), explicit cell positions (`x`, `y`; placed in reading order), a fill
  function on a table (use `table.cell(fill:)` or a table style), left-side
  math scripts (placed on the right), a line break inside an equation.

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
add_table(doc, df, style = table_style("House Green",
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

#styled-table(compact, columns: 3, table.header[Item][Count][Share], …)
```

`styled-table` draws the table in that look when the Typst is compiled to
PDF, and marks it for tabellion, so the Typst preview and the Word result
agree. Per-cell shading from `table.cell(fill:)` is direct formatting on top
of the style, as in Word.

## Packs

A pack says what a template means. It is a small TOML file that a user
supplies next to the template; the repository ships one neutral sample,
`samples/report.pack.toml`, for `samples/report-template.docx`. Without a pack,
tabellion uses Word's built-in style names, which is enough for a template
that uses them.

A pack holds:

- `[pack]`: name, version, language.
- `[[template]]`: the templates it describes, each with an id, a file name or
  a URL, and the SHA-256 that pins the exact file; `tabellion pack fetch`
  downloads and checks them. `[upstream]` optionally names a page to watch
  for new versions.
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
  after a build, and style aliases that Word needs during a field update.
- `[blocks]`: a Typst file of reusable text blocks with parameters.
- `[citations]`: how the citation tool marks citations and reference lists,
  so that edits protect them.
- `[tracking]`: the default author of tracked changes.

The format can express every piece of the current tool's template knowledge:
the style regular expressions of `R/scan.R`, the caption and list rules of
`R/build.R`, the protected kinds and checks of `R/ops.R`, the cover fields of
`R/fields.R`, the number formats of `R/daten.R`, the template list of
`templates.csv`, the text blocks of `typst/methoden.typ`, the citation
markers of `R/zitate.R`, and the Word style-name workaround of
`word/finalize.ps1`. A pack with all of it exists outside this repository.

## Fields

The engine recomputes, without Word, the results Word would store after
"update fields":

- `SEQ` numbers per identifier, with `\r` (reset to a value), `\c` (repeat)
  and `\h` (hidden), in complex and simple fields;
- `REF` to a bookmark: the bookmark's text after the SEQ update, including
  non-breaking hyphens and tabs;
- `NOTEREF`: the footnote's number;
- `REF \r`, `\n`, `\w`: the paragraph number of the bookmarked paragraph, from
  the numbering engine (in the rebuild; the spike leaves these as they are
  rather than write them wrong);
- `STYLEREF`: the text or number of the nearest paragraph in a style;
- the entries of a table of contents and a table of figures: text, levels,
  hyperlinks and bookmarks, rebuilt from headings and captions as
  `R/resolve.R` does today.

A field update that finds nothing to change writes nothing: on the largest
template the spike's update finds all 64 SEQ and REF results equal to Word's
and leaves the part untouched.

Only Word can compute what needs layout: page numbers (`PAGE`, `NUMPAGES`,
`PAGEREF`, the page column of every table of contents), `SECTIONPAGES`, and
anything conditional on them. Those keep their last result. Word refreshes
them when the user updates fields or prints; `tools/word/check.ps1` shows how
to do it by automation. Two more fields are Word's alone: `DATE`/`TIME`
results that should show the time of opening, and `INCLUDETEXT` or
`INCLUDEPICTURE` that read other files.

One open question carries over. Word restarts a `SEQ … \s 1` sequence after
each Heading 1, but on the largest template it does not, apparently because
those headings are not numbered by a list. The engine does not apply `\s`
yet; the numbering engine will decide it from the heading's numbering, and a
Word comparison test pins the rule down.

## How correctness is checked

### Without Word, in CI

- **Byte identity.** For every test document, `roundtrip` must keep every zip
  entry identical, and an edit must leave every byte outside its target
  unchanged. The test compares the longest common prefix and suffix of the
  changed part with the edit's own extent.
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
- **Property tests.** Random content-model documents go through write, read
  and write again; random edit sequences on real templates must keep the
  untouched bytes.
- **Field results.** For documents where Word's results are known (stored
  from a Word run), the engine's field update must reproduce them exactly.

### With Word, where it is available

`tools/word/check.sh` runs `tools/word/check.ps1` from WSL against the Word
on the Windows host. For each file it:

1. starts a new, invisible `Word.Application` by COM, finds its process id
   by comparing the running `WINWORD` processes before and after, and stops
   at once without touching Word if it cannot tell its own instance apart;
2. opens the file with alerts off; a file that needs repair either raises an
   error or makes Word wait on a dialog, which the shell script detects by a
   time limit, after which it stops that one process and reports the file as
   failed;
3. records pages, paragraphs, tables and their styles, the document's table
   styles with Word's visibility flag, revisions, footnotes, and every
   field's code and result;
4. updates all fields in all stories and records the results again, so the
   engine's results can be compared with Word's;
5. saves the updated file, then accepts all revisions and saves, reopens,
   rejects all and saves, and optionally saves a PDF;
6. quits its own Word and makes sure that process is gone.

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
  vendored crates when the tarball has them and the workspace otherwise. The
  build needs `cargo` 1.85 or later. In this repository the package is a
  workspace member and shares the workspace's target directory.
- **Size.** The Typst compiler and its embedded fonts make the library large
  (the spike's release binary with debug symbols is 277 MB). Release builds
  strip symbols and use `lto = "thin"`; fonts are needed only for elements
  that become pictures and can move behind a Cargo feature that R-universe
  enables, so a build without it still maps Typst content.
- **Dependencies.** The R side uses only base R and jsonlite; extendr's list
  conversions can replace jsonlite later.

## Migration

The old code stays until the new one covers its tests. The cheap parts of
migration:

- **Operations.** `tabellion apply` accepts the old JSON keys and maps them,
  with a deprecation warning:

  | Old | New |
  | --- | --- |
  | `aenderungen` | `operations` |
  | `ziel` | `target` |
  | `ziel.tag` | `target.label` (old tags in alt text and in hidden bookmarks are found as labels; the pack names their form) |
  | `ziel.block`, `ziel.bis` | `target.block`, `target.to` |
  | `werte` | `set_cells` |
  | `ersetzen` | `replace` |
  | `einfuegen` + `platzhalter` | `insert` + `at.placeholder` |
  | `titel` | `set_caption` |
  | `felder` | `update_fields` |
  | `tabelle` (`kopf`, `zeilen`, `spalten`) | `table` (`header`, `rows`, `widths`) |
  | cell `spalte`, `zeile`, `fett` | `colspan`, `rowspan`, `bold` |
  | `abbildung` (`datei`, `breite`) | `image` (`file`, `width`) |
  | `text.baustein`, `werte` | `content.block`, `values` |
  | `zitate_loeschen` | `allow_citation_loss` |
  | report `offen`, `platzhalter_offen` | `skipped`, `open_placeholders` |

- **R functions.** A small compatibility file maps the old table, cell, row
  and JSON helpers to `add_table()` and the content model, and the old
  number, interval and p-value formatters to the pack's number formats. They
  warn and will go in a later release.
- **Typst projects.** Old `.typ` projects from `import` use German helper
  names for answers, template tables, new tables, figures and kept blocks. A
  converter rewrites them to the new vocabulary (`#fill`, `#table-rows`,
  `#figure`, `#keep`) by syntax, since the calls are regular; a pack can keep
  the old names working in the meantime.
- **Template knowledge** moves from code into the private pack.

What is not cheap and not attempted: reading an old project's block ids
(`b0123`) against a new template version. `migrate` stays a pack-specific
step that runs on the old code until the new `convert` round trip replaces
it.

## Performance

Targets, for a generated 279-page document with 300 tables, 1,200 fields and
300 footnotes (5.3 MB `document.xml`), on one core of a current desktop:

| Step | Target | Spike |
| --- | --- | --- |
| Open and parse the main part | < 150 ms | 60–80 ms |
| List blocks and styles | < 50 ms | 14 ms |
| Write unchanged | < 10 ms | 1 ms |
| One tracked replacement | < 20 ms | 11–15 ms |
| Field update over the whole body | < 100 ms | 36–42 ms |
| Write after the change | < 50 ms | 23–29 ms |
| Insert 300 chapters (headings, tables, captions, references, footnotes, lists) | < 1 s | 120–280 ms |

On the largest template the current `fetch` downloads (114 pages, 1.8 MB
`document.xml`), the whole cycle of open, tracked replacement, field update
and write takes about 55 ms.

For comparison, the current R code on the same machine takes 6.4 s to read
and parse that template and 10.8 s for one tracked caption edit, and on the
generated document about a minute to read and parse and close to a minute
for one tracked cell edit. officer (read_docx and print) is measured as a
reference; it does neither tracked edits nor field updates, so only its read
and write times compare. The comparison script calls the old R code and
stays with the private pack, since the old code goes away.

`tabellion bench` measures each step as the median of several runs in one
process, reading the file from disk each time. CI runs it on the generated
document and fails when a step exceeds twice its target, so slow changes are
seen when they are made. The generator (`cargo run --example generate`)
makes the benchmark document from the sample template, so anyone can repeat
the numbers.

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

## Spike

The spike shows that the risky parts work. Everything runs from the
repository:

```sh
spike/run.sh [real.docx]                       # builds, generates, times, edits, converts
tools/word/check.sh spike/out/word spike/out/*.docx
python3 spike/word-summary.py spike/out/word   # what Word made of each file
```

- `crates/nebu-docx`: the lossless tree, the package, blocks, styles,
  locators, the content-model writer, tracked replacement, accept and reject,
  table styles, field results. `examples/generate.rs` builds the large test
  document.
- `crates/tabellion-typst`: in-process evaluation and the content walk, math
  to OMML, the picture fallback, table styles from Typst.
- `crates/tabellion`: the command line subset `inspect`, `insert`, `replace`,
  `accept`, `reject`, `roundtrip`, `bench`.
- `r/tabellion`: `open_docx`, `save_docx`, `at`, `add_table`, `add_plot`,
  `add_typst`, `table_style`, `typst_table_style`, `rule`, `docx_blocks`,
  `docx_tables`.
- `samples/`: the neutral template, its sources and its pack.
- `spike/typst/fragment.typ`: a heading, styled text, lists, a table with a
  merged and shaded cell, an image figure, a footnote, cross-references, a
  numbered equation, inline math, a table in a Typst-defined style and a
  drawn box that becomes a picture.
- `tools/word/`: the Word check.

Word 16 (build 20326) opened every spike output without a repair prompt:
the generated document and a tracked cell edit in it, a tracked caption edit
and an unchanged round trip of the largest real template, the Typst fragment,
and the R report. In each file, Word's own field update changed none of the
SEQ and REF results the engine had written (1,200 in the generated document,
where Word's update took 77 to 80 seconds). Accepting and rejecting the
tracked changes in Word gave the expected text. The table styles from the
template, from Typst and from R appear as table styles with the same
visibility as "Table Grid", which the gallery shows.

