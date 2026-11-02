<img src="assets/icon.svg" width="64" height="64" alt="">

# G-BA dossiers in Typst and R

Builds G-BA dossier modules as `.docx` from Typst and R, on top of the official Word templates. Runs on Linux without Word.

## Usage

Requires `rv`, `typst`, `curl`, and `zip`.

```sh
rv sync
rv run gba.R fetch                     # download templates to templates/
rv run gba.R import modul4 dossier/m4  # template -> Typst project
rv run gba.R build dossier/m4/2025-11-18_Anl2_6_Modul4.typ  # -> <name>_gefuellt.docx
rv run gba.R migrate old/m4.typ modul4 dossier/m4-new        # move to a new template version
```

`import` also takes the path of an existing docx instead of a module name.

Modules: `modul1`, `modul1-checkliste`, `modul2`, `modul3`, `modul4`, `k1`, `k1-checkliste`, `k2`, `k3`, `k4`.

Example with R tables and Kaplan-Meier plots: [examples/modul4/demo.R](examples/modul4/demo.R).

Page numbers depend on Word's layout. Before you create the PDF, update all fields in Word (`Ctrl+A`, `F9`). With a non-German Word UI, the update drops the chapter 1 TOC entry.

## Typst API

| Call | Effect |
| --- | --- |
| `#show: dossier.with(felder: (wirkstoff: "...", stand: "..."))` | Fill the cover page, headers, and footers |
| `#antwort("b0141")[...]` | Replace an answer placeholder |
| `#tabelle("b0430", daten: rows, spalten: (2, 1))` | Replace the body rows of a template table |
| `#neue-tabelle(beschriftung: [...], kopf: (...), zeilen: rows)` | Add a captioned table |
| `#neue-tabelle(tabelle: json("os.json"))` | Add a table from `gba_tabelle()`, with merged cells |
| `#abbildung(image("km.png"), beschriftung: [...])` | Add a captioned figure |
| `#mit(("<Endpunkt xxx>": name), [...])` | Replace placeholders in a repeated section |
| `#kreuz(0, true)` | Tick a checkbox |

R helpers: `gba_json()`, `gba_zeilen()`, `gba_tabelle()`, `gba_zelle()`, `gba_zahl()`, `gba_ki()`, `gba_p()`.

`import` writes blocks that Typst cannot represent, such as content controls, as `#fest("b0123")`, and they stay unchanged. `build` stops if the docx changed since `import`, or if text inside a `#feld` was edited. Import again rather than reusing an old `.typ`.

## Edit an existing docx

For a docx that people work on in Word, skip Typst. These commands change only their target and write tracked changes under the author `Automatisierung`.

```sh
rv run gba.R outline modul4.docx                    # headings, tables, placeholders as JSON
rv run gba.R apply modul4.docx ops.json out.docx    # prints a report as JSON
```

```json
{
  "sha256": "<from outline, only needed for block targets>",
  "aenderungen": [
    {"op": "werte", "ziel": {"tag": "tab_os"}, "tabelle": {"kopf": [["Arm", "N"]], "zeilen": [["A", "12"]]}},
    {"op": "ersetzen", "ziel": {"block": "b0145"}, "tabelle": {"kopf": [["Arm"]], "zeilen": [["A"]]}},
    {"op": "einfuegen", "platzhalter": "tab:os", "tabelle": {"titel": "OS", "tag": "tab_os", "kopf": [["Arm"]], "zeilen": [["A"]]}},
    {"op": "felder"}
  ]
}
```

| Operation | Effect |
| --- | --- |
| `werte` | Changes the cells that differ. Needs the same table shape |
| `ersetzen` | Marks the old table as deleted and adds the new one after it. The caption stays |
| `einfuegen` | Replaces a paragraph that holds only `{{name}}` with a captioned table |
| `felder` | Updates numbering, cross references, and contents text |

Any failed check stops the run and writes nothing: an unknown target, a duplicate tag, a different table shape, or pending changes by someone else in a table to replace. `werte` skips cells it cannot change safely and lists them under `offen`: cells with another author's pending change, and cells with a field, citation, footnote, or several paragraphs. EndNote fields stay as they are, so add citations in Word.

A tag is stored in the table's alt text as `gba:<tag>`. After Word saves the file, the tag still finds the table. Run `felder` or update fields in Word after rejecting changes, because Word keeps old caption numbers until then.

## How it works

The template stays the source. `import` writes each template block as Typst with its ID. `build` keeps unchanged blocks byte for byte and rebuilds changed blocks in the template's style. An unchanged `.typ` file reproduces the template.

The G-BA rules require questions and explanations in the document, so do not delete those blocks.

## Limits

- Formulas support sub- and superscripts, fractions, roots, and accents. Matrices and `cases` stop the build.
- Word cannot embed all vector output safely, so the build renders SVG and PDF figures to PNG at 300 ppi.
- `context` and drawing elements such as `#rect` stop the build.
- `gba.R migrate` reports blocks whose template text changed. Review those by hand.

## Test

```sh
rv run tests/testthat.R
```

`tools/word_check.R` compares field results with Word and needs Word on Windows with WSL.

## License

[AGPL-3.0](LICENSE). The repository links the G-BA templates and does not include them. This license does not cover them.
