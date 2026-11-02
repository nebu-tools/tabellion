# Edits an existing docx in place. The docx stays the master: every operation
# touches only its target and writes tracked changes for the team to review.

REV_RE <- "<w:(?:ins|del|moveFrom|moveTo|rPrChange|pPrChange|tcPrChange|trPrChange|tblPrChange) [^>]*w:author=\"([^\"]*)\""

rev_authors <- function(xml) {
  hits <- regmatches(xml, gregexpr(REV_RE, xml, perl = TRUE))[[1]]
  unique(xml_unescape(sub(".*w:author=\"([^\"]*)\"$", "\\1", hits)))
}

rev_attrs <- function(ctx) {
  sprintf("w:id=\"%d\" w:author=\"%s\" w:date=\"%s\"", ctx$next_id(), ctx$autor_xml, ctx$datum)
}

map_matches <- function(xml, re, f) {
  m <- gregexpr(re, xml, perl = TRUE)
  if (m[[1]][1] < 0) return(xml)
  regmatches(xml, m) <- list(vapply(regmatches(xml, m)[[1]], f, "", USE.NAMES = FALSE))
  xml
}

RUN_RE <- "<w:r(?: [^>]*)?>.*?</w:r>"
PARA_RE <- "<w:p(?: [^>]*)?>.*?</w:p>"
TR_RE <- "<w:tr(?: [^>]*)?>.*?</w:tr>"

# An empty paragraph written as <w:p/> would otherwise match up to the next </w:p>.
open_paras <- function(xml) gsub("<w:p( [^>]*?)?/>", "<w:p\\1></w:p>", xml, perl = TRUE)

para_mark <- function(p, tag, ctx) {
  mark <- sprintf("<w:%s %s/>", tag, rev_attrs(ctx))
  if (grepl("^<w:p(?: [^>]*)?><w:pPr>", p, perl = TRUE)) {
    ppr <- first_match("^<w:p(?: [^>]*)?>(<w:pPr>.*?</w:pPr>)", p)
    neu <- if (grepl("<w:rPr>", ppr, fixed = TRUE)) sub("<w:rPr>", paste0("<w:rPr>", mark), ppr, fixed = TRUE)
           else sub("</w:pPr>", paste0("<w:rPr>", mark, "</w:rPr></w:pPr>"), ppr, fixed = TRUE)
    return(sub(ppr, neu, p, fixed = TRUE))
  }
  sub("^(<w:p(?: [^>]*)?>)", paste0("\\1<w:pPr><w:rPr>", gsub("\\", "\\\\", mark, fixed = TRUE), "</w:rPr></w:pPr>"), p, perl = TRUE)
}

row_mark <- function(tr, tag, ctx) {
  mark <- sprintf("<w:%s %s/>", tag, rev_attrs(ctx))
  if (grepl("</w:trPr>", tr, fixed = TRUE)) return(sub("</w:trPr>", paste0(mark, "</w:trPr>"), tr, fixed = TRUE))
  sub("^(<w:tr(?: [^>]*)?>)", paste0("\\1<w:trPr>", mark, "</w:trPr>"), tr, perl = TRUE)
}

as_deleted <- function(run) {
  run <- gsub("<w:t( [^>]*)?>", "<w:delText\\1>", run, perl = TRUE)
  run <- gsub("</w:t>", "</w:delText>", run, fixed = TRUE)
  run <- gsub("<w:instrText( [^>]*)?>", "<w:delInstrText\\1>", run, perl = TRUE)
  gsub("</w:instrText>", "</w:delInstrText>", run, fixed = TRUE)
}

as_live <- function(x) {
  x <- gsub("<w:delText( [^>]*)?>", "<w:t\\1>", x, perl = TRUE)
  x <- gsub("</w:delText>", "</w:t>", x, fixed = TRUE)
  x <- gsub("<w:delInstrText( [^>]*)?>", "<w:instrText\\1>", x, perl = TRUE)
  gsub("</w:delInstrText>", "</w:instrText>", x, fixed = TRUE)
}

#' Marks new paragraphs and tables as a tracked insertion.
track_ins <- function(xml, ctx) {
  xml <- open_paras(xml)
  xml <- map_matches(xml, RUN_RE, function(r) sprintf("<w:ins %s>%s</w:ins>", rev_attrs(ctx), r))
  xml <- map_matches(xml, PARA_RE, function(p) para_mark(p, "ins", ctx))
  map_matches(xml, TR_RE, function(tr) row_mark(tr, "ins", ctx))
}

#' Marks existing paragraphs and tables as a tracked deletion.
track_del <- function(xml, ctx) {
  xml <- open_paras(xml)
  xml <- map_matches(xml, RUN_RE, function(r) sprintf("<w:del %s>%s</w:del>", rev_attrs(ctx), as_deleted(r)))
  xml <- map_matches(xml, PARA_RE, function(p) para_mark(p, "del", ctx))
  map_matches(xml, TR_RE, function(tr) row_mark(tr, "del", ctx))
}

#' Takes back this author's revisions in cell content.
revert_own <- function(xml, ctx) {
  a <- sprintf("w:author=\"%s\"", ctx$autor_re)
  xml <- gsub(sprintf("<w:ins [^>]*%s[^>/]*>.*?</w:ins>", a), "", xml, perl = TRUE)
  xml <- map_matches(xml, sprintf("<w:del [^>]*%s[^>/]*>.*?</w:del>", a), function(d) {
    as_live(sub("</w:del>$", "", sub("^<w:del [^>]*>", "", d, perl = TRUE), perl = TRUE))
  })
  xml
}

cell_paras <- function(xml) {
  xml <- open_paras(xml)
  list(xml = xml, paras = regmatches(xml, gregexpr(PARA_RE, xml, perl = TRUE))[[1]])
}

clean_text <- function(x) trimws(gsub("[ \t\r\n ]+", " ", x, perl = TRUE))

cell_text <- function(xml) {
  ps <- cell_paras(xml)$paras
  clean_text(paste(vapply(ps, para_text, "", USE.NAMES = FALSE), collapse = " "))
}

COMPLEX_RE <- paste0("<w:(fldChar|fldSimple|footnoteReference|endnoteReference|drawing|pict|object|",
                     "hyperlink|sdt|commentReference|commentRangeStart|tbl)[ />]")

#' New value for one cell. Only plain text is replaced, so a field, a citation
#' or a footnote is never lost to a table update.
cell_update <- function(tc, neu, ctx) {
  neu <- clean_text(neu)
  xml <- tc$xml
  if (identical(cell_text(xml), neu)) return(list(status = "gleich", xml = xml))
  fremd <- setdiff(rev_authors(xml), ctx$autor)
  if (length(fremd)) return(list(status = "offen", grund = paste("offene Änderung von", paste(fremd, collapse = ", ")), xml = xml))
  xml <- revert_own(xml, ctx)
  if (grepl(COMPLEX_RE, xml, perl = TRUE)) return(list(status = "offen", grund = "Feld, Fußnote oder Zitat in der Zelle", xml = tc$xml))
  cp <- cell_paras(xml)
  voll <- which(nzchar(trimws(vapply(cp$paras, para_text, "", USE.NAMES = FALSE))))
  if (length(voll) > 1L) return(list(status = "offen", grund = "mehrere Absätze in der Zelle", xml = tc$xml))
  ziel <- cp$paras[[if (length(voll)) voll else 1L]]
  runs <- regmatches(ziel, gregexpr(RUN_RE, ziel, perl = TRUE))[[1]]
  rpr <- if (length(runs)) first_match("^<w:r(?: [^>]*)?>(<w:rPr>.*?</w:rPr>)", runs[1]) else NA_character_
  rpr <- if (is.na(rpr)) "" else rpr
  add <- paste0(
    if (length(runs)) sprintf("<w:del %s>%s</w:del>", rev_attrs(ctx), paste(as_deleted(runs), collapse = "")),
    if (nzchar(neu)) sprintf("<w:ins %s>%s</w:ins>", rev_attrs(ctx), token_run(neu, rpr))
  )
  # Runs left in place by revert_own come first, so all of them go.
  p <- gsub(RUN_RE, "", ziel, perl = TRUE)
  p <- sub("</w:p>$", "", p, perl = TRUE)
  list(status = "geaendert", xml = sub(ziel, paste0(p, add, "</w:p>"), cp$xml, fixed = TRUE))
}

#' Non-continuation cells of a Word table on the grid.
table_shape <- function(tbl) {
  lapply(tbl$rows, function(r) {
    col <- 1L + as.integer(first_match("<w:gridBefore w:val=\"([0-9]+)\"", r$xml) %|na|% "0")
    out <- list()
    for (i in seq_along(r$cells)) {
      pr <- r$cells[[i]]$tcpr
      n <- as.integer(first_match("<w:gridSpan w:val=\"([0-9]+)\"", pr) %|na|% "1")
      cont <- grepl("<w:vMerge/>|<w:vMerge w:val=\"continue\"/>", pr)
      if (!cont) out[[length(out) + 1L]] <- list(i = i, spalte = col, n = n)
      col <- col + n
    }
    out
  })
}

`%|na|%` <- function(a, b) if (is.na(a)) b else a

form <- function(rows) {
  vapply(rows, function(r) paste(vapply(r, function(c) sprintf("%d:%d", c$spalte, c$n), ""), collapse = " "), "")
}

#' Replaces cells by position, so identical cells cannot be mixed up.
set_cells <- function(tbl, neu) {
  sl <- xml_slice(tbl$xml)
  tr <- which(elem_name(sl$children) == "w:tr")
  for (r in seq_along(tr)) {
    zellen <- neu[[as.character(r)]]
    if (is.null(zellen)) next
    rs <- xml_slice(sl$children[tr[r]])
    tc <- which(elem_name(rs$children) == "w:tc")
    for (k in names(zellen)) rs$children[tc[as.integer(k)]] <- zellen[[k]]
    sl$children[tr[r]] <- paste0(rs$open, paste(rs$children, collapse = ""), rs$close)
  }
  paste0(sl$open, paste(sl$children, collapse = ""), sl$close)
}

op_werte <- function(b, spec, ctx, name) {
  ist <- table_shape(b$table)
  soll <- lapply(spec_grid(spec_rows(spec))$rows, function(r) Filter(function(z) !z$cont, r))
  if (!identical(form(ist), form(soll))) {
    stop(sprintf("%s: Tabellenform weicht ab (%d statt %d Zeilen oder andere Spalten). Tabelle mit `ersetzen` austauschen.",
                 name, length(soll), length(ist)))
  }
  neu <- list()
  geaendert <- 0L
  offen <- list()
  for (r in seq_along(ist)) {
    for (j in seq_along(ist[[r]])) {
      i <- ist[[r]][[j]]$i
      u <- cell_update(b$table$rows[[r]]$cells[[i]], soll[[r]][[j]]$text, ctx)
      if (u$status == "geaendert") {
        geaendert <- geaendert + 1L
        neu[[as.character(r)]][[as.character(i)]] <- u$xml
      }
      if (u$status == "offen") {
        offen[[length(offen) + 1L]] <- list(tabelle = name, zeile = r, spalte = ist[[r]][[j]]$spalte, grund = u$grund)
      }
    }
  }
  list(xml = if (geaendert) set_cells(b$table, neu) else b$xml, geaendert = geaendert, offen = offen)
}

op_ersetzen <- function(b, spec, ctx, name) {
  fremd <- setdiff(rev_authors(b$xml), ctx$autor)
  if (length(fremd)) stop(sprintf("%s: offene Änderungen von %s. Erst in Word annehmen oder ablehnen.", name, paste(fremd, collapse = ", ")))
  tag <- spec$tag %||% (get_tag(b$table$head) %|na|% NULL)
  neu <- track_ins(spec_table(spec, b$table, ctx, tag = tag), ctx)
  zeilen <- regmatches(b$xml, gregexpr(TR_RE, b$xml, perl = TRUE))[[1]]
  eigen <- vapply(zeilen, function(z) grepl(sprintf("<w:ins [^>]*w:author=\"%s\"[^>]*/></w:trPr>", ctx$autor_re), z, perl = TRUE), NA)
  # A table from an earlier round the team has not accepted yet just goes away.
  if (all(eigen)) return(neu)
  # The old table keeps its tag, so rejecting the change restores a usable target.
  alt <- track_del(revert_own(b$xml, ctx), ctx)
  trenner <- track_ins(paste0("<w:p>", style_ppr("TextkrperDossier"), "</w:p>"), ctx)
  paste0(alt, trenner, neu)
}

op_einfuegen <- function(b, spec, ctx, titel) {
  tbl <- block_of(ctx, default_table(ctx))$table
  cap <- if (!is.null(titel)) caption_xml(titel, "Tabelle", ctx) else ""
  neu <- paste0(cap, spec_table(spec, tbl, ctx), "<w:p>", style_ppr("TextkrperDossier"), "</w:p>")
  paste0(track_del(b$xml, ctx), track_ins(neu, ctx))
}

all_placeholders <- function(tpl) {
  out <- character()
  for (id in tpl$ids) {
    b <- tpl$blocks[[id]]
    if (b$type != "p") next
    x <- para_text(b$xml)
    t <- regmatches(x, gregexpr("\\{\\{[^{}]+\\}\\}", x))[[1]]
    if (length(t)) out <- c(out, stats::setNames(gsub("[{} ]", "", t), rep(id, length(t))))
  }
  out
}

table_title <- function(tpl, id) {
  i <- match(id, tpl$ids)
  if (i < 2L) return(NA_character_)
  p <- tpl$blocks[[tpl$ids[i - 1L]]]
  if (p$type != "p" || !grepl("Beschriftung", p$style)) return(NA_character_)
  trimws(para_text(p$xml))
}

#' What a user can pick in a document: headings, tables, and placeholders.
gba_outline <- function(docx) {
  tpl <- read_template(docx)
  on.exit(unlink(tpl$dir, recursive = TRUE))
  kinds <- vapply(tpl$blocks, block_kind, "")
  heads <- lapply(tpl$ids[kinds %in% c("heading", "heading-frei")], function(id) {
    b <- tpl$blocks[[id]]
    list(block = id, stil = b$style, text = trimws(para_text(b$xml)))
  })
  tabs <- lapply(tpl$ids[kinds %in% c("tabelle", "fest")], function(id) {
    b <- tpl$blocks[[id]]
    if (b$type != "tbl" || table_removed(b)) return(NULL)
    list(block = id, tag = get_tag(b$table$head) %|na|% NULL, titel = table_title(tpl, id) %|na|% NULL,
         zeilen = length(b$table$rows), geschuetzt = b$table$fest, offen_von = rev_authors(b$xml))
  })
  ph <- all_placeholders(tpl)
  list(sha256 = unname(tools::sha256sum(docx)), ueberschriften = heads, tabellen = Filter(Negate(is.null), tabs),
       platzhalter = lapply(seq_along(ph), function(i) list(block = names(ph)[i], name = ph[[i]])))
}

#' A table waiting for its tracked deletion to be accepted.
table_removed <- function(b) {
  all(vapply(b$table$rows, function(r) grepl("<w:del [^>]*/>", r$pre, perl = TRUE), NA))
}

find_target <- function(ziel, tpl, sha) {
  if (!is.null(ziel$tag)) {
    ids <- Filter(function(id) {
      b <- tpl$blocks[[id]]
      b$type == "tbl" && identical(get_tag(b$table$head), ziel$tag) && !table_removed(b)
    }, tpl$ids)
    if (!length(ids)) stop("Keine Tabelle mit der Kennung ", ziel$tag, ". In der Web-App auswählen.")
    if (length(ids) > 1L) stop("Kennung ", ziel$tag, " kommt mehrfach vor: ", paste(ids, collapse = ", "))
    return(ids)
  }
  if (is.null(ziel$block)) stop("Das Ziel braucht `tag` oder `block`.")
  # Block ids are positions and hold only for the file they were read from.
  if (is.null(sha) || !identical(sha, unname(tools::sha256sum(tpl$path)))) {
    stop("`block` gilt nur zusammen mit der sha256 der hochgeladenen Datei aus gba_outline().")
  }
  b <- tpl$blocks[[ziel$block]]
  if (is.null(b) || b$type != "tbl") stop("Block ", ziel$block, " ist keine Tabelle.")
  if (b$table$fest) stop("Block ", ziel$block, " enthält geschützte Inhalte.")
  ziel$block
}

find_placeholder <- function(name, tpl) {
  ph <- all_placeholders(tpl)
  ids <- names(ph)[ph == name]
  if (!length(ids)) stop("Platzhalter {{", name, "}} nicht gefunden.")
  if (length(ids) > 1L) stop("Platzhalter {{", name, "}} kommt mehrfach vor.")
  b <- tpl$blocks[[ids]]
  if (!identical(clean_text(para_text(b$xml)), paste0("{{", name, "}}"))) {
    stop("Platzhalter {{", name, "}} muss allein im Absatz stehen.")
  }
  ids
}

#' Applies operations to a docx and writes the result.
#'
#' `ops` is a list or the path of a JSON file with `aenderungen` and, for targets
#' by block id, the `sha256` from gba_outline(). Each change is one of
#'   list(op = "werte", ziel = list(tag = "x"), tabelle = spec)
#'   list(op = "ersetzen", ziel = list(block = "b0123"), tabelle = spec)
#'   list(op = "einfuegen", platzhalter = "tab:x", tabelle = spec)
#'   list(op = "felder")
#' Any problem stops the run and writes nothing. Cells it must leave alone are
#' listed under `offen` in the returned report.
gba_apply <- function(docx, ops, out, autor = "Automatisierung") {
  if (is.character(ops)) ops <- jsonlite::fromJSON(read_text(ops), simplifyVector = FALSE)
  tpl <- read_template(docx)
  on.exit(unlink(tpl$dir, recursive = TRUE))
  ctx <- new_ctx(tpl, dirname(normalizePath(docx)))
  ctx$autor <- autor
  ctx$autor_xml <- xml_escape(autor)
  ctx$autor_re <- gsub("([.^$|()*+?{}\\[\\]\\\\])", "\\\\\\1", ctx$autor_xml, perl = TRUE)
  ctx$datum <- format(Sys.time(), "%Y-%m-%dT%H:%M:%SZ", tz = "UTC")
  xml <- vapply(tpl$blocks, `[[`, "", "xml")
  erledigt <- character()
  bericht <- list()
  offen <- list()
  felder <- FALSE
  for (o in ops$aenderungen) {
    if (identical(o$op, "felder")) { felder <- TRUE; next }
    if (!o$op %in% c("werte", "ersetzen", "einfuegen")) stop("Unbekannte Änderung: ", o$op)
    id <- if (o$op == "einfuegen") find_placeholder(o$platzhalter, tpl) else find_target(o$ziel, tpl, ops$sha256)
    if (id %in% erledigt) stop("Block ", id, " ist mehrfach Ziel einer Änderung.")
    erledigt <- c(erledigt, id)
    b <- tpl$blocks[[id]]
    name <- o$ziel$tag %||% o$ziel$block %||% o$platzhalter
    spec <- o$tabelle
    eintrag <- list(op = o$op, ziel = name, block = id)
    if (o$op == "werte") {
      r <- op_werte(b, spec, ctx, name)
      xml[[id]] <- r$xml
      offen <- c(offen, r$offen)
      eintrag$zellen <- r$geaendert
    } else if (o$op == "ersetzen") {
      xml[[id]] <- op_ersetzen(b, spec, ctx, name)
    } else {
      xml[[id]] <- op_einfuegen(b, spec, ctx, spec$titel)
    }
    bericht[[length(bericht) + 1L]] <- eintrag
  }
  trails <- vapply(tpl$blocks, `[[`, "", "trail")
  body <- paste0(tpl$lead, paste0(xml, trails, collapse = ""))
  orig <- paste0(tpl$lead, paste0(vapply(tpl$blocks, `[[`, "", "xml"), trails, collapse = ""))
  if (!felder && identical(body, orig)) {
    file.copy(docx, out, overwrite = TRUE)
  } else {
    body <- resolve_fields(body, ctx)
    write_package(tpl, ctx, paste0(tpl$head, body, tpl$final_sect, tpl$tail), list(), out)
  }
  list(datei = out, aenderungen = bericht, offen = offen,
       platzhalter_offen = lapply(setdiff(all_placeholders(tpl), vapply(Filter(function(o) identical(o$op, "einfuegen"), ops$aenderungen), `[[`, "", "platzhalter")), identity),
       hinweis = "Seitenzahlen aktualisiert Word.")
}
