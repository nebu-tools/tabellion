test_that("Antwort ersetzt nur den Platzhalter, Rest bleibt unveraendert", {
  b <- bauen("modul4", \(s) subst(s, "#antwort(\"b0141\")", "#antwort(\"b0141\")[Eins\n\nZwei]"))
  expect_equal(b$rep$stats$geaendert, 1L)
  tpl <- teile(modul4); neu <- teile(b$out)
  expect_identical(names(which(tpl != neu)), "word/document.xml")
  expect_match(b$xml, ">Eins</w:t>", fixed = TRUE)
  expect_match(b$xml, ">Zwei</w:t>", fixed = TRUE)
})

test_that("Felder ersetzen Platzhalter ueber Run-Grenzen und entfernen die Markierung", {
  b <- bauen("modul4", \(s) {
    s <- subst(s, "    // wirkstoff: \"...\", handelsname: \"...\", unternehmer: \"...\",", "    wirkstoff: \"Aaa\", stand: \"01.02.2026\",")
    s
  })
  text <- gsub("<[^>]+>", "", b$xml)
  expect_false(grepl("&lt;&lt;Wirkstoff", text, fixed = TRUE))
  expect_false(grepl("jjjj", text, fixed = TRUE))
  expect_match(text, "Stand: 01.02.2026", fixed = TRUE)
  i <- regexpr(">Aaa</w:t>", b$xml, fixed = TRUE)
  expect_false(grepl("highlight", substr(b$xml, i - 200L, i), fixed = TRUE))
})

test_that("Geaenderte Beschriftung behaelt das SEQ-Feld", {
  b <- bauen("modul4", \(s) subst(s, "Operationalisierung von \\<Endpunkt xxx\\>]", "Operationalisierung von Tod]"))
  expect_match(b$xml, "Operationalisierung von Tod", fixed = TRUE)
  expect_equal(lengths(regmatches(b$xml, gregexpr("SEQ Tabelle", b$xml, fixed = TRUE))),
               lengths(regmatches(dokument(modul4), gregexpr("SEQ Tabelle", dokument(modul4), fixed = TRUE))))
})

test_that("Wiederholte Bloecke bekommen eindeutige Lesezeichen", {
  b <- bauen("modul4", \(s) {
    z <- "#ueberschrift(5, \"b0381\")[\\<Endpunkt xxx\\> – RCT]"
    subst(s, z, paste0(z, "\n\n", z))
  })
  ids <- regmatches(b$xml, gregexpr("<w:bookmarkStart [^>]*w:name=\"[^\"]+\"", b$xml))[[1]]
  namen <- sub(".*w:name=\"", "", ids)
  expect_false(anyDuplicated(namen) > 0L)
  expect_gt(lengths(regmatches(b$xml, gregexpr("&lt;Endpunkt xxx&gt; – RCT</w:t>", b$xml))), 1L)
})

test_that("Kontrollkaestchen wird gesetzt", {
  b <- bauen("modul4", \(s) subst(s, "#absatz(\"b0773\")[#kreuz(0, false)", "#absatz(\"b0773\")[#kreuz(0, true)"))
  expect_equal(lengths(regmatches(b$xml, gregexpr("<w:checked/>", b$xml))), 1L)
})

test_that("Fehlende Pflichtelemente werden gemeldet", {
  b <- bauen("modul4", \(s) gsub("#frage\\(\"b0419\"\\)[^\n]*\n", "\n", s))
  expect_true("b0419" %in% b$rep$fehlend_pflicht)
})

test_that("Nicht abbildbares Typst-Element bricht ab statt Inhalt zu verlieren", {
  expect_error(
    bauen("modul4", \(s) subst(s, "#antwort(\"b0141\")", "#antwort(\"b0141\")[#rect[x]]")),
    "lässt sich nicht"
  )
})

test_that("Nummern, Querverweise und Verzeichnis folgen dem Inhalt", {
  b <- bauen("modul4", \(s) {
    s <- subst(s, "#antwort(\"b0141\")", "#antwort(\"b0141\")[#neue-tabelle(beschriftung: [Neu], kopf: ([A],), zeilen: (([1],),))]")
    z <- "#ueberschrift(5, \"b0381\")[\\<Endpunkt xxx\\> – RCT]"
    subst(s, z, paste0(z, "\n\n#ueberschrift(5, \"b0381\")[Zweiter Endpunkt]"))
  })
  text <- gsub("<[^>]+>", "", gsub("<w:noBreakHyphen/>", "-", b$xml))
  expect_true(grepl("Tabelle 4-1: Neu", text, fixed = TRUE))
  expect_false(grepl("Tabelle 4-1: Studienpool", text, fixed = TRUE))
  anker <- regmatches(b$xml, gregexpr("<w:hyperlink [^>]*w:anchor=\"[^\"]+\"", b$xml))[[1]]
  namen <- sub(".*w:anchor=\"", "", sub("\"$", "", anker))
  ziele <- sub(".*w:name=\"", "", sub("\"$", "", regmatches(b$xml, gregexpr("<w:bookmarkStart [^>]*w:name=\"[^\"]+\"", b$xml))[[1]]))
  expect_length(setdiff(namen, ziele), 0L)
  expect_gt(lengths(gregexpr("Zweiter Endpunkt", text, fixed = TRUE)), 1L)
  anz <- function(k) lengths(regmatches(b$xml, gregexpr(sprintf("fldCharType=\"%s\"", k), b$xml)))
  expect_equal(anz("begin"), anz("end"))
})

test_that("Gefuellte docx enthaelt keine Aktualisierungsrueckfrage", {
  b <- bauen("modul4", \(s) subst(s, "#antwort(\"b0141\")", "#antwort(\"b0141\")[Eins]"))
  d <- tempfile(); utils::unzip(b$out, "word/settings.xml", exdir = d)
  expect_false(grepl("updateFields", read_text(file.path(d, "word/settings.xml")), fixed = TRUE))
})

test_that("Jedes Modul laesst sich ausfuellen", {
  for (m in module) {
    b <- bauen(m, \(s) sub("    // wirkstoff: \"...\", handelsname: \"...\", unternehmer: \"...\",",
                            "    wirkstoff: \"Aaa\",", s, fixed = TRUE))
    anz <- function(k) lengths(regmatches(b$xml, gregexpr(sprintf("fldCharType=\"%s\"", k), b$xml)))
    expect_equal(anz("begin"), anz("end"), info = m)
    expect_silent(xml2::read_xml(b$xml))
  }
})

test_that("Verzeichnisse behalten alle Eintraege, auch bei fldSimple und geteilten Feldcodes", {
  zahl <- function(x) lengths(regmatches(x, gregexpr("w:pStyle w:val=\"Abbildungsverzeichnis\"", x)))
  for (m in c("k1", "k3", "k4")) {
    b <- bauen(m, \(s) sub("    // wirkstoff: \"...\", handelsname: \"...\", unternehmer: \"...\",",
                            "    wirkstoff: \"Aaa\",", s, fixed = TRUE))
    expect_equal(zahl(b$xml), zahl(dokument(gba_template(m))), info = m)
  }
})

test_that("Wiederholte Antwort mit Fussnote hat fuer jeden Verweis eine Definition", {
  b <- bauen("modul4", \(s) {
    z <- "#antwort(\"b0141\")"
    subst(s, z, "#antwort(\"b0141\")[Eins#footnote[Quelle]]\n\n#antwort(\"b0141\")[Zwei#footnote[Quelle]]")
  })
  d <- tempfile(); utils::unzip(b$out, "word/footnotes.xml", exdir = d)
  fn <- read_text(file.path(d, "word/footnotes.xml"))
  ids <- sub(".*w:id=\"([0-9]+)\".*", "\\1", regmatches(b$xml, gregexpr("<w:footnoteReference [^>]*>", b$xml))[[1]])
  for (i in ids) expect_match(fn, sprintf("<w:footnote w:id=\"%s\"", i), fixed = TRUE)
  expect_false(anyDuplicated(ids) > 0L)
})

test_that("mit und Wiederholung aktualisieren das Verzeichnis", {
  b <- bauen("modul4", \(s) {
    z <- "#ueberschrift(5, \"b0381\")[\\<Endpunkt xxx\\> – RCT]"
    subst(s, z, paste0("#mit((\"<Endpunkt xxx>\": \"Tod\"), [", z, "])"))
  })
  toc <- regmatches(b$xml, gregexpr("<w:p>(?:(?!</w:p>).)*Verzeichnis5(?:(?!</w:p>).)*</w:p>", b$xml, perl = TRUE))[[1]]
  expect_true(any(grepl("Tod – RCT", toc, fixed = TRUE)))
})

test_that("Abbildung funktioniert auch ohne Beispielbeschriftung in der Vorlage", {
  png <- tempfile(fileext = ".png")
  grDevices::png(png); plot(1); grDevices::dev.off()
  b <- bauen("modul3", \(s) {
    id <- regmatches(s, regexpr("#antwort\\(\"b[0-9]+\"\\)", s))
    subst(s, id, sprintf("%s[#abbildung(image(\"bild.png\"), beschriftung: [Bild])]", id))
  }, png)
  expect_match(b$xml, "SEQ Abbildung", fixed = TRUE)
  expect_match(gsub("<[^>]+>", "", b$xml), "Abbildung [^:]*: Bild")
  expect_false(grepl(TOC_LEER, b$xml, fixed = TRUE) && lengths(gregexpr(TOC_LEER, b$xml, fixed = TRUE)) > 1L)
  verzeichnis <- regmatches(b$xml, gregexpr("<w:p>(?:(?!</w:p>).)*Abbildungsverzeichnis(?:(?!</w:p>).)*</w:p>", b$xml, perl = TRUE))[[1]]
  expect_true(any(grepl(": Bild", gsub("<[^>]+>", "", verzeichnis), fixed = TRUE)))
  anz <- function(k) lengths(regmatches(b$xml, gregexpr(sprintf("fldCharType=\"%s\"", k), b$xml)))
  expect_equal(anz("begin"), anz("end"))
})

test_that("Verzeichnis ohne Eintraege verliert alte Eintraege und bleibt ein Feld", {
  b <- bauen("modul4", \(s) gsub("#beschriftung\\(\"(b[0-9]+)\"\\)\\[#feld\\(0\\)\\[Abbildung[^\n]*\n", "\n", s))
  expect_match(b$xml, TOC_LEER, fixed = TRUE)
  anz <- function(k) lengths(regmatches(b$xml, gregexpr(sprintf("fldCharType=\"%s\"", k), b$xml)))
  expect_equal(anz("begin"), anz("end"))
})

test_that("Nummerierte Listen beginnen neu, Punkte bleiben Punkte", {
  b <- bauen("modul4", \(s) subst(s, "#antwort(\"b0141\")",
    "#antwort(\"b0141\")[+ eins\n+ zwei\n\nText\n\n+ neu\n\n- Punkt]"))
  d <- tempfile(); utils::unzip(b$out, "word/numbering.xml", exdir = d)
  nx <- read_text(file.path(d, "word/numbering.xml"))
  expect_equal(lengths(regmatches(nx, gregexpr("<w:startOverride w:val=\"1\"/></w:lvlOverride></w:num>", nx))) >= 2L, TRUE)
  liste <- function(wort) first_match(sprintf("<w:numId w:val=\"([0-9]+)\"/></w:numPr></w:pPr><w:r>(?:<w:rPr>(?:(?!</w:r>).)*</w:rPr>)?<w:t[^>]*>%s<", wort), b$xml)
  expect_equal(liste("eins"), liste("zwei"))
  expect_false(liste("eins") == liste("neu"))
  expect_false(liste("Punkt") %in% c(liste("eins"), liste("neu")))
  xml2::read_xml(nx)
})

test_that("Formeln werden native Word-Formeln, Zellen und Absaetze behalten sie", {
  b <- bauen("modul4", \(s) {
    s <- subst(s, "#antwort(\"b0141\")", "#antwort(\"b0141\")[HR $x^2_i + frac(a, b) + sqrt(y)$\n\n$ E = m c^2 $]")
    subst(s, "([\\<Studie 1\\>], [],),", "([$p < 0.05$], [],),")
  })
  expect_match(b$xml, "<m:sSubSup>", fixed = TRUE)
  expect_match(b$xml, "<m:f>", fixed = TRUE)
  expect_match(b$xml, "<m:oMathPara>", fixed = TRUE)
  expect_silent(xml2::read_xml(b$xml))
  expect_gt(lengths(gregexpr("<m:oMath>", b$xml, fixed = TRUE)), 2L)
})

test_that("SVG-Abbildungen werden als PNG eingebettet", {
  svg <- tempfile(fileext = ".svg")
  writeLines("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100\" height=\"50\"><rect width=\"100\" height=\"50\"/></svg>", svg)
  b <- bauen("modul4", \(s) subst(s, "#antwort(\"b0141\")",
    "#antwort(\"b0141\")[#abbildung(image(\"bild.svg\"), beschriftung: [V])]"), NULL, svg)
  expect_match(b$xml, "SEQ Abbildung", fixed = TRUE)
})

test_that("Layout-Huellen geben ihren Inhalt weiter", {
  b <- bauen("modul4", \(s) subst(s, "#antwort(\"b0141\")", "#antwort(\"b0141\")[#box[Innen] und #align(center)[Mitte]]"))
  expect_match(b$xml, "Innen", fixed = TRUE)
  expect_match(b$xml, "Mitte", fixed = TRUE)
})

test_that("Nummerierung gilt auch in Huellen und Absaetzen", {
  b <- bauen("modul4", \(s) {
    s <- subst(s, "#antwort(\"b0141\")", "#antwort(\"b0141\")[#align(center)[+ huelle]]")
    subst(s, "#absatz(\"b0009\")[Seite]", "#absatz(\"b0009\")[+ absatz]")
  })
  d <- tempfile(); utils::unzip(b$out, "word/numbering.xml", exdir = d)
  nx <- read_text(file.path(d, "word/numbering.xml"))
  expect_gte(lengths(regmatches(nx, gregexpr("<w:startOverride ", nx))), 2L)
})

test_that("Operatoren und abgesetzte Formeln funktionieren auch in Zellen", {
  b <- bauen("modul4", \(s) {
    s <- subst(s, "#antwort(\"b0141\")", "#antwort(\"b0141\")[$sin(x)$]")
    subst(s, "([\\<Studie 1\\>], [],),", "([$ a = b $], [],),")
  })
  expect_match(b$xml, ">sin</m:t>", fixed = TRUE)
  expect_match(b$xml, "<m:oMathPara>", fixed = TRUE)
  expect_silent(xml2::read_xml(b$xml))
})

test_that("Nummerierte Listen in Tabellenzellen bleiben eine Liste", {
  b <- bauen("modul4", \(s) subst(s, "([\\<Studie 1\\>], [],),", "([+ erste\n+ zweite], [],),"))
  liste <- function(wort) first_match(sprintf("<w:numId w:val=\"([0-9]+)\"/></w:numPr></w:pPr><w:r>(?:<w:rPr>(?:(?!</w:r>).)*</w:rPr>)?<w:t[^>]*>%s<", wort), b$xml)
  expect_false(is.na(liste("erste")))
  expect_equal(liste("erste"), liste("zweite"))
  d <- tempfile(); utils::unzip(b$out, "word/numbering.xml", exdir = d)
  expect_match(read_text(file.path(d, "word/numbering.xml")), sprintf("w:numId=\"%s\"", liste("erste")), fixed = TRUE)
})

test_that("Nummerierte Liste im Kopf einer neuen Tabelle", {
  b <- bauen("modul4", \(s) subst(s, "#antwort(\"b0141\")",
    "#antwort(\"b0141\")[#neue-tabelle(beschriftung: [T], kopf: ([+ kopf],), zeilen: (([x],),))]"))
  d <- tempfile(); utils::unzip(b$out, "word/numbering.xml", exdir = d)
  expect_match(read_text(file.path(d, "word/numbering.xml")), "<w:startOverride ", fixed = TRUE)
})
