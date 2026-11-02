module <- manifest()$modul
modul4 <- gba_template("modul4")

teile <- function(docx) {
  n <- utils::unzip(docx, list = TRUE)$Name
  d <- tempfile(); utils::unzip(docx, exdir = d)
  stats::setNames(unname(tools::md5sum(file.path(d, n))), n)
}

dokument <- function(docx) {
  d <- tempfile(); utils::unzip(docx, "word/document.xml", exdir = d)
  read_text(file.path(d, "word/document.xml"))
}

# Imports the template, applies edits to the source, builds without Word.
bauen <- function(modul, aendern = identity, bild = NULL, svg = NULL) {
  dir <- tempfile("gba-test-")
  typ <- gba_import(modul, dir)
  src <- read_text(typ)
  write_text(aendern(src), typ)
  if (!is.null(bild)) file.copy(bild, file.path(dir, "bild.png"))
  if (!is.null(svg)) file.copy(svg, file.path(dir, "bild.svg"))
  out <- file.path(dir, "out.docx")
  capture.output(rep <- gba_build(typ, out = out))
  list(out = out, rep = rep, xml = dokument(out))
}

subst <- function(src, alt, neu) {
  stopifnot(grepl(alt, src, fixed = TRUE))
  sub(alt, neu, src, fixed = TRUE)
}

# A filled module, as the dossier team would upload it: one tagged table and a
# placeholder paragraph.
gefuellt <- local({
  pfad <- NULL
  function() {
    if (is.null(pfad)) {
      pfad <<- bauen("modul4", function(s) {
        id <- regmatches(s, regexpr("#antwort\\(\"b[0-9]+\"\\)", s))
        sub(id, paste0(id, "[Betrachtet werden.\n\n{{tab:neu}}\n\n{{met}}\n\n{{abb}}\n\n",
                       "#neue-tabelle(tabelle: (tag: \"tab_os\", titel: \"OS\", kopf: ((\"A\", \"B\"),), ",
                       "zeilen: ((\"eins\", \"zwei\"), (\"drei\", \"vier\"))))]"), s, fixed = TRUE)
      })$out
    }
    pfad
  }
})

# Rewrites word/document.xml of a docx.
aendere_docx <- function(docx, f) {
  d <- tempfile(); dir.create(d)
  n <- utils::unzip(docx, list = TRUE)$Name
  utils::unzip(docx, exdir = d)
  p <- file.path(d, "word/document.xml")
  write_text(f(read_text(p)), p)
  ziel <- tempfile(fileext = ".docx")
  l <- tempfile(); writeLines(n, l)
  old <- setwd(d); on.exit(setwd(old))
  stopifnot(system2("zip", c("-X", "-q", shQuote(ziel), "-@"), stdin = l) == 0L)
  ziel
}

anwenden <- function(docx, ...) {
  out <- tempfile(fileext = ".docx")
  rep <- gba_apply(docx, list(aenderungen = list(...)), out)
  list(out = out, rep = rep, xml = dokument(out))
}

xml_gueltig <- function(x) xml2::read_xml(x)
