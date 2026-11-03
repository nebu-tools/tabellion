# Edits an existing docx in place. The docx stays the master: every operation
# touches only its target and writes tracked changes for the team to review.

REV_XP <- paste0(".//w:", c("ins", "del", "moveFrom", "moveTo", "rPrChange", "pPrChange", "tcPrChange",
                              "trPrChange", "tblPrChange"), collapse = "|")

rev_authors <- function(n, ns) unique(stats::na.omit(wattr(xfind(n, REV_XP, ns), "author", ns)))

#' A block's content as nodes, parsed once and only read.
blk <- function(tpl, id) {
  n <- tpl$cache[[id]]
  if (is.null(n)) n <- tpl$cache[[id]] <- xml_node1(tpl$blocks[[id]]$xml, tpl$root_tag)
  n
}

#' A fresh copy of a block to edit.
blk_edit <- function(ctx, id) xml_node1(ctx$tpl$blocks[[id]]$xml, ctx$tpl$root_tag)

rev_vals <- function(ctx) list(id = as.character(ctx$next_id()), author = ctx$autor, date = ctx$datum)

#' Puts a node into a new w:ins or w:del.
wrap_rev <- function(n, tag, ctx) {
  xml2::xml_add_parent(n, paste0("w:", tag))
  set_wattrs(xml2::xml_parent(n), rev_vals(ctx), ctx$ns)
}

#' Marks a paragraph mark (ins and del come first) or a table row (they come
#' last, before trPrChange) as inserted or deleted.
mark_el <- function(pr, tag, ctx, row = FALSE) {
  ns <- ctx$ns
  ins <- xml2::xml_find_first(pr, "w:ins", ns)
  m <- if (tag == "del" && !inherits(ins, "xml_missing")) {
    xml2::xml_add_sibling(ins, "w:del")
    xml2::xml_find_first(ins, "following-sibling::*[1]")
  } else if (row) {
    add_el(pr, paste0("w:", tag), ns, before = c(if (tag == "ins") "del", "trPrChange"))
  } else {
    add_el(pr, paste0("w:", tag), ns, first = TRUE)
  }
  set_wattrs(m, rev_vals(ctx), ns)
}

para_mark <- function(p, tag, ctx) {
  ns <- ctx$ns
  ppr <- xml2::xml_find_first(p, "w:pPr", ns)
  if (inherits(ppr, "xml_missing")) ppr <- add_el(p, "w:pPr", ns, first = TRUE)
  mark_el(pr_child(ppr, "rPr", ns, PPR_TAIL), tag, ctx)
}

row_mark <- function(tr, tag, ctx) {
  ns <- ctx$ns
  trpr <- xml2::xml_find_first(tr, "w:trPr", ns)
  if (inherits(trpr, "xml_missing")) {
    ex <- xml2::xml_find_first(tr, "w:tblPrEx", ns)
    if (inherits(ex, "xml_missing")) trpr <- add_el(tr, "w:trPr", ns, first = TRUE)
    else { xml2::xml_add_sibling(ex, "w:trPr"); trpr <- xml2::xml_find_first(tr, "w:trPr", ns) }
  }
  mark_el(trpr, tag, ctx, row = TRUE)
}

rename_all <- function(n, from, to, ns) {
  for (k in xfind(n, sprintf("descendant-or-self::w:%s", from), ns)) xml2::xml_set_name(k, to)
}

as_deleted <- function(n, ns) { rename_all(n, "t", "delText", ns); rename_all(n, "instrText", "delInstrText", ns) }

as_live <- function(n, ns) { rename_all(n, "delText", "t", ns); rename_all(n, "delInstrText", "instrText", ns) }

#' Marks paragraphs, rows, runs and formulas under `n` as a tracked insertion.
mark_ins <- function(n, ctx) {
  ns <- ctx$ns
  for (r in xfind(n, ".//w:r[not(ancestor::w:ins)][not(ancestor::w:txbxContent)]", ns)) wrap_rev(r, "ins", ctx)
  # Formula runs are not w:r, so the tracked change wraps the whole formula.
  if ("m" %in% names(ns)) {
    for (m in xfind(n, ".//m:oMathPara[not(ancestor::w:ins)] | .//m:oMath[not(ancestor::m:oMathPara)][not(ancestor::w:ins)]", ns)) wrap_rev(m, "ins", ctx)
  }
  for (p in xfind(n, "descendant-or-self::w:p", ns)) para_mark(p, "ins", ctx)
  for (tr in xfind(n, "descendant-or-self::w:tr", ns)) row_mark(tr, "ins", ctx)
  n
}

#' Marks content under `n` as a tracked deletion.
mark_del <- function(n, ctx) {
  ns <- ctx$ns
  for (r in xfind(n, ".//w:r[not(ancestor::w:del)][not(ancestor::w:txbxContent)]", ns)) {
    as_deleted(r, ns)
    wrap_rev(r, "del", ctx)
  }
  for (p in xfind(n, "descendant-or-self::w:p", ns)) para_mark(p, "del", ctx)
  for (tr in xfind(n, "descendant-or-self::w:tr", ns)) row_mark(tr, "del", ctx)
  n
}

#' New content as a tracked insertion.
track_ins <- function(xml, ctx) {
  b <- xml_frag(xml, ctx$tpl$root_tag)
  mark_ins(b, ctx)
  frag_str(b, ctx$tpl$root_tag)
}

own <- function(set, ctx) set[wattr(set, "author", ctx$ns) %in% ctx$autor]

#' Takes back this author's insertions and deletions of content. Marks on
#' paragraphs and rows stay, the callers deal with those.
revert_own <- function(n, ctx) {
  ns <- ctx$ns
  frei <- "[not(parent::w:rPr)][not(parent::w:trPr)]"
  rm_nodes(own(xfind(n, paste0(".//w:ins", frei), ns), ctx))
  for (d in own(xfind(n, paste0(".//w:del", frei), ns), ctx)) {
    for (k in xml2::xml_children(d)) {
      as_live(k, ns)
      xml2::xml_add_sibling(d, k, .where = "before")
    }
    rm_nodes(d)
  }
  n
}

clean_text <- function(x) trimws(gsub("[ \t\r\n\u00A0]+", " ", x, perl = TRUE))

node_text <- function(n, ctx) para_text(node_str(n, ctx$tpl$root_tag))

COMPLEX_XP <- paste0("descendant-or-self::w:", c("fldChar", "fldSimple", "footnoteReference", "endnoteReference", "drawing",
                     "pict", "object", "hyperlink", "sdt", "commentReference", "commentRangeStart", "tbl"), collapse = "|")

#' Deletes runs of a paragraph and adds new text, both tracked, at its end.
replace_runs <- function(p, runs, text, ctx) {
  ns <- ctx$ns
  rpr <- NULL
  if (length(runs)) {
    d <- set_wattrs(add_el(p, "w:del", ns), rev_vals(ctx), ns)
    for (r in runs) {
      as_deleted(r, ns)
      xml2::xml_add_child(d, r)
    }
    rm_nodes(runs)
    rpr <- xml2::xml_find_first(d, "w:r[1]/w:rPr", ns)
  }
  if (nzchar(text)) {
    i <- set_wattrs(add_el(p, "w:ins", ns), rev_vals(ctx), ns)
    r <- add_el(i, "w:r", ns)
    if (!is.null(rpr) && !inherits(rpr, "xml_missing")) xml2::xml_add_child(r, rpr)
    add_text(r, text, ns)
  }
  p
}

#' New value for one cell. Only plain text is replaced, so a field, a citation
#' or a footnote is never lost to a table update. Returns the new cell or a reason.
cell_update <- function(tc, neu, ctx) {
  ns <- ctx$ns
  neu <- clean_text(neu)
  if (identical(clean_text(node_text(tc, ctx)), neu)) return(list(status = "gleich"))
  fremd <- setdiff(rev_authors(tc, ns), ctx$autor)
  if (length(fremd)) return(list(status = "offen", grund = paste("offene Änderung von", paste(fremd, collapse = ", "))))
  k <- xml_node1(node_str(tc, ctx$tpl$root_tag), ctx$tpl$root_tag)
  revert_own(k, ctx)
  if (xhas(k, COMPLEX_XP, ns)) return(list(status = "offen", grund = "Feld, Fußnote oder Zitat in der Zelle"))
  ps <- xfind(k, "w:p", ns)
  voll <- which(nzchar(trimws(vapply(ps, node_text, "", ctx = ctx))))
  if (length(voll) > 1L) return(list(status = "offen", grund = "mehrere Absätze in der Zelle"))
  ziel <- ps[[if (length(voll)) voll else 1L]]
  replace_runs(ziel, xfind(ziel, "w:r", ns), neu, ctx)
  list(status = "geaendert", node = k)
}

#' Non-continuation cells of a Word table on the grid.
table_shape <- function(tbl, ns) {
  lapply(xfind(tbl, "w:tr", ns), function(r) {
    col <- 1L + as.integer(wattr(xml2::xml_find_first(r, "w:trPr/w:gridBefore", ns), "val", ns) %|na|% "0")
    out <- list()
    tcs <- xfind(r, "w:tc", ns)
    for (i in seq_along(tcs)) {
      n <- as.integer(wattr(xml2::xml_find_first(tcs[[i]], "w:tcPr/w:gridSpan", ns), "val", ns) %|na|% "1")
      vm <- xml2::xml_find_first(tcs[[i]], "w:tcPr/w:vMerge", ns)
      cont <- !inherits(vm, "xml_missing") && wattr(vm, "val", ns) %in% c(NA, "continue")
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

op_werte <- function(id, spec, ctx, name) {
  ns <- ctx$ns
  tbl <- blk_edit(ctx, id)
  ist <- table_shape(tbl, ns)
  soll <- lapply(spec_grid(spec_rows(spec))$rows, function(r) Filter(function(z) !z$cont, r))
  if (!identical(form(ist), form(soll))) {
    stop(sprintf("%s: Tabellenform weicht ab (%d statt %d Zeilen oder andere Spalten). Tabelle mit `ersetzen` austauschen.",
                 name, length(soll), length(ist)))
  }
  geaendert <- 0L
  offen <- list()
  rows <- xfind(tbl, "w:tr", ns)
  for (r in seq_along(ist)) {
    tcs <- xfind(rows[[r]], "w:tc", ns)
    for (j in seq_along(ist[[r]])) {
      tc <- tcs[[ist[[r]][[j]]$i]]
      u <- cell_update(tc, soll[[r]][[j]]$text, ctx)
      if (u$status == "geaendert") {
        geaendert <- geaendert + 1L
        xml2::xml_replace(tc, u$node)
      }
      if (u$status == "offen") {
        offen[[length(offen) + 1L]] <- list(tabelle = name, zeile = r, spalte = ist[[r]][[j]]$spalte, grund = u$grund)
      }
    }
  }
  list(xml = if (geaendert) node_str(tbl, ctx$tpl$root_tag) else ctx$tpl$blocks[[id]]$xml, geaendert = geaendert, offen = offen)
}

# What may go away with a target. A citation is only removed on request, and a
# paragraph that closes a section or sits inside a field must stay whole.
check_loss <- function(ids, o, ctx, name) {
  xp <- ".//w:instrText[contains(., 'EN.CITE') or contains(., 'EN.REFLIST')] | .//w:fldSimple[contains(@w:instr, 'EN.')]"
  zitat <- any(vapply(ids, function(id) xhas(blk(ctx$tpl, id), xp, ctx$ns), NA))
  if (zitat && !isTRUE(o$zitate_loeschen)) {
    stop(name, " enthält EndNote-Zitate. Sie gingen verloren. Mit `zitate_loeschen: true` bestätigen oder Text in Word ändern.")
  }
}

check_fremd <- function(ids, ctx, name) {
  fremd <- setdiff(unlist(lapply(ids, function(id) rev_authors(blk(ctx$tpl, id), ctx$ns))), ctx$autor)
  if (length(fremd)) stop(sprintf("%s: offene Änderungen von %s. Erst in Word annehmen oder ablehnen.", name, paste(unique(fremd), collapse = ", ")))
}

#' Open fields before each block, so a target cannot cut one in half.
field_depth <- function(ctx) {
  if (is.null(ctx$tiefe)) {
    n <- function(id, ty) length(xfind(blk(ctx$tpl, id), sprintf(".//w:fldChar[@w:fldCharType='%s']", ty), ctx$ns))
    ctx$tiefe <- stats::setNames(cumsum(vapply(ctx$tpl$ids, function(id) n(id, "begin") - n(id, "end"), 0L)), ctx$tpl$ids)
  }
  ctx$tiefe
}

check_paras <- function(ids, ctx, name) {
  ns <- ctx$ns
  tiefe <- field_depth(ctx)
  von <- match(ids[1], ctx$tpl$ids)
  vor <- if (von > 1L) tiefe[[von - 1L]] else 0L
  if (vor != 0L || tiefe[[ids[length(ids)]]] != 0L) stop(name, " liegt in einem Feld, z. B. einem Literaturverzeichnis.")
  nodes <- lapply(ids, function(id) blk(ctx$tpl, id))
  if (any(vapply(nodes, xhas, NA, xp = ".//w:sectPr", ns = ns))) stop(name, " enthält einen Abschnittswechsel.")
  # An own pending paragraph goes away as a whole, so its formula is not at risk.
  fremd <- Filter(function(n) !para_own(n, ctx), nodes)
  if (any(vapply(fremd, xhas, NA, xp = ".//m:oMath", ns = ns))) stop(name, " enthält eine Formel. In Word ändern.")
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
  ns <- ctx$ns
  check_fremd(id, ctx, name)
  tbl <- blk_edit(ctx, id)
  tag <- spec$tag %||% (get_tag(tbl, ns) %|na|% NULL)
  neu <- track_ins(spec_table(spec, ctx$tpl$blocks[[id]]$table, ctx, tag = tag), ctx)
  zeilen <- xfind(tbl, "w:tr", ns)
  eigen <- vapply(zeilen, function(z) length(own(xfind(z, "w:trPr/w:ins", ns), ctx)) > 0L, NA)
  # A table from an earlier round the team has not accepted yet just goes away.
  if (all(eigen)) return(stats::setNames(list(neu), id))
  # The old table keeps its tag, so rejecting the change restores a usable target.
  mark_del(revert_own(tbl, ctx), ctx)
  trenner <- track_ins(paste0("<w:p>", text_ppr(ctx), "</w:p>"), ctx)
  stats::setNames(list(paste0(node_str(tbl, ctx$tpl$root_tag), trenner, neu)), id)
}

para_own <- function(n, ctx) length(own(xfind(n, "w:pPr/w:rPr/w:ins", ctx$ns), ctx)) > 0L

# The new content goes behind the old paragraphs, which stay as a deletion.
ersetze_paras <- function(ids, neu, ctx) {
  alt <- vapply(ids, function(id) {
    p <- blk_edit(ctx, id)
    if (para_own(p, ctx)) "" else node_str(mark_del(revert_own(p, ctx), ctx), ctx$tpl$root_tag)
  }, "")
  n <- length(alt)
  alt[n] <- paste0(alt[n], neu)
  as.list(alt)
}

op_ersetzen_text <- function(ids, s, ctx, name) {
  check_fremd(ids, ctx, name)
  check_paras(ids, ctx, name)
  tag <- s$tag %||% unlist(lapply(ids, function(id) text_tags(blk(ctx$tpl, id), ctx$ns)))[1]
  ersetze_paras(ids, track_ins(neuer_text(s, ctx, if (is.na(tag %||% NA)) NULL else tag), ctx), ctx)
}

PIC_XP <- ".//*[local-name()='pic'][namespace-uri()='http://schemas.openxmlformats.org/drawingml/2006/picture']"

op_ersetzen_abbildung <- function(id, s, ctx, name) {
  ns <- ctx$ns
  check_fremd(id, ctx, name)
  check_paras(id, ctx, name)
  p <- blk(ctx$tpl, id)
  if (length(xfind(p, PIC_XP, ns)) != 1L || xhas(p, ".//w:txbxContent", ns) || nzchar(trimws(node_text(p, ctx)))) {
    stop(name, ": der Absatz enthält mehr als eine Abbildung. In Word ändern.")
  }
  tag <- s$tag %||% (fig_tag(p) %|na|% NULL)
  ersetze_paras(id, track_ins(neue_abbildung(s, ctx, tag), ctx), ctx)
}

op_einfuegen <- function(id, o, ctx, art) {
  neu <- switch(art,
    tabelle = paste0(
      if (!is.null(o$tabelle$titel)) caption_xml(o$tabelle$titel, "Tabelle", ctx),
      spec_table(o$tabelle, block_of(ctx, default_table(ctx))$table, ctx), "<w:p>", text_ppr(ctx), "</w:p>"),
    abbildung = paste0(neue_abbildung(o$abbildung, ctx),
                       if (!is.null(o$abbildung$titel)) caption_xml(o$abbildung$titel, "Abbildung", ctx)),
    text = neuer_text(o$text, ctx))
  paste0(node_str(mark_del(blk_edit(ctx, id), ctx), ctx$tpl$root_tag), track_ins(neu, ctx))
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
    if (p$type == "tbl" && table_removed(blk(tpl, tpl$ids[i]), tpl$ns)) next
    if (p$type != "p") return(NA_character_)
    if (grepl("Beschriftung", p$style)) return(tpl$ids[i])
    if (nzchar(trimws(para_text(p$xml)))) return(NA_character_)
  }
  NA_character_
}

caption_title <- function(xml) sub("^[^:]*:\\s*", "", clean_text(para_text(xml)))

#' Position of the child that closes the caption's number field.
seq_end <- function(p, ns) {
  kids <- xml2::xml_children(p)
  tiefe <- 0L
  code <- ""
  for (i in seq_along(kids)) {
    k <- kids[[i]]
    if (xml2::xml_name(k, ns) == "w:fldSimple") {
      if (grepl("^\\s*SEQ\\b", wattr(k, "instr", ns), perl = TRUE)) return(i)
      next
    }
    for (f in xfind(k, "descendant-or-self::w:fldChar | descendant-or-self::w:instrText", ns)) {
      if (xml2::xml_name(f, ns) == "w:instrText") {
        if (tiefe == 1L) code <- paste0(code, xml2::xml_text(f))
        next
      }
      ty <- wattr(f, "fldCharType", ns)
      if (ty == "begin") { tiefe <- tiefe + 1L; if (tiefe == 1L) code <- "" }
      if (ty == "end") {
        tiefe <- tiefe - 1L
        if (tiefe == 0L && grepl("^\\s*SEQ\\b", code, perl = TRUE)) return(i)
      }
    }
  }
  NA_integer_
}

#' Tracked new title. The number field stays, so only the text behind it changes.
op_titel <- function(id, titel, art, ctx, name) {
  ns <- ctx$ns
  rt <- ctx$tpl$root_tag
  b <- ctx$tpl$blocks[[id]]
  if (identical(caption_title(b$xml), clean_text(titel))) return(list())
  check_fremd(id, ctx, name)
  if (para_own(blk(ctx$tpl, id), ctx)) {
    return(stats::setNames(list(track_ins(caption_xml(titel, if (art == "tabelle") "Tabelle" else "Abbildung", ctx), ctx)), id))
  }
  p <- revert_own(blk_edit(ctx, id), ctx)
  grenze <- seq_end(p, ns)
  if (is.na(grenze)) stop(name, ": Beschriftung ohne Nummernfeld. In Word ändern.")
  kids <- xml2::xml_children(p)
  schwanz <- if (grenze < length(kids)) kids[(grenze + 1L):length(kids)] else list()
  if (any(vapply(schwanz, xhas, NA, xp = COMPLEX_XP, ns = ns))) stop(name, ": Feld oder Zitat im Titel. In Word ändern.")
  rest <- paste0("<w:p>", paste(vapply(schwanz, node_str, "", root_tag = rt), collapse = ""), "</w:p>")
  trenner <- first_match("^(\\s*[:.]\\s*)", clean_text(para_text(rest))) %|na|% ": "
  runs <- Filter(function(k) xml2::xml_name(k, ns) == "w:r", schwanz)
  replace_runs(p, runs, paste0(trenner, clean_text(titel)), ctx)
  stats::setNames(list(node_str(p, rt)), id)
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

is_figure <- function(tpl, id) tpl$blocks[[id]]$type == "p" && xhas(blk(tpl, id), PIC_XP, tpl$ns)

fig_tag <- function(n) {
  d <- xml2::xml_attr(xml2::xml_find_all(n, ".//*[local-name()='docPr']"), "descr")
  d <- d[!is.na(d) & startsWith(d, "gba:")]
  if (length(d)) substring(d[1], 5L) else NA_character_
}

text_tags <- function(n, ns) {
  nm <- wattr(xfind(n, ".//w:bookmarkStart", ns), "name", ns)
  nm <- nm[grepl("^_gba_.+_[0-9]+$", nm)]
  unique(sub("^_gba_(.+)_[0-9]+$", "\\1", nm))
}

para_removed <- function(n, ns) xhas(n, "w:pPr/w:rPr/w:del", ns)

with_nodes <- function(tpl) {
  tpl$cache <- new.env(parent = emptyenv())
  tpl$ns <- root_ns(tpl$root_tag)
  tpl
}

#' What a user can pick in a document: headings, text, tables, figures, placeholders.
gba_outline <- function(docx) {
  tpl <- with_nodes(read_template(docx))
  on.exit(unlink(tpl$dir, recursive = TRUE))
  ns <- tpl$ns
  kinds <- vapply(tpl$blocks, block_kind, "")
  heads <- lapply(tpl$ids[kinds %in% c("heading", "heading-frei")], function(id) {
    b <- tpl$blocks[[id]]
    list(block = id, stil = b$style, text = trimws(para_text(b$xml)))
  })
  tabs <- lapply(tpl$ids[kinds %in% c("tabelle", "fest")], function(id) {
    b <- tpl$blocks[[id]]
    if (b$type != "tbl") return(NULL)
    n <- blk(tpl, id)
    if (table_removed(n, ns)) return(NULL)
    list(block = id, tag = get_tag(n, ns) %|na|% NULL, titel = block_title(tpl, id) %|na|% NULL,
         zeilen = length(b$table$rows), geschuetzt = b$table$fest, offen_von = rev_authors(n, ns))
  })
  live <- Filter(function(id) tpl$blocks[[id]]$type == "p" && !para_removed(blk(tpl, id), ns), tpl$ids)
  bilder <- lapply(Filter(function(id) is_figure(tpl, id), live), function(id) {
    n <- blk(tpl, id)
    list(block = id, tag = fig_tag(n) %|na|% NULL, titel = block_title(tpl, id) %|na|% NULL, offen_von = rev_authors(n, ns))
  })
  texte <- Filter(function(id) kinds[[id]] %in% c("absatz", "antwort") && !is_figure(tpl, id), live)
  absaetze <- Filter(Negate(is.null), lapply(texte, function(id) {
    t <- clean_text(para_text(tpl$blocks[[id]]$xml))
    if (!nzchar(t)) return(NULL)
    n <- blk(tpl, id)
    list(block = id, tag = text_tags(n, ns)[1] %|na|% NULL, text = substr(t, 1L, 80L), offen_von = rev_authors(n, ns))
  }))
  ph <- all_placeholders(tpl)
  list(sha256 = unname(tools::sha256sum(docx)), ueberschriften = heads, absaetze = absaetze,
       tabellen = Filter(Negate(is.null), tabs), abbildungen = bilder,
       platzhalter = lapply(seq_along(ph), function(i) list(block = names(ph)[i], name = ph[[i]])))
}

#' A table waiting for its tracked deletion to be accepted.
table_removed <- function(n, ns) {
  rows <- xfind(n, "w:tr", ns)
  all(vapply(rows, xhas, NA, xp = "w:trPr/w:del", ns = ns))
}

check_editable <- function(id, tpl) {
  if (tpl$blocks[[id]]$table$fest) stop("Block ", id, " enthält geschützte Inhalte.")
  id
}

find_tag <- function(tag, tpl) {
  ns <- tpl$ns
  # Cheap text search first, so only candidate blocks get parsed.
  kandidat <- vapply(tpl$blocks, function(b) grepl(tag, b$xml, fixed = TRUE), NA)
  art <- vapply(tpl$ids, function(id) {
    b <- tpl$blocks[[id]]
    if (!kandidat[[id]]) return("")
    n <- blk(tpl, id)
    if (b$type == "tbl") return(if (identical(get_tag(n, ns), tag) && !table_removed(n, ns)) "tabelle" else "")
    if (b$type != "p" || para_removed(n, ns)) return("")
    if (identical(fig_tag(n), tag)) return("abbildung")
    if (tag %in% text_tags(n, ns)) "text" else ""
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
    ids <- ids[!vapply(ids, function(id) para_removed(blk(tpl, id), ns), NA)]
    if (!all(vapply(ids, function(id) tag %in% text_tags(blk(tpl, id), ns), NA))) stop("Text ", tag, " ist nicht mehr zusammenhängend.")
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
  if (is_figure(tpl, ziel$block)) return(list(ids = ziel$block, art = "abbildung"))
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
  tpl <- with_nodes(read_template(docx))
  on.exit(unlink(tpl$dir, recursive = TRUE))
  ctx <- new_ctx(tpl, basis)
  ctx$basis <- basis
  ctx$autor <- autor
  ctx$ns <- tpl$ns
  # Revision ids must not repeat those of earlier rounds.
  doc <- xml2::read_xml(file.path(tpl$dir, "word/document.xml"))
  belegt <- suppressWarnings(as.numeric(xml2::xml_text(xml2::xml_find_all(doc, "//@w:id", tpl$ns))))
  ctx$uniq <- max(ctx$uniq, belegt, na.rm = TRUE)
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
        r <- op_werte(z$ids, o$tabelle, ctx, name)
        belege(stats::setNames(list(r$xml), z$ids))
        offen <- c(offen, r$offen)
        eintrag$zellen <- r$geaendert
      } else if (o$op == "ersetzen") {
        check_loss(z$ids, o, ctx, name)
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
