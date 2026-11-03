# Typst blocks + template -> docx.

new_ctx <- function(tpl, proj_dir) {
  ctx <- new.env(parent = emptyenv())
  ctx$tpl <- tpl
  ctx$dir <- proj_dir
  ctx$changed <- FALSE
  ctx$stats <- list(verbatim = 0L, geaendert = 0L, neu = 0L)
  ctx$seen <- list()
  ctx$uniq <- 500000L
  ctx$mit <- list()
  ctx$fn_new <- character()
  ctx$num_new <- character()
  ctx$media <- list()
  # An edited file may already hold figures from an earlier round.
  rels <- read_text(file.path(tpl$dir, "word/_rels/document.xml.rels"))
  belegt <- regmatches(rels, gregexpr("(?<=gba_abbildung|rIdGba)[0-9]+", rels, perl = TRUE))[[1]]
  ctx$media_base <- max(0L, as.integer(belegt))
  fp <- file.path(tpl$dir, "word/footnotes.xml")
  ctx$fn_xml <- if (file.exists(fp)) read_text(fp) else ""
  ids <- regmatches(ctx$fn_xml, gregexpr("w:id=\"-?[0-9]+\"", ctx$fn_xml))[[1]]
  ctx$fn_next <- max(c(0L, as.integer(gsub("[^0-9-]", "", ids)))) + 1L
  ctx$next_id <- function() { ctx$uniq <- ctx$uniq + 1L; ctx$uniq }
  ctx$atom <- function(key, atoms, keys) {
    i <- match(as.character(key), keys)
    if (is.na(i)) stop("Feld ", key, " gehört nicht zu diesem Block.")
    atoms[[i]]
  }
  ctx$footnote <- function(body) {
    toks <- content_tokens(body, where = "Fußnote")
    id <- ctx$fn_next
    ctx$fn_next <- id + 1L
    runs <- paste(vapply(toks, function(t) {
      if (identical(t$type, "text")) token_run(t$text, apply_fmt("", t)) else ""
    }, ""), collapse = "")
    ctx$fn_new <- c(ctx$fn_new, sprintf(paste0(
      "<w:footnote w:id=\"%d\"><w:p><w:pPr><w:pStyle w:val=\"Funotentext\"/></w:pPr>",
      "<w:r><w:rPr><w:rStyle w:val=\"Funotenzeichen\"/></w:rPr><w:footnoteRef/></w:r>",
      "<w:r><w:t xml:space=\"preserve\"> </w:t></w:r>%s</w:p></w:footnote>"
    ), id, runs))
    sprintf("<w:r><w:rPr><w:rStyle w:val=\"Funotenzeichen\"/></w:rPr><w:footnoteReference w:id=\"%d\"/></w:r>", id)
  }
  ctx
}

mark_changed <- function(ctx, what) {
  ctx$changed <- TRUE
  ctx$stats[[what]] <- ctx$stats[[what]] + 1L
}

# Second and later uses of a template block must not reuse its identifiers.
clone_xml <- function(xml, ctx, occ) {
  xml <- gsub(" w14:(paraId|textId)=\"[^\"]*\"", "", xml, perl = TRUE)
  xml <- gsub("<w:sectPr.*?</w:sectPr>", "", xml, perl = TRUE)
  m <- gregexpr("<w:bookmarkStart [^>]*>", xml, perl = TRUE)
  tags <- regmatches(xml, m)[[1]]
  map <- character()
  for (t in tags) {
    old_id <- sub(".*w:id=\"([0-9]+)\".*", "\\1", t)
    old_nm <- sub(".*w:name=\"([^\"]+)\".*", "\\1", t)
    new_id <- ctx$next_id()
    new_nm <- sprintf("%s_%d", substr(old_nm, 1L, 30L), occ)
    map[old_nm] <- new_nm
    nt <- sub(paste0("w:id=\"", old_id, "\""), sprintf("w:id=\"%d\"", new_id), t, fixed = TRUE)
    nt <- sub(paste0("w:name=\"", old_nm, "\""), paste0("w:name=\"", new_nm, "\""), nt, fixed = TRUE)
    xml <- sub(t, nt, xml, fixed = TRUE)
    xml <- sub(paste0("<w:bookmarkEnd w:id=\"", old_id, "\"/>"),
               sprintf("<w:bookmarkEnd w:id=\"%d\"/>", new_id), xml, fixed = TRUE)
  }
  for (nm in names(map)) xml <- gsub(nm, map[[nm]], xml, fixed = TRUE)
  fr <- regmatches(xml, gregexpr("<w:footnoteReference w:id=\"[0-9]+\"/>", xml))[[1]]
  for (t in fr) {
    id <- sub(".*w:id=\"([0-9]+)\".*", "\\1", t)
    # The block may hold a footnote that this build created.
    defs <- paste(c(ctx$fn_xml, ctx$fn_new), collapse = "")
    src <- regmatches(defs, regexpr(sprintf("<w:footnote w:id=\"%s\".*?</w:footnote>", id), defs, perl = TRUE))
    nid <- ctx$fn_next
    ctx$fn_next <- nid + 1L
    if (length(src)) {
      ctx$fn_new <- c(ctx$fn_new, sub(sprintf("w:id=\"%s\"", id), sprintf("w:id=\"%d\"", nid), src, fixed = TRUE))
    }
    xml <- sub(t, sprintf("<w:footnoteReference w:id=\"%d\"/>", nid), xml, fixed = TRUE)
  }
  xml
}

para_sig <- function(p) token_signature(p$tokens)

# #feld shows content that build copies from the docx, so a text edit there
# would be lost without a trace.
check_atoms <- function(toks, orig, where) {
  squeeze <- function(x) gsub("[[:space:]\u00A0]+", "", x, perl = TRUE)
  for (t in toks) {
    if (!identical(t$type, "atom") || is.null(t$cached)) next
    a <- Filter(function(o) identical(o$type, "atom") && identical(as.character(o$key), as.character(t$key)), orig)
    if (length(a) && is.na(a[[1]]$on) && !identical(squeeze(a[[1]]$cached), squeeze(t$cached))) {
      stop(sprintf("%s: #feld(%s) ist geschützt und lässt sich hier nicht ändern. Inhalt in Word bearbeiten.", where, t$key))
    }
  }
}

#' Content for a paragraph prototype -> XML of one or more paragraphs/blocks.
fill_from_tokens <- function(proto, toks, ctx) {
  paras <- split_paragraphs(toks)
  out <- character(length(paras))
  in_list <- FALSE
  for (i in seq_along(paras)) {
    pt <- paras[[i]]
    if (length(pt) == 1L && identical(pt[[1]]$type, "block")) {
      node <- pt[[1]]$node
      numbered <- identical(node$func, "item") && (isTRUE(node$nummeriert) || !is.null(node$number))
      # Each run of numbered items is one list that starts at 1.
      if (numbered && !in_list) ctx$list_num <- new_list(ctx, proto)
      in_list <- numbered
      out[i] <- render_block_node(node, proto, ctx)
    } else {
      in_list <- FALSE
      out[i] <- render_para(proto, pt, ctx)
    }
  }
  paste(out, collapse = "")
}

num_style_id <- function(ctx, numbering) {
  st <- read_text(file.path(ctx$tpl$dir, "word/styles.xml"))
  m <- regexpr(sprintf("w:styleId=\"%s\".*?</w:style>", numbering), st, perl = TRUE)
  if (m < 0) return(NA_character_)
  sub(".*<w:numId w:val=\"([0-9]+)\".*", "\\1", regmatches(st, m))
}

list_ppr_for <- function(id) {
  sprintf("<w:pPr><w:pStyle w:val=\"Aufzhlungszeichen\"/><w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"%s\"/></w:numPr></w:pPr>", id)
}

#' A fresh numbering instance of the template's numbered list.
new_list <- function(ctx, proto) {
  id <- num_style_id(ctx, "nummerierteAufzhlungDossier")
  if (is.na(id)) return(NA_character_)
  nx <- read_text(file.path(ctx$tpl$dir, "word/numbering.xml"))
  ab <- first_match(sprintf("<w:num w:numId=\"%s\"[^>]*><w:abstractNumId w:val=\"([0-9]+)\"", id), nx)
  used <- as.integer(sub(".*\"([0-9]+)\"", "\\1", regmatches(nx, gregexpr("<w:num w:numId=\"[0-9]+\"", nx))[[1]]))
  num <- max(used) + 1L + length(ctx$num_new)
  ctx$num_new <- c(ctx$num_new, sprintf(
    "<w:num w:numId=\"%d\"><w:abstractNumId w:val=\"%s\"/><w:lvlOverride w:ilvl=\"0\"><w:startOverride w:val=\"1\"/></w:lvlOverride></w:num>",
    num, ab
  ))
  as.character(num)
}

# The Dossier list styles are numbering styles, not paragraph styles: the
# paragraph uses List Bullet and pulls the list definition through numPr.
style_ppr <- function(style) sprintf("<w:pPr><w:pStyle w:val=\"%s\"/></w:pPr>", style)

render_block_node <- function(node, proto, ctx) {
  f <- node$func %||% ""
  if (f == "item") {
    toks <- content_tokens(node$body, where = "Liste")
    numbered <- isTRUE(node$nummeriert) || !is.null(node$number)
    id <- if (numbered) ctx$list_num else num_style_id(ctx, "AufzhlungDossier")
    ppr <- if (is.na(id)) proto$ppr else list_ppr_for(id)
    return(render_para(proto, Filter(function(t) !identical(t$type, "pb"), toks), ctx, ppr = ppr))
  }
  if (f == "heading") {
    lvl <- node$depth %||% node$level %||% 1L
    toks <- content_tokens(node$body, where = "Überschrift")
    return(render_para(proto, toks, ctx, ppr = style_ppr(paste0("berschrift", lvl))))
  }
  if (f == "figure") {
    body <- node$body
    if (identical(body$func, "image")) {
      return(render_figure(list(bild = body, titel = node$caption$body), ctx))
    }
    if (identical(body$func, "table")) {
      t <- native_table(body)
      return(render_new_table(list(titel = node$caption$body, kopf = t$kopf, zeilen = t$zeilen), ctx))
    }
  }
  if (f == "equation") return(omml_block(node, proto$ppr))
  if (f == "image") return(render_figure(list(bild = node, titel = NULL), ctx))
  if (f == "table") {
    t <- native_table(node)
    return(render_new_table(list(titel = NULL, kopf = t$kopf, zeilen = t$zeilen), ctx))
  }
  kind <- node$kind %||% ""
  if (kind == "neue-tabelle") return(render_new_table(node, ctx))
  if (kind == "abbildung") return(render_figure(node, ctx))
  if (kind == "text") return(render_text(node, proto, ctx))
  if (kind == "tabelle") return(render_table(node, ctx))
  if (kind == "block") return(render_item(node, ctx))
  stop("Blockelement '", f, kind, "' wird nicht unterstützt.")
}

native_table <- function(node) {
  n <- node$columns
  n <- if (is.list(n)) length(n) else as.integer(n)
  kopf <- list()
  cells <- list()
  for (ch in node$children) {
    if (identical(ch$func, "header")) {
      kopf <- lapply(ch$children, function(c) c$body %||% c)
    } else {
      cells <- c(cells, list(ch$body %||% ch))
    }
  }
  rows <- split(cells, (seq_along(cells) - 1L) %/% n)
  list(kopf = kopf, zeilen = unname(rows))
}

# ---- tables -----------------------------------------------------------------

fill_cell <- function(cell, content, ctx, where) {
  toks <- content_tokens(content, where = where)
  check_atoms(toks, unlist(lapply(cell$paras, `[[`, "tokens"), recursive = FALSE), where)
  new <- split_paragraphs(toks)
  new_sig <- vapply(new, token_signature, "")
  old_sig <- vapply(cell$paras, para_sig, "")
  # Blank paragraphs in a cell are spacing, Typst has no way to express them.
  if (nzchar(cell$xml) && identical(unname(new_sig[new_sig != ""]), unname(old_sig[old_sig != ""]))) {
    return(cell$xml)
  }
  proto <- cell$paras[[1]]
  # List items belong together, so a cell with blocks renders as one stream.
  if (any(vapply(new, function(n) length(n) == 1L && identical(n[[1]]$type, "block"), NA))) {
    return(paste0(cell$open, cell$tcpr, fill_from_tokens(proto, toks, ctx), cell$close))
  }
  body <- vapply(seq_along(new), function(i) {
    pr <- if (i <= length(cell$paras)) cell$paras[[i]] else proto
    if (i <= length(old_sig) && identical(new_sig[[i]], old_sig[[i]])) {
      return(render_para(pr, pr$tokens, ctx))
    }
    render_para(pr, new[[i]], ctx)
  }, "")
  paste0(cell$open, cell$tcpr, paste(body, collapse = ""), cell$close)
}

strip_vmerge <- function(x) gsub("<w:vMerge(?: [^>]*)?/>", "", x, perl = TRUE)

render_row <- function(row, cells, ctx, where, clone = FALSE, widths = NULL) {
  n <- length(row$cells)
  xml <- vapply(seq_along(cells), function(j) {
    tc <- row$cells[[min(j, n)]]
    if (clone) {
      tc$tcpr <- strip_vmerge(tc$tcpr)
      tc$xml <- strip_vmerge(tc$xml)
      tc$open <- gsub(" w14:(paraId|textId)=\"[^\"]*\"", "", tc$open, perl = TRUE)
    }
    if (!is.null(widths)) {
      tc$tcpr <- sub("<w:tcW [^>]*/>", sprintf("<w:tcW w:w=\"%d\" w:type=\"dxa\"/>", widths[j]), tc$tcpr)
      tc$tcpr <- gsub("<w:gridSpan [^>]*/>", "", tc$tcpr)
      tc$xml <- ""  # force rebuild, the verbatim copy has the old width
    }
    out <- fill_cell(tc, cells[[j]], ctx, where)
    if (clone || !is.null(widths)) out <- clone_cell_ids(out)
    out
  }, "")
  open <- gsub(" w14:(paraId|textId)=\"[^\"]*\"", "", row$open, perl = TRUE)
  paste0(open, row$pre, paste(xml, collapse = ""), row$post, row$close)
}

clone_cell_ids <- function(x) {
  x <- gsub(" w14:(paraId|textId)=\"[^\"]*\"", "", x, perl = TRUE)
  gsub("<w:bookmark(Start|End) [^>]*/>", "", x, perl = TRUE)
}

as_rows <- function(d) {
  lapply(d, function(r) if (is.list(r)) r else as.list(r))
}

#' Column widths in dxa that keep the template table's total width.
table_widths <- function(tbl, ncol, spalten) {
  gr <- regmatches(tbl$head, gregexpr("<w:gridCol w:w=\"[0-9]+\"", tbl$head))[[1]]
  total <- sum(as.integer(gsub("[^0-9]", "", gr)))
  frac <- if (is.null(spalten)) rep(1, ncol) else as.numeric(unlist(spalten))
  w <- as.integer(round(total * frac / sum(frac)))
  w
}

render_table <- function(item, ctx) {
  id <- item$id
  b <- block_of(ctx, id)
  tbl <- b$table
  where <- paste("Tabelle", id)
  nrow_t <- length(tbl$rows)
  rows <- as_rows(item$rows %||% list())
  daten <- item$daten
  ctx$seen[[id]] <- (ctx$seen[[id]] %||% 0L) + 1L
  occ <- ctx$seen[[id]]

  if (is.null(daten)) {
    out <- vapply(seq_along(rows), function(i) {
      r <- tbl$rows[[min(i, nrow_t)]]
      if (length(rows[[i]]) != length(r$cells)) {
        stop(sprintf("%s, Zeile %d: %d Zellen, Vorlage hat %d. Für andere Spaltenzahl `daten:` mit `spalten:` nutzen.",
                     where, i, length(rows[[i]]), length(r$cells)))
      }
      if (i <= nrow_t) {
        x <- render_row(r, rows[[i]], ctx, where)
        same <- vapply(seq_along(r$cells), function(j) {
          identical(fill_cell(r$cells[[j]], rows[[i]][[j]], ctx, where), r$cells[[j]]$xml)
        }, NA)
        if (all(same)) r$xml else x
      } else {
        render_row(r, rows[[i]], ctx, where, clone = TRUE)
      }
    }, "")
    same_all <- length(rows) == nrow_t && identical(unname(out), vapply(tbl$rows, `[[`, "", "xml"))
    if (same_all) {
      ctx$stats$verbatim <- ctx$stats$verbatim + 1L
      if (occ > 1L) return(paste0(clone_xml(tbl$xml, ctx, occ), b$trail))
      return(paste0(tbl$xml, b$trail))
    }
  } else {
    kopf <- item$kopf %||% max(1L, sum(vapply(tbl$rows, `[[`, NA, "header")))
    pi <- item$zeile %||% (kopf + 1L)
    proto <- tbl$rows[[min(pi, nrow_t)]]
    data_rows <- as_rows(daten)
    nc <- length(data_rows[[1]])
    reshape <- nc != length(proto$cells) || !is.null(item$spalten) ||
      any(vapply(rows, function(r) length(r) != length(tbl$rows[[1]]$cells), NA))
    widths <- if (reshape) table_widths(tbl, nc, item$spalten)
    keep <- character()
    if (length(rows)) {
      keep <- vapply(seq_along(rows), function(i) {
        render_row(tbl$rows[[min(i, nrow_t)]], rows[[i]], ctx, where, clone = reshape, widths = widths)
      }, "")
    } else if (kopf > 0L) {
      keep <- vapply(seq_len(min(kopf, nrow_t)), function(i) {
        r <- tbl$rows[[i]]
        if (is.null(widths)) return(r$xml)
        render_row(r, lapply(r$cells, cell_content), ctx, where, clone = TRUE, widths = widths)
      }, "")
    }
    body <- vapply(data_rows, function(r) render_row(proto, r, ctx, where, clone = TRUE, widths = widths), "")
    out <- c(keep, body)
    if (!is.null(widths)) {
      tbl$head <- sub("<w:tblGrid>.*</w:tblGrid>", paste0("<w:tblGrid>",
        paste(sprintf("<w:gridCol w:w=\"%d\"/>", widths), collapse = ""), "</w:tblGrid>"), tbl$head)
    }
  }
  mark_changed(ctx, "geaendert")
  x <- paste0(tbl$open, tbl$head, paste(out, collapse = ""), tbl$close)
  if (occ > 1L) x <- clone_xml(x, ctx, occ)
  paste0(x, b$trail)
}

cell_content <- function(cell) {
  list(func = "sequence", children = list(list(func = "text", text = tokens_plain(cell$paras[[1]]$tokens))))
}

block_of <- function(ctx, id) {
  b <- ctx$tpl$blocks[[id]]
  if (is.null(b)) stop("Unbekannte Block-Kennung: ", id)
  b
}

first_caption <- function(ctx, word) {
  for (b in ctx$tpl$blocks) {
    if (b$type == "p" && grepl("Beschriftung", b$style)) {
      a <- Filter(function(t) t$type == "atom" && identical(t$kind, "caption"), b$para$tokens)
      if (length(a) && startsWith(a[[1]]$cached, word)) return(b)
    }
  }
  if (word == "Tabelle") stop("Keine Beschriftung '", word, "' in der Vorlage gefunden.")
  # Modul 3 has no figure caption to copy, so a table caption becomes one.
  b <- first_caption(ctx, "Tabelle")
  b$para$tokens <- lapply(b$para$tokens, function(t) {
    if (identical(t$kind, "caption")) t$xml <- gsub("Tabelle", word, t$xml, fixed = TRUE)
    t
  })
  style <- sub("Tabelle", word, b$para$style, fixed = TRUE)
  if (style %in% ctx$tpl$styles) b$para$ppr <- sub(b$para$style, style, b$para$ppr, fixed = TRUE)
  b
}

caption_xml <- function(titel, word, ctx) {
  b <- first_caption(ctx, word)
  a <- Filter(function(t) t$type == "atom", b$para$tokens)[[1]]
  proto <- b$para
  proto$tokens <- Filter(function(t) t$type != "mark", proto$tokens)
  toks <- c(list(list(type = "atom", key = a$key, on = NA)),
            list(plain_text_tok(": ", no_fmt)), content_tokens(titel, where = "Beschriftung"))
  render_para(proto, toks, ctx)
}

default_table <- function(ctx) {
  for (id in ctx$tpl$ids) {
    b <- ctx$tpl$blocks[[id]]
    if (b$type != "tbl" || length(b$table$rows) < 2L) next
    r2 <- b$table$rows[[2]]
    if (length(b$table$rows[[1]]$cells) == length(r2$cells) &&
        grepl("Inhalt", r2$cells[[1]]$paras[[1]]$style) &&
        !grepl("<w:shd", r2$xml, fixed = TRUE)) return(id)
  }
  ctx$tpl$ids[1]
}

render_new_table <- function(node, ctx) {
  id <- node$vorlage %||% default_table(ctx)
  tbl <- block_of(ctx, id)$table
  if (!is.null(node$tabelle)) {
    mark_changed(ctx, "neu")
    titel <- node$titel %||% node$tabelle$titel
    cap <- if (!is.null(titel)) caption_xml(titel, "Tabelle", ctx) else ""
    return(paste0(cap, spec_table(node$tabelle, tbl, ctx), "<w:p>", style_ppr("TextkrperDossier"), "</w:p>"))
  }
  kopf <- as.list(node$kopf)
  zeilen <- as_rows(node$zeilen)
  nc <- max(length(kopf), if (length(zeilen)) length(zeilen[[1]]) else 0L)
  widths <- table_widths(tbl, nc, node$spalten)
  head <- sub("<w:tblGrid>.*</w:tblGrid>", paste0("<w:tblGrid>",
    paste(sprintf("<w:gridCol w:w=\"%d\"/>", widths), collapse = ""), "</w:tblGrid>"), tbl$head)
  where <- "neue Tabelle"
  rows <- character()
  if (length(kopf)) rows <- render_row(tbl$rows[[1]], kopf, ctx, where, clone = TRUE, widths = widths)
  proto <- tbl$rows[[min(2L, length(tbl$rows))]]
  rows <- c(rows, vapply(zeilen, function(r) render_row(proto, r, ctx, where, clone = TRUE, widths = widths), ""))
  mark_changed(ctx, "neu")
  cap <- if (!is.null(node$titel)) caption_xml(node$titel, "Tabelle", ctx) else ""
  paste0(cap, tbl$open, head, paste(rows, collapse = ""), tbl$close,
         "<w:p>", style_ppr("TextkrperDossier"), "</w:p>")
}

# ---- figures ----------------------------------------------------------------

image_size <- function(path) {
  r <- readBin(path, "raw", 65536L)
  if (r[1] == as.raw(0x89)) {
    v <- as.integer(r[17:24])
    w <- sum(v[1:4] * 256^(3:0)); h <- sum(v[5:8] * 256^(3:0))
    dpi <- 96
    i <- grepRaw("pHYs", r, fixed = TRUE)
    if (length(i)) {
      p <- as.integer(r[(i + 4):(i + 7)])
      ppm <- sum(p * 256^(3:0))
      if (ppm > 0) dpi <- ppm * 0.0254
    }
    return(list(w = w, h = h, dpi = dpi, ext = "png"))
  }
  if (r[1] == as.raw(0xFF) && r[2] == as.raw(0xD8)) {
    i <- 3L
    while (i < length(r)) {
      m <- as.integer(r[i + 1L])
      len <- as.integer(r[i + 2L]) * 256L + as.integer(r[i + 3L])
      if (m >= 0xC0 && m <= 0xC3) {
        return(list(h = as.integer(r[i + 5L]) * 256L + as.integer(r[i + 6L]),
                    w = as.integer(r[i + 7L]) * 256L + as.integer(r[i + 8L]), dpi = 96, ext = "jpeg"))
      }
      i <- i + 2L + len
    }
  }
  stop("Abbildungen müssen PNG, JPEG, SVG oder PDF sein: ", path)
}

parse_length <- function(x) {
  x <- as.character(x)
  n <- as.numeric(gsub("[^0-9.]", "", x))
  u <- gsub("[0-9. ]", "", x)
  switch(u, cm = n * 360000, mm = n * 36000, "in" = n * 914400, pt = n * 12700, NA_real_)
}

# Word has no safe common format for vector figures from every R device, so
# Typst renders them to PNG at print resolution.
rasterize <- function(src, dir, ppi = 300L) {
  tmp <- tempfile("gba-bild-", tmpdir = dir, fileext = ".typ")
  on.exit(unlink(tmp))
  png <- tempfile(fileext = ".png")
  writeLines(sprintf("#set page(width: auto, height: auto, margin: 0pt)\n#image(\"%s\")", src), tmp)
  st <- system2(typst_bin(), c("compile", "--root", shQuote(dir), "--format", "png", "--ppi", ppi,
                               shQuote(tmp), shQuote(png)))
  if (st != 0L) stop("Abbildung lässt sich nicht rastern: ", src)
  attr(png, "dpi") <- ppi
  png
}

render_figure <- function(node, ctx) {
  img <- node$bild
  src <- sub("^/", "", img$source)
  path <- file.path(ctx$dir, src)
  if (!file.exists(path)) stop("Abbildung nicht gefunden: ", path)
  if (grepl("\\.(svg|pdf)$", path, ignore.case = TRUE)) path <- rasterize(src, ctx$dir)
  sz <- image_size(path)
  if (!is.null(attr(path, "dpi"))) sz$dpi <- attr(path, "dpi")
  page_w <- 5760000  # A4 text width of the templates: 16 cm
  cx <- if (!is.null(img$width)) parse_length(img$width) else NA_real_
  if (is.na(cx)) cx <- min(page_w, sz$w / sz$dpi * 914400)
  cy <- cx * sz$h / sz$w
  n <- ctx$media_base + length(ctx$media) + 1L
  name <- sprintf("gba_abbildung%d.%s", n, sz$ext)
  rid <- sprintf("rIdGba%d", n)
  ctx$media[[length(ctx$media) + 1L]] <- list(path = path, name = name, rid = rid, ext = sz$ext)
  did <- ctx$next_id()
  alt <- if (is.null(node$tag)) "" else sprintf(" descr=\"gba:%s\"", check_tag(node$tag))
  draw <- sprintf(paste0(
    "<w:r><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\">",
    "<wp:extent cx=\"%d\" cy=\"%d\"/><wp:docPr id=\"%d\" name=\"Abbildung %d\"%s/>",
    "<a:graphic xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\">",
    "<a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\">",
    "<pic:pic xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\">",
    "<pic:nvPicPr><pic:cNvPr id=\"%d\" name=\"%s\"/><pic:cNvPicPr/></pic:nvPicPr>",
    "<pic:blipFill><a:blip r:embed=\"%s\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>",
    "<pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"%d\" cy=\"%d\"/></a:xfrm>",
    "<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic>",
    "</a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"
  ), round(cx), round(cy), did, n, alt, did, name, rid, round(cx), round(cy))
  style <- if ("AbbildungDossier" %in% ctx$tpl$styles) style_ppr("AbbildungDossier") else ""
  mark_changed(ctx, "neu")
  cap <- if (!is.null(node$titel)) caption_xml(node$titel, "Abbildung", ctx) else ""
  paste0("<w:p>", style, draw, "</w:p>", cap)
}

# ---- text blocks ------------------------------------------------------------

baustein_eval <- function(name, werte) {
  out <- tempfile(fileext = ".json")
  err <- tempfile()
  st <- system2(typst_bin(), c(
    "eval", shQuote("import \"gba.typ\": baustein-inhalt; baustein-inhalt(sys.inputs.name, json(bytes(sys.inputs.werte)))"),
    "--root", shQuote(file.path(gba_root(), "typst")),
    "--input", shQuote(paste0("name=", name)),
    "--input", shQuote(paste0("werte=", jsonlite::toJSON(werte %||% list(), auto_unbox = TRUE, null = "null")))
  ), stdout = out, stderr = err)
  if (st != 0L) stop("Textbaustein ", name, ": ", paste(readLines(err), collapse = "\n"))
  jsonlite::fromJSON(read_text(out), simplifyVector = FALSE)
}

#' Tags paragraphs with a hidden bookmark each. One bookmark per paragraph, because
#' Word moves the end of a range across paragraphs to a place outside them.
tag_paras <- function(xml, tag, ctx) {
  if (!grepl("^[A-Za-z0-9_]{1,27}$", tag)) stop("Ungültige Kennung für Text: ", tag)
  rt <- ctx$tpl$root_tag
  ns <- root_ns(rt)
  b <- xml_frag(xml, rt)
  for (p in xfind(b, "w:p", ns)) {
    id <- as.character(ctx$next_id())
    ppr <- xml2::xml_find_first(p, "w:pPr", ns)
    s <- if (inherits(ppr, "xml_missing")) add_el(p, "w:bookmarkStart", ns, first = TRUE)
         else { xml2::xml_add_sibling(ppr, "w:bookmarkStart"); xml2::xml_find_first(p, "w:bookmarkStart", ns) }
    set_wattrs(s, list(id = id, name = sprintf("_gba_%s_%s", tag, id)), ns)
    set_wattrs(add_el(p, "w:bookmarkEnd", ns), list(id = id), ns)
  }
  frag_str(b, rt)
}

render_text <- function(node, proto, ctx) {
  xml <- fill_from_tokens(proto, content_tokens(node$body, where = "Textbaustein"), ctx)
  if (grepl("<w:tbl[ >]|<w:drawing>", xml, perl = TRUE)) stop("Ein Textbaustein enthält nur Text.")
  mark_changed(ctx, "neu")
  if (is.null(node$tag)) xml else tag_paras(xml, node$tag, ctx)
}

# ---- block dispatch ---------------------------------------------------------

apply_mit <- function(xml, ctx) {
  for (m in ctx$mit) xml <- replace_map(xml, m)
  xml
}

render_item <- function(item, ctx) {
  kind <- item$kind
  if (kind == "tabelle") return(apply_mit(render_table(item, ctx), ctx))
  if (kind %in% c("neue-tabelle", "abbildung")) return(apply_mit(render_block_node(item, NULL, ctx), ctx))
  id <- item$id
  b <- block_of(ctx, id)
  ctx$seen[[id]] <- (ctx$seen[[id]] %||% 0L) + 1L
  occ <- ctx$seen[[id]]
  if (kind %in% c("antwort", "fest") && is.null(item$body)) {
    ctx$stats$verbatim <- ctx$stats$verbatim + 1L
    x <- if (occ > 1L) clone_xml(b$xml, ctx, occ) else b$xml
    return(apply_mit(paste0(x, b$trail), ctx))
  }
  toks <- content_tokens(item$body, where = paste("Block", id))
  check_atoms(toks, b$para$tokens, paste("Block", id))
  if (kind == "block" && identical(token_signature(toks), token_signature(b$para$tokens)) &&
      !has_special(toks)) {
    ctx$stats$verbatim <- ctx$stats$verbatim + 1L
    x <- if (occ > 1L) clone_xml(b$xml, ctx, occ) else b$xml
    return(apply_mit(paste0(x, b$trail), ctx))
  }
  mark_changed(ctx, "geaendert")
  x <- fill_from_tokens(b$para, toks, ctx)
  if (occ > 1L) x <- clone_xml(x, ctx, occ)
  apply_mit(paste0(x, b$trail), ctx)
}

has_special <- function(toks) {
  any(vapply(toks, function(t) t$type %in% c("pb", "block", "footnote", "raw"), NA))
}

#' Compiles a Typst dossier to docx.
gba_build <- function(file, out = sub("\\.typ$", "_gefuellt.docx", file)) {
  items <- typst_query(file)
  kinds <- vapply(items, function(i) i$kind %||% "", "")
  cfg <- items[[match("konfig", kinds)]]
  proj <- dirname(normalizePath(file))
  vorlage <- file.path(proj, cfg$vorlage)
  if (!is.null(cfg$pruefsumme) && !identical(cfg$pruefsumme, unname(tools::sha256sum(vorlage)))) {
    stop(cfg$vorlage, " wurde seit dem Import geändert. Neu importieren, sonst gehen Änderungen verloren.")
  }
  tpl <- read_template(vorlage)
  on.exit(unlink(tpl$dir, recursive = TRUE))
  ctx <- new_ctx(tpl, proj)
  used <- character()
  parts <- character()
  for (it in items) {
    k <- it$kind %||% ""
    if (k == "konfig") next
    if (k == "mit-start") { ctx$mit <- c(ctx$mit, list(unlist(it$ersetzungen))); next }
    if (k == "mit-ende") { ctx$mit <- ctx$mit[-length(ctx$mit)]; next }
    if (k == "bereich") {
      rng <- tpl$ids[match(it$von, tpl$ids):match(it$bis, tpl$ids)]
      used <- c(used, rng)
      parts <- c(parts, vapply(rng, function(id) {
        b <- tpl$blocks[[id]]
        paste0(b$xml, b$trail)
      }, ""))
      ctx$stats$verbatim <- ctx$stats$verbatim + length(rng)
      next
    }
    if (!is.null(it$id)) used <- c(used, it$id)
    parts <- c(parts, render_item(it, ctx))
  }
  missing <- setdiff(tpl$ids, used)
  if (length(missing)) ctx$changed <- TRUE
  body <- paste0(tpl$lead, paste(parts, collapse = ""))
  if (ctx$changed) body <- clean_bookmarks(body)
  felder <- field_map(cfg$felder)
  if (length(felder)) body <- replace_map(body, felder)
  # Repeated blocks and `mit` replacements change the document without touching
  # a single block's signature.
  original <- paste0(tpl$lead, paste(vapply(tpl$blocks, function(b) paste0(b$xml, b$trail), ""), collapse = ""))
  ctx$changed <- ctx$changed || !identical(body, original)
  if (ctx$changed) body <- resolve_fields(body, ctx)
  xml <- paste0(tpl$head, body, tpl$final_sect, tpl$tail)
  write_package(tpl, ctx, xml, felder, out)
  rep <- gba_report(tpl, ctx, xml, missing)
  print_report(rep, out)
  invisible(rep)
}

clean_bookmarks <- function(x) {
  starts <- regmatches(x, gregexpr("<w:bookmarkStart [^>]*w:id=\"[0-9]+\"[^>]*/>", x))[[1]]
  sid <- sub(".*w:id=\"([0-9]+)\".*", "\\1", starts)
  ends <- regmatches(x, gregexpr("<w:bookmarkEnd w:id=\"[0-9]+\"/>", x))[[1]]
  eid <- sub(".*w:id=\"([0-9]+)\".*", "\\1", ends)
  for (t in unique(starts[!sid %in% eid])) x <- gsub(t, "", x, fixed = TRUE)
  for (t in unique(ends[!eid %in% sid])) x <- gsub(t, "", x, fixed = TRUE)
  x
}

gba_report <- function(tpl, ctx, xml, missing) {
  tt <- gsub("<[^>]+>", "", gsub("</w:p>", "\n", gsub("<w:instrText.*?</w:instrText>", "", drop_removed(xml), perl = TRUE), fixed = TRUE))
  tt <- xml_unescape(tt)
  offen <- regmatches(tt, gregexpr("<<[^<>]*>>", tt))[[1]]
  beispiele <- regmatches(tt, gregexpr("<[^<>\n]{2,60}>", tt))[[1]]
  geschuetzt <- vapply(missing, function(id) {
    k <- block_kind(tpl$blocks[[id]])
    k %in% c("frage", "erlaeuterung", "heading")
  }, NA)
  list(stats = ctx$stats, fehlend = missing, fehlend_pflicht = missing[geschuetzt],
       offen = table(offen), beispiele = table(beispiele))
}

print_report <- function(r, out) {
  cat(sprintf("%s: %d unverändert, %d geändert, %d neu, %d entfernt\n", basename(out),
              r$stats$verbatim, r$stats$geaendert, r$stats$neu, length(r$fehlend)))
  if (length(r$fehlend_pflicht)) {
    cat("  WARNUNG: Pflichtelemente der Vorlage fehlen:", paste(r$fehlend_pflicht, collapse = ", "), "\n")
  }
  if (length(r$offen)) cat(sprintf("  offene Platzhalter: %d Stellen\n", sum(r$offen)))
}
