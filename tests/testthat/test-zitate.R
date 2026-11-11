# Citations of the Word add-in (addin/): content controls plus a custom XML part.

text_ops <- list(baustein = "ereigniszeit", werte = list(n = "12", studie = "A"))

ZITAT <- paste0(
  "<w:sdt><w:sdtPr><w:alias w:val=\"Zitat: mueller2020; csr\"/><w:tag w:val=\"zitat:k1\"/><w:id w:val=\"11\"/></w:sdtPr>",
  "<w:sdtContent><w:r><w:t>[1,2]</w:t></w:r></w:sdtContent></w:sdt>")
LISTE <- paste0(
  "<w:sdt><w:sdtPr><w:alias w:val=\"Literaturverzeichnis\"/><w:tag w:val=\"literaturverzeichnis\"/><w:id w:val=\"12\"/></w:sdtPr>",
  "<w:sdtContent><w:p><w:r><w:t xml:space=\"preserve\">1.\tMüller A. Overall survival.</w:t></w:r></w:p>",
  "<w:p><w:r><w:t xml:space=\"preserve\">2.\tMuster Pharma GmbH. CSR.</w:t></w:r></w:p></w:sdtContent></w:sdt>")
STORE <- paste0(
  "<?xml version=\"1.0\" encoding=\"UTF-8\"?><zitate xmlns=\"urn:newga:zitate:1\">",
  "{\"quellen\":{\"mueller2020\":{\"id\":\"mueller2020\",\"title\":\"Overall survival &amp; more\"},\"csr\":{\"id\":\"csr\",\"title\":\"CSR\"}},",
  "\"zitate\":{\"k1\":[{\"id\":\"mueller2020\"},{\"id\":\"csr\"}]}}</zitate>")

# The filled module with one citation behind "Betrachtet werden." and a list
# after its paragraph, as the add-in saves them.
mit_zitaten <- local({
  pfad <- NULL
  function() {
    if (is.null(pfad)) {
      doc <- aendere_docx(gefuellt(), function(x) {
        x <- sub("Betrachtet werden.</w:t></w:r>", paste0("Betrachtet werden.</w:t></w:r>", ZITAT), x, fixed = TRUE)
        sub("(Betrachtet werden.</w:t></w:r>.*?</w:p>)", paste0("\\1", LISTE), x, perl = TRUE)
      })
      d <- tempfile(); dir.create(d)
      n <- utils::unzip(doc, list = TRUE)$Name
      utils::unzip(doc, exdir = d)
      # The templates bring their own custom XML parts.
      dir.create(file.path(d, "customXml"), showWarnings = FALSE)
      writeLines(STORE, file.path(d, "customXml/item99.xml"), useBytes = TRUE)
      pfad <<- tempfile(fileext = ".docx")
      l <- tempfile(); writeLines(c(n, "customXml/item99.xml"), l)
      old <- setwd(d); on.exit(setwd(old))
      stopifnot(system2("zip", c("-X", "-q", shQuote(pfad), "-@"), stdin = l) == 0L)
    }
    pfad
  }
})

test_that("outline nennt Zitate mit Schlüsseln und Literaturverzeichnisse", {
  o <- gba_outline(mit_zitaten())
  expect_equal(o$quellen, 2L)
  expect_length(o$zitate, 1L)
  z <- o$zitate[[1]]
  expect_equal(as.character(z$schluessel), c("mueller2020", "csr"))
  expect_equal(z$text, "[1,2]")
  b <- Filter(function(a) grepl("Betrachtet", a$text), o$absaetze)[[1]]$block
  expect_equal(z$block, b)
  expect_length(o$literaturverzeichnisse, 1L)
  json <- jsonlite::toJSON(gba_outline(gefuellt()), auto_unbox = TRUE)
  expect_match(json, "\"zitate\":[]", fixed = TRUE)
  ein <- jsonlite::toJSON(o$zitate, auto_unbox = TRUE)
  expect_match(ein, "\"schluessel\":[\"mueller2020\",\"csr\"]", fixed = TRUE)
})

test_that("ersetzen schützt Zitate des Add-ins wie EndNote-Zitate", {
  doc <- mit_zitaten()
  o <- gba_outline(doc)
  b <- o$zitate[[1]]$block
  a <- list(sha256 = o$sha256, aenderungen = list(list(op = "ersetzen", ziel = list(block = b), text = text_ops)))
  expect_error(gba_apply(doc, a, tempfile(fileext = ".docx")), "Zitate")
  a$aenderungen[[1]]$zitate_loeschen <- TRUE
  out <- tempfile(fileext = ".docx")
  gba_apply(doc, a, out)
  expect_match(dokument(out), "<w:tag w:val=\"zitat:k1\"/>.*<w:delText>\\[1,2\\]</w:delText>", perl = TRUE)
  # The reference list is a content control and no target at all.
  l <- o$literaturverzeichnisse[[1]]$block
  a2 <- list(sha256 = o$sha256, aenderungen = list(list(op = "ersetzen", ziel = list(block = l), text = text_ops)))
  expect_error(gba_apply(doc, a2, tempfile()), "lässt sich nicht bearbeiten")
})

test_that("Änderungen anderswo lassen Zitate, Liste und Bibliothek unberührt", {
  doc <- mit_zitaten()
  r <- anwenden(doc, list(op = "werte", ziel = list(tag = "tab_os"), tabelle = list(kopf = list(list("A", "B")), zeilen = list(list("eins", "neu"), list("drei", "vier")))))
  expect_equal(r$rep$aenderungen[[1]]$zellen, 1L)
  expect_match(r$xml, ZITAT, fixed = TRUE)
  expect_match(r$xml, LISTE, fixed = TRUE)
  expect_equal(teile(r$out)[["customXml/item99.xml"]], teile(doc)[["customXml/item99.xml"]])
  expect_equal(gba_outline(r$out)$zitate, gba_outline(doc)$zitate)
})
