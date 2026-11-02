<img src="assets/icon.svg" width="64" height="64" alt="">

# G-BA dossiers in Typst and R

Write German G-BA benefit dossier modules in Typst, fill them from R, and build a `.docx` from the official G-BA Word template. The dossier team opens the file in Word and works with it like a filled-in template.

The build runs on Linux and does not need Word.

## Set up

You need `rv`, `typst`, `curl`, and `zip`.

```sh
rv sync
rv run gba.R fetch
```

`fetch` downloads the templates from the G-BA to `templates/` and verifies their checksums. This repository does not contain the templates. [templates.csv](templates.csv) lists their URLs.

## Create a module

1. Generate a Typst project:

   ```sh
   rv run gba.R import modul4 dossier/m4
   ```

2. Edit the generated `.typ` file in `dossier/m4/`.
3. Build the docx:

   ```sh
   rv run gba.R build dossier/m4/2025-11-18_Anl2_6_Modul4.typ
   ```

The build writes `<name>_gefuellt.docx` next to the source. It reports the number of open placeholders and warns if a question, an explanation, or a heading from the template is missing.

To preview while you write, run `typst watch` on the `.typ` file. The preview shows the content, not the Word layout.

[examples/modul4/demo.R](examples/modul4/demo.R) fills Modul 4 with public data from `survival::colon`. It shows R tables, Kaplan-Meier plots, and a section repeated for each endpoint.

Available modules: `modul1`, `modul1-checkliste`, `modul2`, `modul3`, `modul4`, `k1`, `k1-checkliste`, `k2`, `k3`, `k4`.

## Update page numbers in Word

The build writes table and figure numbers, cross references, and the text of all tables of contents into the docx. It cannot compute page numbers, because those depend on Word's layout. New entries in a table of contents carry an estimated page.

Before you create the submission PDF, press `Ctrl+A` and then `F9` in Word. For each table of contents, choose **Update entire table**.

If your Word UI is not German, this update drops the chapter 1 entry from the table of contents. The field code names the style "Überschrift 1" literally. A template filled in by hand behaves the same way.

## Edit the .typ file

Each block carries the ID of its counterpart in the template, for example `"b0141"`. Do not delete questions and explanations, because the G-BA rules require them in the document.

Fill the cover page, headers, and footers with `felder`:

```typst
#show: dossier.with(vorlage: "...", felder: (
	wirkstoff: "...", handelsname: "...", unternehmer: "...",
	kodierung: "A", anwendungsgebiet: "...", stand: "01.12.2026",
))
```

An answer replaces the placeholder `<< Angaben des pharmazeutischen Unternehmers >>`. It can contain text, lists, footnotes, tables, and figures:

```typst
#antwort("b0141")[
	Text with a footnote#footnote[Source].

	- List item
]
```

Data from R replaces all rows below the header of a template table. Write the data in R with `gba_json()`:

```typst
#tabelle("b0430", daten: json("ergebnisse.json").at(0).zeilen)
```

If the number of columns differs from the template, pass relative widths with `spalten:`.

New tables and figures get a caption in the template's style:

```typst
#neue-tabelle(beschriftung: [Title], kopf: ([A], [B]), zeilen: rows)
#abbildung(image("km.png", width: 14cm), beschriftung: [Title])
```

To repeat a section, use a function and `mit`. `mit` replaces placeholders such as `<Endpunkt xxx>` in all blocks of the section:

```typst
#let abschnitt(e) = mit(("<Endpunkt xxx>": e.name), [
	#ueberschrift(5, "b0381")[\<Endpunkt xxx\> – RCT]
])
#for e in endpunkte { abschnitt(e) }
```

To tick a checkbox, use `#kreuz(0, true)`.

`#feld(n)[...]` stands for an object in the template, such as a cross reference. The text in the brackets is the stored value. Changing that text does not change the field.

For R results, use `gba_zeilen()`, `gba_json()`, `gba_zahl()`, `gba_ki()`, and `gba_p()`.

## Why the Word template stays the source

The G-BA rules require the templates, and the templates contain things a converter does not rebuild: form fields, frames, sections, and field structures. So the build edits the template instead of generating a new file.

`gba.R import` splits the template into blocks and writes them as Typst. `gba.R build` compares each block with the template. It keeps unchanged blocks byte for byte and rebuilds changed blocks with the template's style and bookmarks. An unchanged `.typ` file gives a docx whose parts are byte-identical to the template.

## Limits

- Numbered lists from Typst continue through the document and do not restart.
- Figures must be PNG or JPEG.
- The build writes formulas as text with superscript and subscript.
- Typst elements with no Word equivalent, such as `#rect` or `context`, stop the build with a message.
- A `.typ` file belongs to one template version. A new template changes the block IDs.

## Test

```sh
rv run tests/testthat.R
```

CI runs the same tests. A weekly run reports when the G-BA page no longer links a template from `templates.csv`.

`tools/word_check.R` compares the build's field results with Word's. It needs Word on Windows with WSL, so it does not run in CI.

## License

[AGPL-3.0](LICENSE). The G-BA templates are not part of this repository and this license does not cover them.
