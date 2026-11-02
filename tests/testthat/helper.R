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
