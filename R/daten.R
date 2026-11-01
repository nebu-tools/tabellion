# Helpers to hand R results to Typst.

#' German decimal formatting.
gba_zahl <- function(x, stellen = 2) {
  out <- formatC(x, format = "f", digits = stellen, decimal.mark = ",")
  out[is.na(x)] <- "n. b."
  out
}

#' Effect with confidence interval, e.g. "0,82 [0,71; 0,95]".
gba_ki <- function(est, lo, hi, stellen = 2) {
  sprintf("%s [%s; %s]", gba_zahl(est, stellen), gba_zahl(lo, stellen), gba_zahl(hi, stellen))
}

#' p-values in dossier convention.
gba_p <- function(p) {
  ifelse(p < 0.001, "<0,001", gba_zahl(p, 3))
}

#' A data frame as table rows (one array per row, all cells strings).
gba_zeilen <- function(df) {
  m <- as.matrix(data.frame(lapply(df, as.character), check.names = FALSE, stringsAsFactors = FALSE))
  lapply(seq_len(nrow(m)), function(i) unname(as.list(m[i, ])))
}

#' Writes results as JSON for `json("...")` in Typst.
gba_json <- function(x, path) {
  dir.create(dirname(path), recursive = TRUE, showWarnings = FALSE)
  jsonlite::write_json(x, path, auto_unbox = TRUE, pretty = TRUE)
  invisible(path)
}
