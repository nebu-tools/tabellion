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

Illustrated guide: [docs/how-to-use.html](docs/how-to-use.html).

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
| `#abbildung(image("km.png"), beschriftung: [...], tag: "abb_os")` | Add a captioned figure |
| `#baustein("ereigniszeit", werte: (n: 12, studie: "A"), tag: "met_km")` | Add a text block from `typst/methoden.typ` inside `#antwort` |
| `#mit(("<Endpunkt xxx>": name), [...])` | Replace placeholders in a repeated section |
| `#kreuz(0, true)` | Tick a checkbox |

R helpers: `gba_json()`, `gba_zeilen()`, `gba_tabelle()`, `gba_zelle()`, `gba_zahl()`, `gba_ki()`, `gba_p()`.

`import` writes blocks that Typst cannot represent, such as content controls, as `#fest("b0123")`, and they stay unchanged. `build` stops if the docx changed since `import`, or if text inside a `#feld` was edited. Import again rather than reusing an old `.typ`.

## Edit an existing docx

For a docx that people work on in Word, skip Typst. These commands change only their target and write tracked changes under the author `Automatisierung`.

```sh
rv run gba.R outline modul4.docx                    # headings, text, tables, figures, placeholders as JSON
rv run gba.R apply modul4.docx ops.json out.docx    # prints a report as JSON
```

```json
{
  "sha256": "<from outline, only needed for block targets>",
  "aenderungen": [
    {"op": "werte", "ziel": {"tag": "tab_os"}, "tabelle": {"kopf": [["Arm", "N"]], "zeilen": [["A", "12"]]}},
    {"op": "ersetzen", "ziel": {"block": "b0145"}, "tabelle": {"kopf": [["Arm"]], "zeilen": [["A"]]}},
    {"op": "einfuegen", "platzhalter": "tab:os", "tabelle": {"titel": "OS", "tag": "tab_os", "kopf": [["Arm"]], "zeilen": [["A"]]}},
    {"op": "einfuegen", "platzhalter": "abb:km", "abbildung": {"datei": "km.png", "titel": "KM OS", "tag": "abb_km", "breite": "14cm"}},
    {"op": "einfuegen", "platzhalter": "met:km", "text": {"baustein": "ereigniszeit", "werte": {"n": 12, "studie": "A"}, "tag": "met_km"}},
    {"op": "ersetzen", "ziel": {"tag": "abb_km"}, "abbildung": {"datei": "km2.png"}},
    {"op": "ersetzen", "ziel": {"block": "b0150", "bis": "b0152"}, "text": {"baustein": "subgruppen", "werte": {"alpha": 0.05}}},
    {"op": "titel", "ziel": {"tag": "tab_os"}, "titel": "Neuer Titel"},
    {"op": "felder"}
  ]
}
```

| Operation | Effect |
| --- | --- |
| `werte` | Changes the cells that differ. Needs the same table shape |
| `ersetzen` | Marks the old table, figure, or text as deleted and adds the new one after it |
| `einfuegen` | Replaces a paragraph that holds only `{{name}}` with a table, a figure, or a text block |
| `titel` | Changes the text of a table's or figure's caption. The number stays |
| `felder` | Updates numbering, cross references, and contents text without any other change |

A `titel` in a `tabelle` or `abbildung` of `werte` or `ersetzen` changes the caption too. These caption edits are tracked.

Fields update after every run that changes the document, and those updates are not tracked. A run that changes nothing returns the file unchanged, so use `felder` after rejecting changes in Word.

Text blocks live in `typst/methoden.typ`, so they support bold, lists, formulas, and footnotes. A missing value stops the run. Relative figure paths start at the ops file.

Any failed check stops the run and writes nothing: an unknown target, a duplicate tag, a different table shape, or pending changes by someone else in a target to replace. `ersetzen` also stops on EndNote citations unless the change sets `"zitate_loeschen": true`, and on text that holds a formula or section break or sits inside a field. Only text and answer paragraphs can be replaced, not questions, explanations, or headings. `werte` skips cells it cannot change safely and lists them under `offen`: cells with another author's pending change, and cells with a field, citation, footnote, or several paragraphs. EndNote fields stay as they are, so add citations in Word.

Tables and figures store a tag in their alt text as `gba:<tag>`, and each text paragraph holds a hidden bookmark `_gba_<tag>_<n>`. Tags still find their target after Word saves the file. Text tags use letters, digits, and `_`, up to 27 characters.

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
