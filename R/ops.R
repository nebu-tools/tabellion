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
  # Formula runs are not w:r, so the tracked change wraps the whole formula.
  xml <- map_matches(xml, "<m:oMathPara>.*?</m:oMathPara>|<m:oMath>.*?</m:oMath>", function(m) {
    sprintf("<w:ins %s>%s</w:ins>", rev_attrs(ctx), m)
  })
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

# What may go away with a target. A citation is only removed on request, and a
# paragraph that closes a section or sits inside a field must stay whole.
check_loss <- function(xml, o, name) {
  if (grepl("EN\\.(CITE|REFLIST)", xml, perl = TRUE) && !isTRUE(o$zitate_loeschen)) {
    stop(name, " enthält EndNote-Zitate. Sie gingen verloren. Mit `zitate_loeschen: true` bestätigen oder Text in Word ändern.")
  }
}

check_fremd <- function(xml, ctx, name) {
  fremd <- setdiff(rev_authors(xml), ctx$autor)
  if (length(fremd)) stop(sprintf("%s: offene Änderungen von %s. Erst in Word annehmen oder ablehnen.", name, paste(fremd, collapse = ", ")))
}

zaehle_re <- function(x, re) lengths(regmatches(x, gregexpr(re, x, fixed = TRUE)))

check_paras <- function(ids, ctx, name) {
  tpl <- ctx$tpl
  xml <- vapply(tpl$ids, function(i) tpl$blocks[[i]]$xml, "")
  tiefe <- cumsum(zaehle_re(xml, "w:fldCharType=\"begin\"") - zaehle_re(xml, "w:fldCharType=\"end\""))
  von <- match(ids[1], tpl$ids)
  vor <- if (von > 1L) tiefe[von - 1L] else 0L
  if (vor != 0L || tiefe[match(ids[length(ids)], tpl$ids)] != 0L) stop(name, " liegt in einem Feld, z. B. einem Literaturverzeichnis.")
  x <- paste(xml[ids], collapse = "")
  if (grepl("<w:sectPr", x, fixed = TRUE)) stop(name, " enthält einen Abschnittswechsel.")
  # An own pending paragraph goes away as a whole, so its formula is not at risk.
  fremd <- xml[ids][!vapply(xml[ids], para_own, NA, ctx = ctx)]
  if (any(grepl("<m:oMath", fremd, fixed = TRUE))) stop(name, " enthält eine Formel. In Word ändern.")
}

text_ppr <- function(ctx) if ("TextkrperDossier" %in% ctx$tpl$styles) style_ppr("TextkrperDossier") else ""

neuer_text <- function(s, ctx, tag = s$tag) {
  if (is.null(s$baustein)) stop("`text` braucht `baustein`.")
  render_text(list(body = baustein_eval(s$baustein, s$werte), tag = tag), list(ppr = text_ppr(ctx), tokens = list()), ctx)
}

neue_abbildung <- function(s, ctx, tag = s$tag) {
  if (is.null(s$datei)) stop("`abbildung` braucht `datei`.")
  if (!is.null(s$breite) && is.na(parse_length(s$breite))) stop("Ungültige Breite: ", s$breite)
  pfad <- if (startsWith(s$datei, "/")) s$datei else file.path(ctx$basis, s$datei)
  ctx$dir <- dirname(pfad)
  render_figure(list(bild = list(source = basename(pfad), width = s$breite), tag = tag), ctx)
}

op_ersetzen_tabelle <- function(id, spec, ctx, name) {
  b <- ctx$tpl$blocks[[id]]
  check_fremd(b$xml, ctx, name)
  tag <- spec$tag %||% (get_tag(b$table$head) %|na|% NULL)
  neu <- track_ins(spec_table(spec, b$table, ctx, tag = tag), ctx)
  zeilen <- regmatches(b$xml, gregexpr(TR_RE, b$xml, perl = TRUE))[[1]]
  eigen <- vapply(zeilen, function(z) grepl(sprintf("<w:ins [^>]*w:author=\"%s\"[^>]*/></w:trPr>", ctx$autor_re), z, perl = TRUE), NA)
  # A table from an earlier round the team has not accepted yet just goes away.
  if (all(eigen)) return(stats::setNames(list(neu), id))
  # The old table keeps its tag, so rejecting the change restores a usable target.
  alt <- track_del(revert_own(b$xml, ctx), ctx)
  trenner <- track_ins(paste0("<w:p>", text_ppr(ctx), "</w:p>"), ctx)
  stats::setNames(list(paste0(alt, trenner, neu)), id)
}

para_own <- function(x, ctx) {
  grepl(sprintf("^<w:p(?: [^>]*)?><w:pPr>(?:(?!</w:pPr>).)*<w:rPr>(?:(?!</w:rPr>).)*<w:ins [^>]*w:author=\"%s\"", ctx$autor_re), x, perl = TRUE)
}

# The new content goes behind the old paragraphs, which stay as a deletion.
ersetze_paras <- function(ids, neu, ctx) {
  alt <- vapply(ids, function(id) {
    x <- open_paras(ctx$tpl$blocks[[id]]$xml)
    if (para_own(x, ctx)) "" else track_del(revert_own(x, ctx), ctx)
  }, "")
  n <- length(alt)
  alt[n] <- paste0(alt[n], neu)
  as.list(alt)
}

op_ersetzen_text <- function(ids, s, ctx, name) {
  xml <- paste(vapply(ids, function(id) ctx$tpl$blocks[[id]]$xml, ""), collapse = "")
  check_fremd(xml, ctx, name)
  check_paras(ids, ctx, name)
  tag <- s$tag %||% text_tags(xml)[1]
  ersetze_paras(ids, track_ins(neuer_text(s, ctx, if (is.na(tag %||% NA)) NULL else tag), ctx), ctx)
}

op_ersetzen_abbildung <- function(id, s, ctx, name) {
  x <- ctx$tpl$blocks[[id]]$xml
  check_fremd(x, ctx, name)
  check_paras(id, ctx, name)
  if (zaehle_re(x, "<pic:pic") != 1L || grepl("txbxContent", x, fixed = TRUE) || nzchar(trimws(para_text(x)))) {
    stop(name, ": der Absatz enthält mehr als eine Abbildung. In Word ändern.")
  }
  tag <- s$tag %||% (fig_tag(x) %|na|% NULL)
  ersetze_paras(id, track_ins(neue_abbildung(s, ctx, tag), ctx), ctx)
}

op_einfuegen <- function(id, o, ctx, art) {
  b <- ctx$tpl$blocks[[id]]
  neu <- switch(art,
    tabelle = paste0(
      if (!is.null(o$tabelle$titel)) caption_xml(o$tabelle$titel, "Tabelle", ctx),
      spec_table(o$tabelle, block_of(ctx, default_table(ctx))$table, ctx), "<w:p>", text_ppr(ctx), "</w:p>"),
    abbildung = paste0(neue_abbildung(o$abbildung, ctx),
                       if (!is.null(o$abbildung$titel)) caption_xml(o$abbildung$titel, "Abbildung", ctx)),
    text = neuer_text(o$text, ctx))
  paste0(track_del(b$xml, ctx), track_ins(neu, ctx))
}

# ---- captions ----------------------------------------------------------------

#' Block of a table's or figure's caption. A tracked replacement leaves the old
#' table and an empty paragraph between a table and its caption.
caption_block <- function(tpl, id) {
  b <- tpl$blocks[[id]]
  i <- match(id, tpl$ids)
  if (b$type == "p") {
    n <- if (i < length(tpl$ids)) tpl$blocks[[tpl$ids[i + 1L]]]
    return(if (!is.null(n) && n$type == "p" && grepl("Beschriftung", n$style)) tpl$ids[i + 1L] else NA_character_)
  }
  while ((i <- i - 1L) >= 1L) {
    p <- tpl$blocks[[tpl$ids[i]]]
    if (p$type == "tbl" && table_removed(p)) next
    if (p$type != "p") return(NA_character_)
    if (grepl("Beschriftung", p$style)) return(tpl$ids[i])
    if (nzchar(trimws(para_text(p$xml)))) return(NA_character_)
  }
  NA_character_
}

caption_title <- function(xml) sub("^[^:]*:\\s*", "", clean_text(para_text(xml)))

#' Tracked new title. The number field stays, so only the text behind it changes.
op_titel <- function(id, titel, art, ctx, name) {
  b <- ctx$tpl$blocks[[id]]
  if (identical(caption_title(b$xml), clean_text(titel))) return(list())
  check_fremd(b$xml, ctx, name)
  if (para_own(b$xml, ctx)) {
    return(stats::setNames(list(track_ins(caption_xml(titel, if (art == "tabelle") "Tabelle" else "Abbildung", ctx), ctx)), id))
  }
  x <- revert_own(open_paras(b$xml), ctx)
  seq_at <- regexpr("SEQ ", x, fixed = TRUE)
  if (seq_at < 0) stop(name, ": Beschriftung ohne Nummernfeld. In Word ändern.")
  rest <- substr(x, seq_at, nchar(x))
  ende <- regexpr("w:fldCharType=\"end\"[^>]*/></w:r>|</w:fldSimple>", rest, perl = TRUE)
  if (ende < 0) stop(name, ": Beschriftung ohne Nummernfeld. In Word ändern.")
  grenze <- seq_at + ende + attr(ende, "match.length") - 2L
  kopf <- substr(x, 1L, grenze)
  schwanz <- sub("</w:p>$", "", substr(x, grenze + 1L, nchar(x)), perl = TRUE)
  if (grepl(COMPLEX_RE, schwanz, perl = TRUE)) stop(name, ": Feld oder Zitat im Titel. In Word ändern.")
  runs <- regmatches(schwanz, gregexpr(RUN_RE, schwanz, perl = TRUE))[[1]]
  rpr <- if (length(runs)) first_match("^<w:r(?: [^>]*)?>(<w:rPr>.*?</w:rPr>)", runs[1]) %|na|% "" else ""
  trenner <- first_match("^(\\s*[:.]\\s*)", clean_text(para_text(paste0("<w:p>", schwanz, "</w:p>")))) %|na|% ": "
  neu <- paste0(
    if (length(runs)) sprintf("<w:del %s>%s</w:del>", rev_attrs(ctx), paste(as_deleted(runs), collapse = "")),
    sprintf("<w:ins %s>%s</w:ins>", rev_attrs(ctx), token_run(paste0(trenner, clean_text(titel)), rpr))
  )
  stats::setNames(list(paste0(kopf, gsub(RUN_RE, "", schwanz, perl = TRUE), neu, "</w:p>")), id)
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

block_title <- function(tpl, id) {
  cap <- caption_block(tpl, id)
  if (is.na(cap)) NA_character_ else trimws(para_text(tpl$blocks[[cap]]$xml))
}

is_figure <- function(b) b$type == "p" && grepl("<pic:pic", b$xml, fixed = TRUE)

fig_tag <- function(xml) first_match("<wp:docPr [^>]*descr=\"gba:([^\"]+)\"", xml)

text_tags <- function(xml) {
  hits <- regmatches(xml, gregexpr("w:name=\"_gba_[A-Za-z0-9_]+_[0-9]+\"", xml))[[1]]
  unique(sub("w:name=\"_gba_(.+)_[0-9]+\"", "\\1", hits))
}

para_removed <- function(x) {
  grepl("^<w:p(?: [^>]*)?><w:pPr>(?:(?!</w:pPr>).)*<w:rPr>(?:(?!</w:rPr>).)*<w:del ", x, perl = TRUE)
}

#' What a user can pick in a document: headings, text, tables, figures, placeholders.
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
    list(block = id, tag = get_tag(b$table$head) %|na|% NULL, titel = block_title(tpl, id) %|na|% NULL,
         zeilen = length(b$table$rows), geschuetzt = b$table$fest, offen_von = rev_authors(b$xml))
  })
  live <- Filter(function(id) tpl$blocks[[id]]$type == "p" && !para_removed(tpl$blocks[[id]]$xml), tpl$ids)
  bilder <- lapply(Filter(function(id) is_figure(tpl$blocks[[id]]), live), function(id) {
    x <- tpl$blocks[[id]]$xml
    list(block = id, tag = fig_tag(x) %|na|% NULL, titel = block_title(tpl, id) %|na|% NULL, offen_von = rev_authors(x))
  })
  texte <- Filter(function(id) kinds[[id]] %in% c("absatz", "antwort") && !is_figure(tpl$blocks[[id]]), live)
  absaetze <- Filter(Negate(is.null), lapply(texte, function(id) {
    x <- tpl$blocks[[id]]$xml
    t <- clean_text(para_text(x))
    if (!nzchar(t)) return(NULL)
    list(block = id, tag = text_tags(x)[1] %|na|% NULL, text = substr(t, 1L, 80L), offen_von = rev_authors(x))
  }))
  ph <- all_placeholders(tpl)
  list(sha256 = unname(tools::sha256sum(docx)), ueberschriften = heads, absaetze = absaetze,
       tabellen = Filter(Negate(is.null), tabs), abbildungen = bilder,
       platzhalter = lapply(seq_along(ph), function(i) list(block = names(ph)[i], name = ph[[i]])))
}

#' A table waiting for its tracked deletion to be accepted.
table_removed <- function(b) {
  all(vapply(b$table$rows, function(r) grepl("<w:del [^>]*/>", r$pre, perl = TRUE), NA))
}

check_editable <- function(id, tpl) {
  if (tpl$blocks[[id]]$table$fest) stop("Block ", id, " enthält geschützte Inhalte.")
  id
}

find_tag <- function(tag, tpl) {
  art <- vapply(tpl$ids, function(id) {
    b <- tpl$blocks[[id]]
    if (b$type == "tbl") return(if (identical(get_tag(b$table$head), tag) && !table_removed(b)) "tabelle" else "")
    if (b$type != "p" || para_removed(b$xml)) return("")
    if (identical(fig_tag(b$xml), tag)) return("abbildung")
    if (tag %in% text_tags(b$xml)) "text" else ""
  }, "")
  ids <- tpl$ids[nzchar(art)]
  if (!length(ids)) stop("Keine Tabelle, Abbildung oder kein Text mit der Kennung ", tag, ". Mit `outline` nachsehen.")
  arten <- unique(art[nzchar(art)])
  if (length(arten) > 1L || (arten != "text" && length(ids) > 1L)) stop("Kennung ", tag, " kommt mehrfach vor: ", paste(ids, collapse = ", "))
  if (arten == "tabelle") check_editable(ids, tpl)
  if (arten == "text") {
    ids <- tpl$ids[match(ids[1], tpl$ids):match(ids[length(ids)], tpl$ids)]
    if (any(vapply(tpl$blocks[ids], `[[`, "", "type") != "p")) stop("Text ", tag, " ist nicht mehr zusammenhängend.")
    # Deleted paragraphs in between stay as they are.
    ids <- ids[!vapply(tpl$blocks[ids], function(b) para_removed(b$xml), NA)]
    if (!all(vapply(tpl$blocks[ids], function(b) tag %in% text_tags(b$xml), NA))) stop("Text ", tag, " ist nicht mehr zusammenhängend.")
  }
  list(ids = ids, art = arten)
}

find_target <- function(ziel, tpl, sha) {
  if (!is.null(ziel$tag)) return(find_tag(ziel$tag, tpl))
  if (is.null(ziel$block)) stop("Das Ziel braucht `tag` oder `block`.")
  # Block ids are positions and hold only for the file they were read from.
  if (is.null(sha) || !identical(sha, unname(tools::sha256sum(tpl$path)))) {
    stop("`block` gilt nur zusammen mit der sha256 der hochgeladenen Datei aus gba_outline().")
  }
  b <- tpl$blocks[[ziel$block]]
  if (is.null(b) || b$type == "sdt") stop("Block ", ziel$block, " lässt sich nicht bearbeiten.")
  if (b$type == "tbl") return(list(ids = check_editable(ziel$block, tpl), art = "tabelle"))
  if (is_figure(b)) return(list(ids = ziel$block, art = "abbildung"))
  bis <- ziel$bis %||% ziel$block
  von <- match(ziel$block, tpl$ids)
  if (is.na(match(bis, tpl$ids)) || match(bis, tpl$ids) < von) stop("`bis` liegt nicht hinter ", ziel$block, ".")
  ids <- tpl$ids[von:match(bis, tpl$ids)]
  # Questions, explanations, and headings are required by the G-BA.
  arten <- vapply(tpl$blocks[ids], block_kind, "")
  if (!all(arten %in% c("absatz", "antwort"))) stop("Nur Text und Antworten lassen sich ersetzen, nicht ", paste(unique(setdiff(arten, c("absatz", "antwort"))), collapse = ", "), ".")
  list(ids = ids, art = "text")
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

payload_art <- function(o) {
  art <- intersect(c("tabelle", "abbildung", "text"), names(o))
  if (length(art) != 1L) stop("Eine Änderung braucht genau eins von `tabelle`, `abbildung`, `text`.")
  art
}

#' Applies operations to a docx and writes the result.
#'
#' `ops` is a list or the path of a JSON file with `aenderungen` and, for targets
#' by block id, the `sha256` from gba_outline(). Each change is one of
#'   list(op = "werte", ziel = list(tag = "x"), tabelle = spec)
#'   list(op = "ersetzen", ziel = list(block = "b0123"), tabelle|abbildung|text = ...)
#'   list(op = "einfuegen", platzhalter = "x", tabelle|abbildung|text = ...)
#'   list(op = "titel", ziel = list(tag = "x"), titel = "Neuer Titel")
#'   list(op = "felder")
#' `abbildung` is list(datei=, titel=, tag=, breite=) and `text` is
#' list(baustein=, werte=, tag=), see typst/methoden.typ. Relative paths start at
#' the ops file. A changed `titel` in a spec changes the caption. Fields update
#' after every change; `felder` asks for that without one. Any problem stops the
#' run and writes nothing. Cells it must leave alone are listed under `offen` in
#' the returned report.
gba_apply <- function(docx, ops, out, autor = "Automatisierung") {
  basis <- if (is.character(ops)) dirname(normalizePath(ops)) else getwd()
  if (is.character(ops)) ops <- jsonlite::fromJSON(read_text(ops), simplifyVector = FALSE)
  tpl <- read_template(docx)
  on.exit(unlink(tpl$dir, recursive = TRUE))
  ctx <- new_ctx(tpl, basis)
  ctx$basis <- basis
  ctx$autor <- autor
  ctx$autor_xml <- xml_escape(autor)
  ctx$autor_re <- gsub("([.^$|()*+?{}\\[\\]\\\\])", "\\\\\\1", ctx$autor_xml, perl = TRUE)
  # Revision ids must not repeat those of earlier rounds.
  roh <- read_text(file.path(tpl$dir, "word/document.xml"))
  belegt <- regmatches(roh, gregexpr("(?<= w:id=\")[0-9]+(?=\")", roh, perl = TRUE))[[1]]
  ctx$uniq <- max(ctx$uniq, as.numeric(belegt))
  ctx$datum <- format(Sys.time(), "%Y-%m-%dT%H:%M:%SZ", tz = "UTC")
  xml <- vapply(tpl$blocks, `[[`, "", "xml")
  erledigt <- character()
  bericht <- list()
  offen <- list()
  felder <- FALSE
  belege <- function(neu) {
    doppelt <- intersect(names(neu), erledigt)
    if (length(doppelt)) stop("Block ", doppelt[1], " ist mehrfach Ziel einer Änderung.")
    erledigt <<- c(erledigt, names(neu))
    for (id in names(neu)) xml[[id]] <<- neu[[id]]
  }
  for (o in ops$aenderungen) {
    if (identical(o$op, "felder")) { felder <- TRUE; next }
    if (!o$op %in% c("werte", "ersetzen", "einfuegen", "titel")) stop("Unbekannte Änderung: ", o$op)
    name <- o$ziel$tag %||% o$ziel$block %||% o$platzhalter
    eintrag <- list(op = o$op, ziel = name)
    if (o$op == "einfuegen") {
      art <- payload_art(o)
      id <- find_placeholder(o$platzhalter, tpl)
      check_paras(id, ctx, name)
      belege(stats::setNames(list(op_einfuegen(id, o, ctx, art)), id))
      eintrag$block <- id
    } else {
      z <- find_target(o$ziel, tpl, ops$sha256)
      eintrag$block <- z$ids[1]
      titel <- o$titel %||% o$tabelle$titel %||% o$abbildung$titel
      if (o$op == "titel") {
        if (is.null(titel)) stop("`titel` fehlt.")
      } else {
        art <- if (o$op == "werte") "tabelle" else payload_art(o)
        if (art != z$art) stop(sprintf("%s ist %s, die Änderung liefert %s.", name, z$art, art))
      }
      if (o$op == "werte") {
        r <- op_werte(tpl$blocks[[z$ids]], o$tabelle, ctx, name)
        belege(stats::setNames(list(r$xml), z$ids))
        offen <- c(offen, r$offen)
        eintrag$zellen <- r$geaendert
      } else if (o$op == "ersetzen") {
        check_loss(paste(vapply(tpl$blocks[z$ids], `[[`, "", "xml"), collapse = ""), o, name)
        belege(switch(art,
          tabelle = op_ersetzen_tabelle(z$ids, o$tabelle, ctx, name),
          abbildung = op_ersetzen_abbildung(z$ids, o$abbildung, ctx, name),
          text = op_ersetzen_text(z$ids, o$text, ctx, name)))
      }
      if (!is.null(titel)) {
        if (z$art == "text") stop(name, ": Text hat keine Beschriftung.")
        cap <- caption_block(tpl, z$ids)
        if (is.na(cap)) stop(name, ": keine Beschriftung gefunden.")
        belege(op_titel(cap, titel, z$art, ctx, name))
      }
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
  fertig <- unlist(lapply(ops$aenderungen, `[[`, "platzhalter"))
  list(datei = out, aenderungen = bericht, offen = offen,
       platzhalter_offen = lapply(setdiff(all_placeholders(tpl), fertig), identity),
       hinweis = "Seitenzahlen aktualisiert Word.")
}
