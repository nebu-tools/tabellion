spec <- function(z = list(list("eins", "zwei"), list("drei", "vier")), k = list(list("A", "B")), ...) {
  list(kopf = k, zeilen = z, ...)
}
werte <- function(z, tag = "tab_os") list(op = "werte", ziel = list(tag = tag), tabelle = spec(z))
ins <- "<w:ins w:id=\"9\" w:author=\"Team\" w:date=\"2026-01-01T00:00:00Z\"><w:r><w:t>Team</w:t></w:r></w:ins>"
zaehle <- function(x, re) lengths(regmatches(x, gregexpr(re, x, perl = TRUE)))

test_that("Zeilen- und Spaltenspannen ergeben ein gültiges Raster", {
  g <- spec_grid(list(
    list(gba_zelle("Studie", zeile = 2L), gba_zelle("Effekt", spalte = 2L)),
    list("HR", "p"),
    list("A", "0,8", "0,1")
  ))
  expect_equal(g$breite, 3L)
  expect_true(g$rows[[2]][[1]]$cont)
  expect_error(spec_grid(list(list("a", "b"), list("c"))), "Spalten")
  expect_error(spec_grid(list(list(gba_zelle("a", zeile = 2L)))), "Tabellenende")
})

test_that("neue-tabelle rendert verbundene Zellen im Vorlagenstil", {
  s <- gba_tabelle(data.frame(a = "1", b = "2", c = "3"), kopf = list(
    list(gba_zelle("Studie", zeile = 2L), gba_zelle("Effekt", spalte = 2L)), list("HR", "p")
  ), titel = "Merge", tag = "tab_m")
  d <- tempfile("gba-spec-")
  typ <- gba_import("modul4", d)
  gba_json(s, file.path(d, "t.json"))
  src <- read_text(typ)
  id <- regmatches(src, regexpr("#antwort\\(\"b[0-9]+\"\\)", src))
  write_text(sub(id, paste0(id, "[#neue-tabelle(tabelle: json(\"t.json\"))]"), src, fixed = TRUE), typ)
  out <- file.path(d, "o.docx")
  capture.output(gba_build(typ, out = out))
  x <- dokument(out)
  expect_match(x, "<w:gridSpan w:val=\"2\"/>", fixed = TRUE)
  expect_match(x, "<w:vMerge w:val=\"restart\"/>", fixed = TRUE)
  expect_match(x, "<w:tblDescription w:val=\"gba:tab_m\"/>", fixed = TRUE)
  xml_gueltig(x)
  expect_equal(system2(typst_bin(), c("compile", shQuote(typ), shQuote(file.path(d, "p.pdf"))), stderr = FALSE), 0L)
})

test_that("gelöschter Text gelangt nicht ins Inhaltsverzeichnis", {
  doc <- aendere_docx(gefuellt(), function(x) {
    m <- regexpr("<w:p[ >](?:(?!</w:p>).)*?<w:pStyle w:val=\"berschrift2\"/>.*?</w:p>", x, perl = TRUE)
    p <- regmatches(x, m)
    neu <- sub("</w:p>$", "<w:del w:id=\"9\" w:author=\"T\" w:date=\"2026-01-01T00:00:00Z\"><w:r><w:delText>GELOESCHT</w:delText></w:r></w:del></w:p>", p)
    sub(p, neu, x, fixed = TRUE)
  })
  x <- anwenden(doc, list(op = "felder"))$xml
  expect_equal(zaehle(x, "GELOESCHT"), 1L)
  expect_equal(zaehle(x, "<w:t[^>]*>[^<]*GELOESCHT"), 0L)
})

test_that("Import nimmt fremde docx und schützt, was Typst nicht abbildet", {
  doc <- aendere_docx(gefuellt(), function(x) {
    x <- sub("</w:tcPr><w:p ", "</w:tcPr><w:sdt><w:sdtPr/><w:sdtContent><w:p><w:r><w:t>c</w:t></w:r></w:p></w:sdtContent></w:sdt><w:p ", x, fixed = TRUE)
    sub("<w:body>", "<w:body><w:sdt><w:sdtPr><w:tag w:val=\"x\"/></w:sdtPr><w:sdtContent><w:p><w:r><w:t>Block</w:t></w:r></w:p></w:sdtContent></w:sdt>", x, fixed = TRUE)
  })
  d <- tempfile("gba-frei-")
  typ <- gba_import(doc, d)
  src <- read_text(typ)
  expect_gte(zaehle(src, "#fest\\("), 2L)
  out <- file.path(d, "o.docx")
  capture.output(gba_build(typ, out = out))
  expect_identical(teile(out), teile(doc))
})

test_that("Änderung an geschütztem Feldinhalt bricht ab", {
  doc <- aendere_docx(gefuellt(), function(x) {
    sub("<w:t xml:space=\"preserve\">Betrachtet werden.</w:t>",
        "<w:t xml:space=\"preserve\">Betrachtet </w:t></w:r><w:sdt><w:sdtPr/><w:sdtContent><w:r><w:t>Sdt</w:t></w:r></w:sdtContent></w:sdt><w:r><w:t>Ende</w:t>", x, fixed = TRUE)
  })
  d <- tempfile("gba-sdt-")
  typ <- gba_import(doc, d)
  write_text(sub("#feld(0)[Sdt]", "#feld(0)[Anders]", read_text(typ), fixed = TRUE), typ)
  expect_error(gba_build(typ, out = file.path(d, "o.docx")), "geschützt")
})

test_that("Build bricht ab, wenn die docx seit dem Import geändert wurde", {
  d <- tempfile("gba-alt-")
  typ <- gba_import(gefuellt(), d)
  file.copy(aendere_docx(gefuellt(), function(x) sub("Betrachtet werden", "Team", x, fixed = TRUE)),
            file.path(d, basename(gefuellt())), overwrite = TRUE)
  expect_error(gba_build(typ, out = file.path(d, "o.docx")), "Neu importieren")
})

test_that("werte ändert nur abweichende Zellen, nachverfolgt und wiederholbar", {
  r <- anwenden(gefuellt(), werte(list(list("eins", "NEU"), list("drei", "vier"))))
  expect_equal(r$rep$aenderungen[[1]]$zellen, 1L)
  expect_equal(zaehle(r$xml, "<w:ins [^>]*w:author=\"Automatisierung\""), 1L)
  expect_match(r$xml, "<w:delText xml:space=\"preserve\">zwei</w:delText>")
  xml_gueltig(r$xml)
  # A second round with other values replaces the pending value and keeps the original.
  r2 <- anwenden(r$out, werte(list(list("eins", "NOCHMAL"), list("drei", "vier"))))
  expect_false(grepl(">NEU<", r2$xml, fixed = TRUE))
  expect_equal(zaehle(r2$xml, "<w:delText[^>]*>zwei"), 1L)
  expect_equal(zaehle(r2$xml, "<w:delText"), 1L)
  gleich <- anwenden(gefuellt(), werte(list(list("eins", "zwei"), list("drei", "vier"))))
  expect_identical(teile(gleich$out), teile(gefuellt()))
})

test_that("werte lässt offene Team-Änderungen und Zitate stehen", {
  doc <- aendere_docx(gefuellt(), function(x) {
    x <- sub("<w:t xml:space=\"preserve\">zwei</w:t></w:r>", paste0("<w:t xml:space=\"preserve\">zwei</w:t></w:r>", ins), x)
    sub("<w:t xml:space=\"preserve\">vier</w:t></w:r>", paste0(
      "<w:t xml:space=\"preserve\">vier</w:t></w:r><w:r><w:fldChar w:fldCharType=\"begin\"/></w:r>",
      "<w:r><w:instrText> ADDIN EN.CITE </w:instrText></w:r><w:r><w:fldChar w:fldCharType=\"end\"/></w:r>"), x)
  })
  r <- anwenden(doc, werte(list(list("eins", "X"), list("drei", "Y"))))
  expect_length(r$rep$offen, 2L)
  expect_match(r$rep$offen[[1]]$grund, "Team")
  expect_match(r$rep$offen[[2]]$grund, "Zitat")
  expect_match(r$xml, "EN.CITE", fixed = TRUE)
  expect_false(grepl("<w:ins [^>]*Automatisierung", r$xml, perl = TRUE))
})

test_that("Abweichende Tabellenform und unbekannte Ziele stoppen ohne Ausgabe", {
  out <- tempfile(fileext = ".docx")
  run <- function(...) gba_apply(gefuellt(), list(...), out)
  expect_error(run(aenderungen = list(werte(list(list("a", "b"))))), "Tabellenform")
  expect_error(run(aenderungen = list(werte(list(), "fehlt"))), "Keine Tabelle, Abbildung")
  expect_error(run(aenderungen = list(list(op = "werte", ziel = list(block = "b0001"), tabelle = spec()))), "sha256")
  expect_error(run(aenderungen = list(list(op = "einfuegen", platzhalter = "tab:x", tabelle = spec()))), "nicht gefunden")
  expect_false(file.exists(out))
})

test_that("ersetzen behält die alte Tabelle als Löschung, Wiederholung bleibt sauber", {
  s <- spec(list(list("1", "2", "3")), list(list("X", "Y", "Z")))
  op <- list(op = "ersetzen", ziel = list(tag = "tab_os"), tabelle = s)
  r <- anwenden(gefuellt(), op)
  xml_gueltig(r$xml)
  # Old and new table both carry the tag, only the new one can be a target.
  expect_equal(zaehle(r$xml, "gba:tab_os"), 2L)
  expect_match(r$xml, "<w:delText xml:space=\"preserve\">zwei</w:delText>")
  expect_gte(zaehle(r$xml, "<w:del [^>]*/></w:trPr>"), 3L)
  r2 <- anwenden(r$out, op)
  xml_gueltig(r2$xml)
  expect_equal(zaehle(r2$xml, "gba:tab_os"), 2L)
  expect_equal(zaehle(r2$xml, "<w:ins [^>]*/></w:trPr>"), 2L)
  expect_equal(zaehle(r2$xml, "<w:ins [^>]*/><w:ins [^>]*/>"), 0L)
  expect_equal(zaehle(r2$xml, "<w:rPr><w:ins [^>]*/><w:ins "), 0L)
})

test_that("Revisions-IDs bleiben über Runden hinweg eindeutig", {
  r <- anwenden(gefuellt(), werte(list(list("eins", "X"), list("drei", "vier"))))
  r2 <- anwenden(r$out, werte(list(list("eins", "X"), list("drei", "Y"))))
  ids <- regmatches(r2$xml, gregexpr("(?<=<w:(?:ins|del) w:id=\")[0-9]+", r2$xml, perl = TRUE))[[1]]
  expect_gte(length(ids), 4L)
  expect_false(anyDuplicated(ids) > 0L)
})

test_that("Getaggte Tabelle mit geschütztem Inhalt stoppt", {
  doc <- aendere_docx(gefuellt(), function(x) {
    sub("(<w:tblDescription w:val=\"gba:tab_os\"/>.*?</w:tcPr>)", "\\1<w:sdt><w:sdtPr/><w:sdtContent><w:p/></w:sdtContent></w:sdt>", x, perl = TRUE)
  })
  expect_error(anwenden(doc, werte(list(list("eins", "X"), list("drei", "vier")))), "geschützte")
})

test_that("einfuegen füllt den Platzhalter mit Beschriftung und nummeriert neu", {
  r <- anwenden(gefuellt(), list(op = "einfuegen", platzhalter = "tab:neu", tabelle = spec(tag = "tab_neu", titel = "Neu")))
  xml_gueltig(r$xml)
  expect_match(r$xml, "gba:tab_neu", fixed = TRUE)
  geloescht <- regmatches(r$xml, gregexpr("(?<=<w:delText xml:space=\"preserve\">)[^<]*", r$xml, perl = TRUE))[[1]]
  expect_equal(paste(geloescht, collapse = ""), "{{tab:neu}}")
  expect_gte(zaehle(r$xml, "<w:ins [^>]*><w:r>(?:(?!</w:r>).)*SEQ"), 1L)
  o <- gba_outline(r$out)
  expect_true("tab_neu" %in% unlist(lapply(o$tabellen, `[[`, "tag")))
  expect_match(Filter(function(t) identical(t$tag, "tab_neu"), o$tabellen)[[1]]$offen_von, "Automatisierung")
})

test_that("Mit Nachverfolgung getippter Platzhalter und Beschriftung bleiben lesbar", {
  doc <- aendere_docx(gefuellt(), function(x) {
    sub("<w:t xml:space=\"preserve\">Betrachtet werden.</w:t></w:r>", paste0(
      "</w:r><w:ins w:id=\"9\" w:author=\"Team\" w:date=\"2026-01-01T00:00:00Z\"><w:r><w:t>{{tab:team}}</w:t></w:r></w:ins>"), x, fixed = TRUE)
  })
  r <- anwenden(doc, list(op = "einfuegen", platzhalter = "tab:team", tabelle = spec(titel = "Team", tag = "tab_team")))
  xml_gueltig(r$xml)
  expect_match(r$xml, "<w:ins [^>]*Team[^>]*><w:del ", perl = TRUE)
  t <- Filter(function(x) identical(x$tag, "tab_team"), gba_outline(r$out)$tabellen)[[1]]
  expect_false(grepl("SEQ", t$titel, fixed = TRUE))
  expect_match(t$titel, "Team", fixed = TRUE)
})

test_that("outline liefert Auswahl, und block wirkt mit passender sha256", {
  o <- gba_outline(gefuellt())
  t <- Filter(function(x) identical(x$tag, "tab_os"), o$tabellen)[[1]]
  expect_match(t$titel, "OS", fixed = TRUE)
  expect_true(any(vapply(o$platzhalter, function(p) p$name == "tab:neu", NA)))
  expect_gt(length(o$ueberschriften), 10L)
  out <- tempfile(fileext = ".docx")
  gba_apply(gefuellt(), list(sha256 = o$sha256, aenderungen = list(
    list(op = "werte", ziel = list(block = t$block), tabelle = spec(list(list("eins", "BLOCK"), list("drei", "vier"))))
  )), out)
  expect_match(dokument(out), ">BLOCK<", fixed = TRUE)
})

test_that("EndNote-Felder und Team-Änderungen überstehen eine Tabellenänderung", {
  doc <- aendere_docx(gefuellt(), function(x) {
    sub("<w:t xml:space=\"preserve\">Betrachtet werden.</w:t></w:r>", paste0(
      "<w:t xml:space=\"preserve\">Betrachtet werden.</w:t></w:r>", ins,
      "<w:r><w:fldChar w:fldCharType=\"begin\"><w:fldData xml:space=\"preserve\">QUJD</w:fldData></w:fldChar></w:r>",
      "<w:r><w:instrText> ADDIN EN.CITE </w:instrText></w:r><w:r><w:fldChar w:fldCharType=\"separate\"/></w:r>",
      "<w:r><w:t>(1)</w:t></w:r><w:r><w:fldChar w:fldCharType=\"end\"/></w:r>"), x, fixed = TRUE)
  })
  r <- anwenden(doc, werte(list(list("eins", "X"), list("drei", "vier"))))
  expect_match(r$xml, ">QUJD<", fixed = TRUE)
  expect_match(r$xml, ins, fixed = TRUE)
})

bild <- function(farbe = "red") {
  p <- tempfile(fileext = ".png")
  grDevices::png(p, width = 200, height = 100); graphics::par(bg = farbe, mar = rep(0, 4)); plot.new(); grDevices::dev.off()
  p
}
platzhalter <- function(doc, name) doc
klar <- function(x) gsub("<[^>]+>", "", drop_removed(x))
km <- function(n = "12", ...) list(baustein = "ereigniszeit", werte = list(n = n, studie = "A, B"), ...)
ziel <- function(tag) list(tag = tag)

test_that("einfuegen setzt Textbausteine mit Werten nachverfolgt ein", {
  r <- anwenden(platzhalter(gefuellt(), "met"), list(op = "einfuegen", platzhalter = "met", text = km(tag = "met_km")))
  xml_gueltig(r$xml)
  expect_match(klar(r$xml), "12 Personen aus A, B", fixed = TRUE)
  expect_match(r$xml, "<w:numPr>", fixed = TRUE)
  # Four paragraphs, each tagged and tracked.
  expect_equal(zaehle(r$xml, "w:name=\"_gba_met_km_[0-9]+\""), 4L)
  o <- gba_outline(r$out)
  expect_true("met_km" %in% unlist(lapply(o$absaetze, `[[`, "tag")))
})

test_that("Fehlender Wert und unbekannter Baustein stoppen", {
  doc <- platzhalter(gefuellt(), "met")
  expect_error(anwenden(doc, list(op = "einfuegen", platzhalter = "met", text = list(baustein = "ereigniszeit", werte = list(n = 1)))), "studie")
  expect_error(anwenden(doc, list(op = "einfuegen", platzhalter = "met", text = list(baustein = "gibtsnicht"))), "Unbekannter Textbaustein")
})

test_that("Formeln und Fußnoten im Baustein sind nachverfolgt", {
  r <- anwenden(platzhalter(gefuellt(), "met"), list(op = "einfuegen", platzhalter = "met",
    text = list(baustein = "subgruppen", werte = list(alpha = "0,05"))))
  xml_gueltig(r$xml)
  expect_match(r$xml, "<w:ins [^>]*><m:oMath>", perl = TRUE)
  expect_match(r$xml, "<w:ins [^>]*><w:r>(?:(?!</w:r>).)*footnoteReference", perl = TRUE)
})

test_that("Text lässt sich wiederholt ersetzen und bleibt auffindbar", {
  ein <- anwenden(platzhalter(gefuellt(), "met"), list(op = "einfuegen", platzhalter = "met", text = km(tag = "met_km")))
  op <- function(n) list(op = "ersetzen", ziel = ziel("met_km"), text = km(n))
  r2 <- anwenden(ein$out, op("99"))
  xml_gueltig(r2$xml)
  # Still pending: the earlier text goes, nothing piles up.
  expect_false(grepl("12 Personen", klar(r2$xml), fixed = TRUE))
  expect_match(klar(r2$xml), "99 Personen", fixed = TRUE)
  expect_equal(zaehle(r2$xml, "w:name=\"_gba_met_km_"), 4L)
  # The team accepted it: the old text becomes a tracked deletion and keeps its tag.
  angenommen <- aendere_docx(r2$out, function(x) {
    x <- gsub("<w:rPr><w:ins [^>]*/>", "<w:rPr>", x, perl = TRUE)
    x <- gsub("<w:del (?:[^>]*[^/])?>.*?</w:del>", "", x, perl = TRUE)
    x <- gsub("<w:rPr><w:del [^>]*/>", "<w:rPr>", x, perl = TRUE)
    x <- gsub("<w:ins (?:[^>]*[^/])?>|</w:ins>", "", x, perl = TRUE)
    x <- gsub("<w:rPr></w:rPr>", "", x, fixed = TRUE)
    gsub("<w:r><w:t[^>]*>\\{\\{met\\}\\}</w:t></w:r>", "", x, perl = TRUE)
  })
  r3 <- anwenden(angenommen, op("7"))
  xml_gueltig(r3$xml)
  expect_gte(zaehle(r3$xml, "<w:del [^>]*>"), 4L)
  expect_match(klar(r3$xml), "7 Personen", fixed = TRUE)
  r4 <- anwenden(r3$out, op("8"))
  expect_match(klar(r4$xml), "8 Personen", fixed = TRUE)
  expect_false(grepl("7 Personen", gsub("<[^>]+>", "", r4$xml), fixed = TRUE))
  expect_match(gsub("<[^>]+>", "", r4$xml), "99 Personen", fixed = TRUE)
})

test_that("Text mit Team-Änderung, Zitat oder Pflichtblock bleibt unberührt", {
  o <- gba_outline(gefuellt())
  b <- Filter(function(a) grepl("Betrachtet", a$text), o$absaetze)[[1]]$block
  op <- function(...) list(sha256 = NULL, aenderungen = list(list(op = "ersetzen", ziel = list(block = b), text = km(), ...)))
  mit <- function(doc, ...) { a <- op(...); a$sha256 <- unname(tools::sha256sum(doc)); out <- tempfile(fileext = ".docx"); gba_apply(doc, a, out); dokument(out) }
  x <- mit(gefuellt())
  expect_match(x, "<w:delText[^>]*>Betrachtet", perl = TRUE)
  team <- aendere_docx(gefuellt(), function(x) sub("Betrachtet werden.</w:t></w:r>", paste0("Betrachtet werden.</w:t></w:r>", ins), x, fixed = TRUE))
  expect_error(mit(team), "Team")
  zitat <- aendere_docx(gefuellt(), function(x) sub("Betrachtet werden.</w:t></w:r>", paste0(
    "Betrachtet werden.</w:t></w:r><w:r><w:fldChar w:fldCharType=\"begin\"/></w:r><w:r><w:instrText> ADDIN EN.CITE </w:instrText></w:r>",
    "<w:r><w:fldChar w:fldCharType=\"end\"/></w:r>"), x, fixed = TRUE))
  expect_error(mit(zitat), "EndNote")
  expect_match(mit(zitat, zitate_loeschen = TRUE), "delInstrText", fixed = TRUE)
  k <- Filter(function(u) u$stil == "berschrift2", o$ueberschriften)[[1]]$block
  a <- list(sha256 = o$sha256, aenderungen = list(list(op = "ersetzen", ziel = list(block = k), text = km())))
  expect_error(gba_apply(gefuellt(), a, tempfile()), "Nur Text")
})

test_that("Absatz mitten im Literaturverzeichnis stoppt", {
  doc <- aendere_docx(gefuellt(), function(x) {
    sub("<w:t xml:space=\"preserve\">Betrachtet werden.</w:t></w:r>", paste0(
      "<w:fldChar w:fldCharType=\"begin\"/></w:r><w:r><w:instrText> ADDIN EN.REFLIST </w:instrText></w:r><w:r><w:fldChar w:fldCharType=\"separate\"/></w:r></w:p>",
      "<w:p><w:r><w:t>Quelle eins</w:t></w:r></w:p><w:p><w:r><w:fldChar w:fldCharType=\"end\"/></w:r>"), x, fixed = TRUE)
  })
  o <- gba_outline(doc)
  b <- Filter(function(a) a$text == "Quelle eins", o$absaetze)[[1]]$block
  a <- list(sha256 = o$sha256, aenderungen = list(list(op = "ersetzen", ziel = list(block = b), text = km(), zitate_loeschen = TRUE)))
  expect_error(gba_apply(doc, a, tempfile()), "Feld")
})

test_that("Abbildungen lassen sich einfügen, finden und ersetzen", {
  ein <- list(op = "einfuegen", platzhalter = "abb", abbildung = list(datei = bild(), titel = "KM OS", tag = "abb_os", breite = "10cm"))
  r <- anwenden(platzhalter(gefuellt(), "abb"), ein)
  xml_gueltig(r$xml)
  expect_match(r$xml, "descr=\"gba:abb_os\"", fixed = TRUE)
  expect_match(r$xml, "<w:ins [^>]*><w:r><w:drawing>", perl = TRUE)
  expect_match(r$xml, "cx=\"3600000\"", fixed = TRUE)
  o <- gba_outline(r$out)$abbildungen
  expect_equal(o[[1]]$tag, "abb_os")
  expect_match(o[[1]]$titel, "KM OS", fixed = TRUE)
  # Round two: a new media part, no clash with the first.
  r2 <- anwenden(r$out, list(op = "ersetzen", ziel = ziel("abb_os"), abbildung = list(datei = bild("blue"))))
  xml_gueltig(r2$xml)
  expect_equal(zaehle(r2$xml, "descr=\"gba:abb_os\""), 1L)
  expect_true(all(c("word/media/gba_abbildung1.png", "word/media/gba_abbildung2.png") %in% utils::unzip(r2$out, list = TRUE)$Name))
  expect_match(r2$xml, "r:embed=\"rIdGba2\"", fixed = TRUE)
  expect_match(r2$xml, "descr=\"gba:abb_os\"", fixed = TRUE)
  expect_error(anwenden(r$out, list(op = "ersetzen", ziel = ziel("abb_os"), tabelle = spec())), "abbildung")
  expect_error(anwenden(platzhalter(gefuellt(), "abb"), list(op = "einfuegen", platzhalter = "abb", abbildung = list(datei = "/gibt/es/nicht.png"))), "nicht gefunden")
})

test_that("Relative Bildpfade gelten ab der Ops-Datei", {
  d <- tempfile(); dir.create(d)
  file.copy(bild(), file.path(d, "km.png"))
  json <- file.path(d, "ops.json")
  jsonlite::write_json(list(aenderungen = list(list(op = "einfuegen", platzhalter = "abb", abbildung = list(datei = "km.png")))), json, auto_unbox = TRUE)
  out <- tempfile(fileext = ".docx")
  gba_apply(platzhalter(gefuellt(), "abb"), json, out)
  expect_match(dokument(out), "<w:drawing>", fixed = TRUE)
})

test_that("titel ändert die Beschriftung nachverfolgt und lässt die Nummer stehen", {
  r <- anwenden(gefuellt(), list(op = "titel", ziel = ziel("tab_os"), titel = "Gesamtüberleben"))
  xml_gueltig(r$xml)
  expect_match(r$xml, "<w:delText[^>]*>[^<]*OS<", perl = TRUE)
  expect_match(r$xml, "<w:ins [^>]*><w:r>(?:(?!</w:r>).)*Gesamtüberleben", perl = TRUE)
  expect_match(r$xml, "SEQ Tabelle", fixed = TRUE)
  # The list of tables follows the new title.
  expect_gte(zaehle(r$xml, "Gesamtüberleben"), 2L)
  r2 <- anwenden(r$out, list(op = "titel", ziel = ziel("tab_os"), titel = "Noch anders"))
  xml_gueltig(r2$xml)
  expect_false(grepl("Gesamtüberleben", r2$xml, fixed = TRUE))
  expect_equal(zaehle(r2$xml, "<w:delText[^>]*>[^<]*OS<"), 1L)
  gleich <- anwenden(gefuellt(), list(op = "titel", ziel = ziel("tab_os"), titel = "OS"))
  expect_identical(teile(gleich$out), teile(gefuellt()))
})

test_that("ersetzen und werte übernehmen einen neuen Titel, auch nach einem Ersetzen", {
  s <- spec(list(list("1", "2", "3")), list(list("X", "Y", "Z")), titel = "Neu A")
  r <- anwenden(gefuellt(), list(op = "ersetzen", ziel = ziel("tab_os"), tabelle = s))
  expect_match(r$xml, "<w:ins [^>]*><w:r>(?:(?!</w:r>).)*Neu A", perl = TRUE)
  s$titel <- "Neu B"
  r2 <- anwenden(r$out, list(op = "ersetzen", ziel = ziel("tab_os"), tabelle = s))
  xml_gueltig(r2$xml)
  expect_false(grepl("Neu A", r2$xml, fixed = TRUE))
  t <- Filter(function(x) identical(x$tag, "tab_os"), gba_outline(r2$out)$tabellen)[[1]]
  expect_match(t$titel, "Neu B", fixed = TRUE)
  w <- anwenden(gefuellt(), list(op = "werte", ziel = ziel("tab_os"), tabelle = spec(titel = "Mit Werten", list(list("eins", "zwei"), list("drei", "vier")))))
  expect_match(w$xml, "Mit Werten", fixed = TRUE)
  expect_equal(w$rep$aenderungen[[1]]$zellen, 0L)
})

test_that("Typst-Pfad kennt Bausteine und Abbildungskennungen", {
  png <- bild()
  b <- bauen("modul4", function(s) {
    id <- regmatches(s, regexpr("#antwort\\(\"b[0-9]+\"\\)", s))
    sub(id, paste0(id, "[#baustein(\"ereigniszeit\", werte: (n: 5, studie: \"X\"), tag: \"met_t\")\n\n",
                   "#abbildung(image(\"bild.png\"), beschriftung: [B], tag: \"abb_t\")]"), s, fixed = TRUE)
  }, png)
  xml_gueltig(b$xml)
  expect_match(klar(b$xml), "5 Personen", fixed = TRUE)
  expect_equal(zaehle(b$xml, "w:name=\"_gba_met_t_"), 4L)
  expect_match(b$xml, "descr=\"gba:abb_t\"", fixed = TRUE)
  r <- anwenden(b$out, list(op = "ersetzen", ziel = ziel("met_t"), text = km("6")))
  expect_match(klar(r$xml), "6 Personen", fixed = TRUE)
  expect_equal(unlist(lapply(gba_outline(r$out)$abbildungen, `[[`, "tag")), "abb_t")
})

test_that("Eigener Baustein mit Formel lässt sich erneut ersetzen", {
  sg <- list(baustein = "subgruppen", werte = list(alpha = "0,05"), tag = "met_sg")
  r <- anwenden(gefuellt(), list(op = "einfuegen", platzhalter = "met", text = sg))
  sg$werte$alpha <- "0,01"
  r2 <- anwenden(r$out, list(op = "ersetzen", ziel = ziel("met_sg"), text = sg))
  xml_gueltig(r2$xml)
  expect_equal(zaehle(r2$xml, "<m:oMath>"), 1L)
})

test_that("Fremder Absatz zwischen getaggtem Text stoppt", {
  r <- anwenden(gefuellt(), list(op = "einfuegen", platzhalter = "met", text = km(tag = "met_km")))
  doc <- aendere_docx(r$out, function(x) {
    sub("(<w:bookmarkEnd w:id=\"[0-9]+\"/></w:p>)(<w:p><w:pPr>(?:(?!</w:p>).)*_gba_met_km)", "\\1<w:p><w:r><w:t>Dazwischen</w:t></w:r></w:p>\\2", x, perl = TRUE)
  })
  expect_error(anwenden(doc, list(op = "ersetzen", ziel = ziel("met_km"), text = km("1"))), "zusammenhängend")
})
