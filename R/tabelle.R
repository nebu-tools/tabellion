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

spec_row <- function(row, cells, ctx) {
  n <- length(row$cells)
  xml <- vapply(seq_along(cells), function(j) {
    z <- cells[[j]]
    tc <- row$cells[[min(j, n)]]
    tc$xml <- ""
    inhalt <- list(func = "text", text = z$text)
    if (z$fett) inhalt <- list(func = "strong", body = inhalt)
    fill_cell(tc, inhalt, ctx, "Tabelle")
  }, "")
  paste0(row$open, row$pre, paste(xml, collapse = ""), row$post, row$close)
}

#' A table's tag lives in its alt text: Word keeps it on the table, and editing
#' the caption around it cannot remove it.
get_tag <- function(tbl, ns) {
  d <- wattr(xml2::xml_find_first(tbl, "w:tblPr/w:tblDescription", ns), "val", ns)
  if (!is.na(d) && startsWith(d, "gba:")) substring(d, 5L) else NA_character_
}

#' XML of a table in the look of `tbl`, the parsed prototype from the document.
spec_table <- function(spec, tbl, ctx, tag = spec$tag) {
  if (!is.null(tag)) check_tag(tag)
  g <- spec_grid(spec_rows(spec))
  widths <- table_widths(tbl, g$breite, spec$spalten)
  nk <- length(spec$kopf)
  kopf <- tbl$rows[[1]]
  body <- tbl$rows[[min(2L, length(tbl$rows))]]
  rows <- vapply(seq_along(g$rows), function(i) spec_row(if (i <= nk) kopf else body, g$rows[[i]], ctx), "")
  rt <- ctx$tpl$root_tag
  ns <- root_ns(rt)
  t <- xml_node1(paste0(tbl$open, tbl$head, paste(rows, collapse = ""), tbl$close), rt)
  grid <- xml2::xml_find_first(t, "w:tblGrid", ns)
  rm_nodes(xml2::xml_children(grid))
  for (w in widths) set_wattrs(add_el(grid, "w:gridCol", ns), list(w = w), ns)
  pr <- xml2::xml_find_first(t, "w:tblPr", ns)
  rm_nodes(xfind(pr, "w:tblCaption | w:tblDescription", ns))
  if (!is.null(tag)) set_wattrs(pr_child(pr, "tblDescription", ns, TBLPR_ORDER), list(val = paste0("gba:", tag)), ns)
  # The prototype may carry pending revisions, ids and bookmarks of its own.
  rm_nodes(xfind(t, ".//w:trPr/w:ins | .//w:trPr/w:del | .//w:pPr/w:rPr/w:ins | .//w:pPr/w:rPr/w:del | .//w:bookmarkStart | .//w:bookmarkEnd", ns))
  if ("w14" %in% names(ns)) {
    for (k in xfind(t, "descendant-or-self::*[@w14:paraId or @w14:textId]", ns)) {
      xml2::xml_set_attr(k, "w14:paraId", NULL, ns = ns)
      xml2::xml_set_attr(k, "w14:textId", NULL, ns = ns)
    }
  }
  trs <- xfind(t, "w:tr", ns)
  for (i in seq_along(trs)) {
    tcs <- xfind(trs[[i]], "w:tc", ns)
    for (j in seq_along(tcs)) {
      z <- g$rows[[i]][[j]]
      tcpr <- xml2::xml_find_first(tcs[[j]], "w:tcPr", ns)
      if (inherits(tcpr, "xml_missing")) tcpr <- add_el(tcs[[j]], "w:tcPr", ns, first = TRUE)
      rm_nodes(xfind(tcpr, "w:tcW | w:gridSpan | w:vMerge", ns))
      set_wattrs(pr_child(tcpr, "tcW", ns, TCPR_ORDER), list(w = sum(widths[z$spalte:(z$spalte + z$n - 1L)]), type = "dxa"), ns)
      if (z$n > 1L) set_wattrs(pr_child(tcpr, "gridSpan", ns, TCPR_ORDER), list(val = z$n), ns)
      if (z$cont) pr_child(tcpr, "vMerge", ns, TCPR_ORDER)
      else if (z$m > 1L) set_wattrs(pr_child(tcpr, "vMerge", ns, TCPR_ORDER), list(val = "restart"), ns)
    }
  }
  node_str(t, rt)
}
