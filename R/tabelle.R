# Table format with merged cells, written by R and rendered in the look of an
# existing Word table.
#
#   titel, tag, spalten (relative widths), kopf and zeilen (lists of rows)
#
# A cell is a string or list(text=, spalte=, zeile=, fett=). `spalte` and `zeile`
# are the column and row span. Positions a span covers are left out of later
# rows, as in HTML.

#' One merged or formatted cell for gba_tabelle().
gba_zelle <- function(text, spalte = 1L, zeile = 1L, fett = FALSE) {
  list(text = as.character(text), spalte = as.integer(spalte), zeile = as.integer(zeile), fett = isTRUE(fett))
}

#' A data frame as a table. `kopf` is one row or a list of rows.
gba_tabelle <- function(df, kopf = names(df), titel = NULL, tag = NULL, spalten = NULL) {
  if (!is.list(kopf) || length(kopf) && !is.list(kopf[[1]])) kopf <- list(kopf)
  Filter(Negate(is.null), list(
    titel = titel, tag = tag, spalten = spalten,
    kopf = lapply(kopf, as.list), zeilen = gba_zeilen(df)
  ))
}

check_tag <- function(tag) {
  if (!grepl("^[A-Za-z0-9_-]+$", tag)) stop("Ungültige Kennung: ", tag)
  tag
}

spec_cell <- function(z) {
  if (!is.list(z)) z <- list(text = z)
  txt <- if (is.null(z$text) || !length(z$text) || is.na(z$text)) "" else as.character(z$text)
  list(text = txt, n = as.integer(z$spalte %||% 1L), m = as.integer(z$zeile %||% 1L),
       fett = isTRUE(z$fett), cont = FALSE)
}

#' Places a table's cells on the grid. Covered positions come back as cells
#' with cont = TRUE, because Word needs a cell there.
spec_grid <- function(rows) {
  if (!length(rows)) stop("Die Tabelle hat keine Zeilen.")
  offen <- list()
  breite <- NA_integer_
  out <- vector("list", length(rows))
  for (r in seq_along(rows)) {
    frei <- as.list(rows[[r]])
    cells <- list()
    col <- 1L
    weiter <- list()
    repeat {
      m <- Filter(function(o) o$spalte == col, offen)
      if (length(m)) {
        o <- m[[1]]
        cells[[length(cells) + 1L]] <- list(text = "", n = o$n, m = 1L, fett = FALSE, cont = TRUE, spalte = col)
        if (o$rest > 1L) { o$rest <- o$rest - 1L; weiter[[length(weiter) + 1L]] <- o }
        col <- col + o$n
        next
      }
      if (!length(frei)) break
      z <- spec_cell(frei[[1]])
      frei <- frei[-1L]
      z$spalte <- col
      cells[[length(cells) + 1L]] <- z
      if (z$m > 1L) weiter[[length(weiter) + 1L]] <- list(spalte = col, n = z$n, rest = z$m - 1L)
      col <- col + z$n
    }
    offen <- weiter
    n <- col - 1L
    if (is.na(breite)) breite <- n
    if (n != breite) stop(sprintf("Zeile %d hat %d Spalten, erwartet sind %d.", r, n, breite))
    out[[r]] <- cells
  }
  if (length(offen)) stop("Eine verbundene Zelle reicht über das Tabellenende hinaus.")
  list(rows = out, breite = breite)
}

spec_rows <- function(spec) c(as.list(spec$kopf), as.list(spec$zeilen))

spec_row <- function(row, cells, widths, ctx) {
  n <- length(row$cells)
  xml <- vapply(seq_along(cells), function(j) {
    z <- cells[[j]]
    tc <- row$cells[[min(j, n)]]
    pr <- gsub("<w:gridSpan [^>]*/>", "", strip_vmerge(tc$tcpr))
    add <- paste0(
      sprintf("<w:tcW w:w=\"%d\" w:type=\"dxa\"/>", sum(widths[z$spalte:(z$spalte + z$n - 1L)])),
      if (z$n > 1L) sprintf("<w:gridSpan w:val=\"%d\"/>", z$n),
      if (z$cont) "<w:vMerge/>" else if (z$m > 1L) "<w:vMerge w:val=\"restart\"/>"
    )
    tc$tcpr <- if (grepl("<w:tcW ", pr, fixed = TRUE)) sub("<w:tcW [^>]*/>", add, pr) else sub("<w:tcPr>", paste0("<w:tcPr>", add), pr, fixed = TRUE)
    tc$open <- gsub(" w14:(paraId|textId)=\"[^\"]*\"", "", tc$open, perl = TRUE)
    tc$xml <- ""
    inhalt <- list(func = "text", text = z$text)
    if (z$fett) inhalt <- list(func = "strong", body = inhalt)
    clone_cell_ids(fill_cell(tc, inhalt, ctx, "Tabelle"))
  }, "")
  open <- gsub(" w14:(paraId|textId)=\"[^\"]*\"", "", row$open, perl = TRUE)
  paste0(open, row$pre, paste(xml, collapse = ""), row$post, row$close)
}

#' A table's tag lives in its alt text: Word keeps it on the table, and editing
#' the caption around it cannot remove it.
set_tag <- function(head, tag) {
  head <- gsub("<w:tbl(Caption|Description) [^>]*/>", "", head, perl = TRUE)
  if (is.null(tag)) return(head)
  add <- sprintf("<w:tblDescription w:val=\"gba:%s\"/>", check_tag(tag))
  if (grepl("<w:tblPrChange", head, fixed = TRUE)) return(sub("<w:tblPrChange", paste0(add, "<w:tblPrChange"), head, fixed = TRUE))
  sub("</w:tblPr>", paste0(add, "</w:tblPr>"), head, fixed = TRUE)
}

get_tag <- function(head) first_match("<w:tblDescription w:val=\"gba:([^\"]+)\"", head)

#' XML of a table in the look of `tbl`.
spec_table <- function(spec, tbl, ctx, tag = spec$tag) {
  g <- spec_grid(spec_rows(spec))
  widths <- table_widths(tbl, g$breite, spec$spalten)
  head <- sub("<w:tblGrid>.*</w:tblGrid>", paste0("<w:tblGrid>",
    paste(sprintf("<w:gridCol w:w=\"%d\"/>", widths), collapse = ""), "</w:tblGrid>"), tbl$head)
  nk <- length(spec$kopf)
  kopf <- tbl$rows[[1]]
  body <- tbl$rows[[min(2L, length(tbl$rows))]]
  rows <- vapply(seq_along(g$rows), function(i) {
    spec_row(if (i <= nk) kopf else body, g$rows[[i]], widths, ctx)
  }, "")
  paste0(tbl$open, set_tag(head, tag), paste(rows, collapse = ""), tbl$close)
}
