# rv run gba.R fetch                       Vorlagen laden und prüfen
# rv run gba.R import <modul> <ordner>     Typst-Projekt aus einer Vorlage erzeugen
# rv run gba.R build <dossier.typ>         docx bauen
# rv run gba.R migrate <alt.typ> <modul> <ordner>  auf eine neue Vorlage umziehen
# rv run gba.R outline <in.docx>           Überschriften, Text, Tabellen, Abbildungen, Platzhalter als JSON
# rv run gba.R apply <in.docx> <ops.json> <out.docx>  Änderungen nachverfolgt anwenden
# rv run gba.R check-templates             Vorlagen mit der G-BA-Seite abgleichen
a <- commandArgs(TRUE)
root <- normalizePath(dirname(sub("^--file=", "", grep("^--file=", commandArgs(), value = TRUE)[1])))
options(gba.root = root)
for (f in list.files(file.path(root, "R"), "\\.R$", full.names = TRUE)) source(f)
switch(a[1],
  fetch = gba_fetch(),
  import = cat(gba_import(a[2], a[3]), "\n"),
  build = gba_build(a[2]),
  migrate = gba_migrate(a[2], a[3], a[4]),
  outline = cat(jsonlite::toJSON(gba_outline(a[2]), auto_unbox = TRUE, pretty = TRUE), "\n"),
  apply = cat(jsonlite::toJSON(gba_apply(a[2], a[3], a[4]), auto_unbox = TRUE, pretty = TRUE), "\n"),
  "check-templates" = gba_check_templates(),
  stop("Befehle: fetch, import, build, migrate, outline, apply, check-templates")
)
