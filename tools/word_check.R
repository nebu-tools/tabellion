# Compares R's field results with Word's for each module (Windows host via WSL).
# Fills the cover and the first answer, lets Word update the same docx, and
# reports paragraphs that differ apart from page numbers.
#
#   rv run tools/word_check.R [modul ...]
options(gba.root = normalizePath("."), warn = 1)
for (f in list.files("R", "\\.R$", full.names = TRUE)) source(f)

POWERSHELL <- "/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe"

word_update <- function(docx, updated) {
  win <- trimws(tail(system2(POWERSHELL, c("-NoProfile", "-Command", "'$env:TEMP'"), stdout = TRUE), 1L))
  id <- basename(tempfile("gba"))
  dir <- file.path(trimws(system2("wslpath", c("-u", shQuote(win)), stdout = TRUE)), id)
  dir.create(dir)
  on.exit(unlink(dir, recursive = TRUE))
  file.copy(docx, file.path(dir, "in.docx"))
  file.copy("word/finalize.ps1", file.path(dir, "finalize.ps1"))
  wdir <- paste0(win, "\\", id)
  st <- system2(POWERSHELL, c("-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                              shQuote(paste0(wdir, "\\finalize.ps1")), shQuote(wdir)))
  if (st != 0L) stop("Word-Durchlauf fehlgeschlagen")
  file.copy(file.path(dir, "out.docx"), updated, overwrite = TRUE)
}

paragraphs <- function(docx) {
  d <- tempfile()
  utils::unzip(docx, "word/document.xml", exdir = d)
  x <- as_bytes(read_text(file.path(d, "word/document.xml")))
  m <- gregexpr("<w:p[ >].*?</w:p>", x, perl = TRUE, useBytes = TRUE)[[1]]
  p <- vapply(as_utf8(substring(x, m, m + attr(m, "match.length") - 1L)), para_text, "", USE.NAMES = FALSE)
  p <- gsub("­", "", p, fixed = TRUE)
  p <- gsub("[\t ]+", " ", sub("\\s*[0-9ivxlc]+$", "", p))
  p <- trimws(p)
  p[nzchar(p)]
}

check <- function(modul) {
  dir <- tempfile("gba-word-")
  typ <- gba_import(modul, dir)
  s <- read_text(typ)
  s <- sub("    // wirkstoff: \"...\", handelsname: \"...\", unternehmer: \"...\",",
           "    wirkstoff: \"Fluoruracil\", handelsname: \"Demozil\", unternehmer: \"Muster Pharma GmbH\",", s, fixed = TRUE)
  s <- sub("    // kodierung: \"A\", anwendungsgebiet: \"...\", stand: \"01.12.2026\",",
           "    kodierung: \"A\", anwendungsgebiet: \"Kolonkarzinom\", stand: \"02.11.2026\",", s, fixed = TRUE)
  id <- regmatches(s, regexpr("#antwort\\(\"b[0-9]+\"\\)", s))
  if (length(id)) s <- sub(id, paste0(id, "[Testantwort.]"), s, fixed = TRUE)
  write_text(s, typ)
  ours <- file.path(dir, "r.docx")
  capture.output(gba_build(typ, out = ours))
  word <- file.path(dir, "word.docx")
  word_update(ours, word)
  a <- paragraphs(word)
  b <- paragraphs(ours)
  diff <- setdiff(a, b)
  extra <- setdiff(b, a)
  cat(sprintf("%-18s Word %4d, R %4d Absaetze, nur Word %d, nur R %d\n", modul, length(a), length(b), length(diff), length(extra)))
  for (x in head(diff, 6L)) cat("  W:", substr(x, 1L, 130L), "\n")
  for (x in head(extra, 6L)) cat("  R:", substr(x, 1L, 130L), "\n")
}

args <- commandArgs(TRUE)
for (m in if (length(args)) args else manifest()$modul) check(m)
