# G-BA templates are not part of this repository. templates.csv lists their
# download URLs and checksums; gba_fetch() stores them in templates/.

MANIFEST <- "templates.csv"

gba_root <- function() getOption("gba.root", ".")

manifest <- function() {
  utils::read.csv(file.path(gba_root(), MANIFEST), stringsAsFactors = FALSE)
}

template_path <- function(row) {
  file.path(gba_root(), "templates", basename(row$url))
}

gba_download <- function(url, dest) {
  st <- system2("curl", c("-sSfL", "-A", shQuote("Mozilla/5.0"), "-o", shQuote(dest), shQuote(url)))
  if (st != 0L) stop("Download fehlgeschlagen: ", url)
}

#' Downloads missing templates and verifies all checksums.
gba_fetch <- function() {
  m <- manifest()
  dir.create(file.path(gba_root(), "templates"), showWarnings = FALSE)
  for (i in seq_len(nrow(m))) {
    p <- template_path(m[i, ])
    if (!file.exists(p)) gba_download(m$url[i], p)
    if (!identical(unname(tools::sha256sum(p)), m$sha256[i])) {
      stop("Prüfsumme stimmt nicht: ", basename(p), ". Der G-BA hat die Datei geändert.")
    }
  }
  invisible(vapply(seq_len(nrow(m)), function(i) template_path(m[i, ]), ""))
}

#' Fails if the G-BA page no longer links a template from the manifest.
gba_check_templates <- function() {
  url <- paste0("https://www.g-ba.de/themen/arzneimittel/arzneimittel-richtlinie-anlagen/",
                "nutzenbewertung-35a/informationen-fuer-unternehmen/formulare-und-vorgaben/")
  page <- tempfile(fileext = ".html")
  gba_download(url, page)
  html <- read_text(page)
  m <- manifest()
  links <- sub("^https://www.g-ba.de", "", m$url)
  gone <- m$modul[!vapply(links, grepl, NA, x = html, fixed = TRUE)]
  if (length(gone)) stop("Nicht mehr auf der G-BA-Seite verlinkt: ", paste(gone, collapse = ", "))
  cat(sprintf("%d Vorlagen sind auf der G-BA-Seite aktuell.\n", nrow(m)))
}

gba_template <- function(modul) {
  m <- manifest()
  i <- match(modul, m$modul)
  if (is.na(i)) stop("Unbekanntes Modul: ", modul, ". Bekannt: ", paste(m$modul, collapse = ", "))
  p <- template_path(m[i, ])
  if (!file.exists(p)) stop("Vorlage fehlt. Führe zuerst `rv run gba.R fetch` aus.")
  p
}
