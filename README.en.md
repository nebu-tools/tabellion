# G-BA dossiers in Typst and R

Write German G-BA benefit dossier modules in Typst, fill them from R, and build a `.docx` from the official G-BA Word template. The build runs on Linux and does not need Word.

German documentation: [README.md](README.md)

## Use it

You need `rv`, `typst`, `curl`, and `zip`.

```sh
rv sync
rv run gba.R fetch
rv run gba.R import modul4 dossier/m4
rv run gba.R build dossier/m4/2025-11-18_Anl2_6_Modul4.typ
```

`fetch` downloads the templates to `templates/` and verifies their checksums. This repository does not contain the templates. [templates.csv](templates.csv) lists their URLs.

Before you create the submission PDF, open the docx in Word and update all fields. The build writes table numbers, cross references, and table of contents text, but it cannot compute page numbers.

## How it works

`import` splits the template into blocks and writes them as Typst. `build` compares each block with the template. It keeps unchanged blocks byte for byte and rebuilds changed blocks with the template's style. An unchanged `.typ` file gives a docx whose parts are byte-identical to the template.

## License

[AGPL-3.0](LICENSE). The G-BA templates are not part of this repository and this license does not cover them.
