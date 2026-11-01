# Fuellt die Modul-4-Vorlage mit oeffentlichen Beispieldaten (survival::colon).
# Die Zahlen sind Demo-Daten und kein Dossierinhalt.
#
#   rv run gba.R fetch
#   rv run examples/modul4/demo.R
root <- normalizePath(".")
options(gba.root = root)
for (f in list.files("R", "\\.R$", full.names = TRUE)) source(f)
library(survival)

proj <- file.path(root, "examples/modul4/projekt")
tname <- "2025-11-18_Anl2_6_Modul4"
gba_import("modul4", proj)

# ---- Statistik -------------------------------------------------------------
d <- subset(colon, rx != "Lev")
d$rx <- droplevels(d$rx)
d$arm <- ifelse(d$rx == "Lev+5FU", "Lev+5FU", "Beobachtung")
endpunkte <- list(
  list(name = "Gesamtüberleben", etype = 2L, slug = "os"),
  list(name = "Rezidivfreies Überleben", etype = 1L, slug = "rfs")
)
ergebnis <- lapply(endpunkte, function(e) {
  x <- d[d$etype == e$etype, ]
  fit <- coxph(Surv(time, status) ~ I(arm == "Lev+5FU"), data = x)
  s <- summary(fit)
  km <- survfit(Surv(time / 365.25, status) ~ arm, data = x)
  median_j <- summary(km)$table[, "median"]
  png(file.path(proj, paste0("km_", e$slug, ".png")), width = 1800, height = 1100, res = 200)
  par(mar = c(4.2, 4.2, 1, 1), family = "sans")
  plot(km, lty = 1:2, lwd = 2, xlab = "Zeit (Jahre)", ylab = "Anteil ohne Ereignis")
  legend("bottomleft", c("Beobachtung", "Lev+5FU"), lty = 1:2, lwd = 2, bty = "n")
  dev.off()
  ar <- split(x, x$arm)
  zeile <- function(nm) {
    g <- ar[[nm]]
    c(nm, nrow(g), sum(g$status), gba_zahl(median_j[paste0("arm=", nm)], 1))
  }
  list(
    name = e$name, slug = e$slug,
    zeilen = list(
      c("NCCTG 89-46-51", zeile("Lev+5FU"), gba_ki(s$conf.int[1], s$conf.int[3], s$conf.int[4]), gba_p(s$coefficients[5])),
      c("", zeile("Beobachtung"), "", "")
    )
  )
})
gba_json(ergebnis, file.path(proj, "ergebnisse.json"))

# ---- Typst-Quelle ----------------------------------------------------------
# Im echten Projekt bearbeitet das Team die generierte .typ-Datei direkt. Die
# Demo macht dieselben Aenderungen per Skript, damit sie reproduzierbar sind.
typ <- file.path(proj, paste0(tname, ".typ"))
src <- readLines(typ, encoding = "UTF-8")

src <- sub("^    // wirkstoff: .*$", paste0(
  "    wirkstoff: \"Fluoruracil\", handelsname: \"Demozil\", unternehmer: \"Muster Pharma GmbH\","
), src)
src <- sub("^    // kodierung: .*$", "    kodierung: \"A\", anwendungsgebiet: \"Kolonkarzinom Stadium III\", stand: \"02.11.2026\",", src)

ersetze <- function(muster, neu) {
  i <- grep(muster, src, fixed = TRUE)
  stopifnot(length(i) == 1L)
  src[i] <<- neu
}

# Freitext, Liste, Fussnote und native Typst-Tabelle in einer Antwort.
ersetze("#antwort(\"b0141\")", paste(
  "#antwort(\"b0141\")[",
  "Ziel dieser Bewertung ist der Zusatznutzen von Demozil gegenüber Beobachtung#footnote[Demonstration mit dem öffentlichen Datensatz `colon` aus dem R-Paket survival.].",
  "",
  "Betrachtet werden:",
  "",
  "- Gesamtüberleben",
  "- rezidivfreies Überleben",
  "",
  "#neue-tabelle(",
  "  beschriftung: [Übersicht der Endpunkte],",
  "  kopf: ([Endpunkt], [Effektmaß]),",
  "  zeilen: json(\"ergebnisse.json\").map(e => (e.name, [Hazard Ratio])),",
  ")",
  "]", sep = "\n"
))

# Abschnitt 4.3.1.3.1 je Endpunkt wiederholen.
von <- grep("#ueberschrift(5, \"b0381\")", src, fixed = TRUE)
bis <- grep("#ueberschrift(5, \"b0437\")", src, fixed = TRUE) - 1L
abschnitt <- src[von:bis]
abschnitt[grepl("#tabelle(\"b0430\",", abschnitt, fixed = TRUE)] <- paste0(
  "#tabelle(\"b0430\", zeile: 2, spalten: (20, 15, 7, 13, 12, 21, 12), daten: e.zeilen,\n",
  "  ([Studie], [Arm], [N], [Ereignisse], [Median (Jahre)], [HR [95 %-KI]], [p-Wert],),"
)
i <- which(startsWith(abschnitt, "#tabelle(\"b0430\""))
j <- i + which(abschnitt[(i + 1L):length(abschnitt)] == ")")[1]
abschnitt <- abschnitt[-((i + 1L):(j - 1L))]
abschnitt[grepl("#absatz(\"b0433\")", abschnitt, fixed = TRUE)] <-
  "#abbildung(image(\"km_\" + e.slug + \".png\", width: 14cm), beschriftung: [Kaplan-Meier-Kurven für #e.name])"
abschnitt <- abschnitt[!grepl("#beschriftung(\"b0434\")", abschnitt, fixed = TRUE)]
src <- c(
  src[seq_len(von - 1L)],
  "#let endpunkt-abschnitt(e) = mit((\"<Endpunkt xxx>\": e.name), [",
  abschnitt,
  "])",
  "",
  "#for e in json(\"ergebnisse.json\") { endpunkt-abschnitt(e) }",
  "",
  src[(bis + 1L):length(src)]
)

# Tabellenzeilen der Vorlage ueberschreiben und ein Formular-Kaestchen setzen.
src <- sub("^  \\(\\[\\\\<Studie 1\\\\>\\], \\[\\],\\),$", "  ([NCCTG 89-46-51], [Zeit vom Randomisierungsdatum bis zum Ereignis],),", src)
src <- sub("#absatz(\"b0773\")[#kreuz(0, false)", "#absatz(\"b0773\")[#kreuz(0, true)", src, fixed = TRUE)

writeLines(enc2utf8(src), typ, useBytes = TRUE)
out <- file.path(proj, "Modul4A_Demo.docx")
gba_build(typ, out = out)
