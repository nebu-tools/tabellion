# Paragraph <-> token model.
#
# Tokens are the common language of the template side and the Typst side:
#   text : list(type="text", text=, b=, i=, sub=, sup=, u=)
#   atom : list(type="atom", key=, xml=, cached=, on=)  an opaque inline object
#          (field, footnote reference, tab, drawing, hyperlink ...)
#   mark : list(type="mark", xml=, pos="start"|"end")   bookmarks; invisible
# If both sides normalise to the same signature, the block is byte-copied.

parse_para <- function(xml, root_tag, st) {
  doc <- xml2::read_xml(paste0(root_tag, xml, "</w:document>"))
  p <- xml2::xml_child(doc)
  kids <- xml2::xml_children(p)
  nm <- xml2::xml_name(kids)
  S <- new.env(parent = emptyenv())
  S$tokens <- list()
  S$depth <- 0L
  S$buf <- character()
  S$cached <- character()
  S$instr <- character()
  S$sep <- FALSE
  S$st <- st

  ppr <- ""
  style <- ""
  for (i in seq_along(kids)) {
    k <- kids[[i]]
    n <- nm[i]
    if (n == "pPr") {
      ppr <- raw_xml(k)
      s <- xml2::xml_find_first(k, "./w:pStyle")
      if (!inherits(s, "xml_missing")) style <- xml2::xml_attr(s, "val")
    } else if (n == "r") {
      run_tokens(k, S)
    } else if (n == "proofErr") {
      next
    } else if (S$depth > 0L) {
      S$buf <- c(S$buf, raw_xml(k))
    } else if (n %in% c("bookmarkStart", "bookmarkEnd", "permStart", "permEnd", "commentRangeStart", "commentRangeEnd")) {
      push(S, list(type = "mark", xml = raw_xml(k), pos = if (n == "bookmarkEnd") "end" else "start"))
    } else {
      add_atom(S, raw_xml(k), xml2::xml_text(k), n)
    }
  }
  if (S$depth > 0L) flush_field(S)
  toks <- merge_caption(S$tokens, style, st)
  list(
    ppr = ppr, style = style, tokens = toks, open = xml_open_tag(xml),
    sect = grepl("<w:sectPr", ppr, fixed = TRUE)
  )
}

xml_open_tag <- function(x) regmatches(x, regexpr("^<[^>]*>", x))

raw_xml <- function(n) as.character(n, options = character())

push <- function(S, tok) S$tokens[[length(S$tokens) + 1L]] <- tok

add_atom <- function(S, xml, cached, kind = "") {
  key <- S$st$n
  S$st$n <- S$st$n + 1L
  on <- NA
  if (grepl("w:checkBox", xml, fixed = TRUE)) {
    on <- grepl("<w:checked/>|<w:checked w:val=\"1\"", xml) ||
      (grepl("<w:default w:val=\"1\"", xml, fixed = TRUE) && !grepl("<w:checked", xml, fixed = TRUE))
  }
  push(S, list(type = "atom", key = key, xml = xml, cached = cached, on = on, kind = kind))
}

flush_field <- function(S) {
  add_atom(S, paste(S$buf, collapse = ""), paste(S$cached, collapse = ""), paste(S$instr, collapse = ""))
  S$buf <- character()
  S$cached <- character()
  S$instr <- character()
  S$sep <- FALSE
  S$depth <- 0L
}

run_tokens <- function(r, S) {
  kids <- xml2::xml_children(r)
  nm <- xml2::xml_name(kids)
  rpr <- ""
  fmt <- list(b = FALSE, i = FALSE, sub = FALSE, sup = FALSE, u = FALSE)
  j <- which(nm == "rPr")
  if (length(j)) {
    rpr <- raw_xml(kids[[j[1]]])
    fmt <- rpr_fmt(kids[[j[1]]])
  }
  for (i in seq_along(kids)) {
    n <- nm[i]
    if (n == "rPr") next
    k <- kids[[i]]
    wrapped <- function() paste0("<w:r>", rpr, raw_xml(k), "</w:r>")
    if (n == "fldChar") {
      ty <- xml2::xml_attr(k, "fldCharType")
      if (ty == "begin") {
        S$depth <- S$depth + 1L
        S$buf <- c(S$buf, wrapped())
      } else if (ty == "separate") {
        S$sep <- TRUE
        S$buf <- c(S$buf, wrapped())
      } else {
        S$buf <- c(S$buf, wrapped())
        if (S$depth > 0L) S$depth <- S$depth - 1L
        if (S$depth == 0L) flush_field(S)
      }
      next
    }
    if (S$depth > 0L) {
      S$buf <- c(S$buf, wrapped())
      if (n == "instrText") S$instr <- c(S$instr, xml2::xml_text(k))
      if (S$sep && n == "t") S$cached <- c(S$cached, xml2::xml_text(k))
      if (S$sep && n == "noBreakHyphen") S$cached <- c(S$cached, "\u2011")
      next
    }
    if (n %in% c("t", "noBreakHyphen", "softHyphen")) {
      txt <- switch(n, t = xml2::xml_text(k), noBreakHyphen = "\u2011", softHyphen = "\u00AD")
      push(S, c(list(type = "text", text = txt, rpr = rpr), fmt))
    } else if (n == "lastRenderedPageBreak") {
      next
    } else {
      add_atom(S, wrapped(), "", n)
    }
  }
}

rpr_fmt <- function(rpr) {
  on <- function(tag) {
    x <- xml2::xml_find_first(rpr, paste0("./w:", tag))
    if (inherits(x, "xml_missing")) return(FALSE)
    v <- xml2::xml_attr(x, "val")
    is.na(v) || !v %in% c("0", "false", "none")
  }
  va <- xml2::xml_find_first(rpr, "./w:vertAlign")
  va <- if (inherits(va, "xml_missing")) "" else xml2::xml_attr(va, "val")
  list(b = on("b"), i = on("i"), u = on("u"), sub = identical(va, "subscript"), sup = identical(va, "superscript"))
}

# "Tabelle <STYLEREF>-<SEQ>" is one thing for an author, so captions show it as
# a single atom instead of a chain of field calls.
merge_caption <- function(toks, style, st) {
  if (!grepl("Beschriftung|Caption", style)) return(toks)
  vis <- vapply(toks, `[[`, "", "type")
  seqs <- which(vapply(toks, function(t) identical(t$type, "atom") && grepl("SEQ", t$kind %||% "", fixed = TRUE), NA))
  if (!length(seqs)) return(toks)
  k <- seqs[1]
  idx <- seq_len(k)
  keep <- idx[vis[idx] == "mark"]
  use <- idx[vis[idx] != "mark"]
  xml <- vapply(toks[use], function(t) {
    if (t$type == "atom") return(t$xml)
    token_run(t$text, t$rpr)
  }, "")
  cached <- vapply(toks[use], function(t) if (t$type == "atom") t$cached else t$text, "")
  first <- Filter(function(t) t$type == "atom", toks[use])[[1]]
  merged <- list(
    type = "atom", key = first$key, xml = paste(xml, collapse = ""),
    cached = paste(cached, collapse = ""), on = NA, kind = "caption"
  )
  c(toks[keep], list(merged), toks[-idx])
}

`%||%` <- function(a, b) if (is.null(a)) b else a

tokens_plain <- function(toks) {
  paste(vapply(toks, function(t) {
    if (t$type == "text") t$text else if (t$type == "atom") t$cached else ""
  }, ""), collapse = "")
}

# ---- signature --------------------------------------------------------------

fmt_code <- function(t) {
  paste0("", if (isTRUE(t$b)) "b", if (isTRUE(t$i)) "i", if (isTRUE(t$u)) "u",
         if (isTRUE(t$sub)) "v", if (isTRUE(t$sup)) "^")
}

#' Collapses a token list to segments the way Typst sees text: runs merged,
#' whitespace collapsed, whitespace never formatted, ends trimmed.
normalise_tokens <- function(toks) {
  segs <- list()
  add <- function(text, code) {
    n <- length(segs)
    if (n && segs[[n]]$kind == "text" && identical(segs[[n]]$code, code)) {
      segs[[n]]$text <<- paste0(segs[[n]]$text, text)
    } else {
      segs[[n + 1L]] <<- list(kind = "text", text = text, code = code)
    }
  }
  for (t in toks) {
    if (t$type == "mark") next
    if (t$type == "atom") {
      segs[[length(segs) + 1L]] <- list(kind = "atom", key = t$key, on = t$on)
      next
    }
    if (t$type != "text") {
      # Formulas and new footnotes have no template counterpart.
      segs[[length(segs) + 1L]] <- list(kind = "atom", key = paste0("neu-", t$type), on = NA)
      next
    }
    text <- gsub("[ \t\r\n]+", " ", t$text)
    code <- fmt_code(t)
    if (code == "") {
      add(text, "")
    } else {
      m <- regmatches(text, regexec("^( ?)(.*?)( ?)$", text, perl = TRUE))[[1]]
      if (nzchar(m[2])) add(" ", "")
      if (nzchar(m[3])) add(m[3], code)
      if (nzchar(m[4])) add(" ", "")
    }
  }
  out <- list()
  for (s in segs) {
    n <- length(out)
    if (s$kind == "text" && n && out[[n]]$kind == "text" && out[[n]]$code == s$code) {
      out[[n]]$text <- paste0(out[[n]]$text, s$text)
    } else {
      out[[n + 1L]] <- s
    }
  }
  # "**a** **b**" and "**a b**" read the same; formatted spaces are invisible.
  i <- 2L
  while (i < length(out)) {
    a <- out[[i - 1L]]; m <- out[[i]]; z <- out[[i + 1L]]
    if (m$kind == "text" && a$kind == "text" && z$kind == "text" && m$code == "" &&
        grepl("^ +$", m$text) && a$code == z$code) {
      out[[i - 1L]]$text <- paste0(a$text, " ", z$text)
      out <- out[-c(i, i + 1L)]
    } else {
      i <- i + 1L
    }
  }
  for (i in seq_along(out)) {
    if (out[[i]]$kind == "text") out[[i]]$text <- gsub(" +", " ", out[[i]]$text)
  }
  if (length(out) && out[[1]]$kind == "text") out[[1]]$text <- sub("^ +", "", out[[1]]$text)
  n <- length(out)
  if (n && out[[n]]$kind == "text") out[[n]]$text <- sub(" +$", "", out[[n]]$text)
  Filter(function(s) s$kind == "atom" || nzchar(s$text), out)
}

token_signature <- function(toks) {
  paste(vapply(normalise_tokens(toks), function(s) {
    if (s$kind == "atom") {
      paste0("\u0001", s$key, if (isTRUE(s$on)) "x" else "")
    } else {
      paste0("\u0002", s$code, "\u0003", s$text)
    }
  }, ""), collapse = "")
}

# ---- rendering --------------------------------------------------------------

token_run <- function(text, rpr) {
  # U+2011 and U+00AD are elements in WordprocessingML, not characters.
  parts <- regmatches(text, gregexpr("[\u2011\u00AD]|[^\u2011\u00AD]+", text, perl = TRUE))[[1]]
  body <- vapply(parts, function(p) {
    if (p == "\u2011") return("<w:noBreakHyphen/>")
    if (p == "\u00AD") return("<w:softHyphen/>")
    paste0("<w:t xml:space=\"preserve\">", xml_escape(p), "</w:t>")
  }, "", USE.NAMES = FALSE)
  paste0("<w:r>", rpr, paste(body, collapse = ""), "</w:r>")
}

strip_rpr <- function(rpr, tags = c("b", "bCs", "i", "iCs", "u", "vertAlign", "highlight")) {
  for (t in tags) {
    rpr <- gsub(paste0("<w:", t, "(?: [^>]*)?/>"), "", rpr, perl = TRUE)
  }
  if (rpr == "<w:rPr></w:rPr>") "" else rpr
}

#' Direct run formatting the template uses for ordinary text in this paragraph.
base_rpr <- function(toks) {
  tx <- Filter(function(t) t$type == "text" && nzchar(t$rpr %||% ""), toks)
  plain <- Filter(function(t) !grepl("w:highlight", t$rpr, fixed = TRUE), tx)
  use <- if (length(plain)) plain else tx
  if (!length(use)) return("")
  strip_rpr(use[[1]]$rpr)
}

apply_fmt <- function(rpr, t) {
  add <- paste0(
    "", if (isTRUE(t$b)) "<w:b/><w:bCs/>", if (isTRUE(t$i)) "<w:i/><w:iCs/>",
    if (isTRUE(t$u)) "<w:u w:val=\"single\"/>",
    if (isTRUE(t$sub)) "<w:vertAlign w:val=\"subscript\"/>",
    if (isTRUE(t$sup)) "<w:vertAlign w:val=\"superscript\"/>"
  )
  if (add == "") return(rpr)
  if (rpr == "") return(paste0("<w:rPr>", add, "</w:rPr>"))
  # CT_RPr is a sequence: vertAlign must follow the rest, b/i precede colour.
  lead <- gsub("<w:vertAlign[^>]*>", "", add)
  tailpart <- paste(regmatches(add, gregexpr("<w:vertAlign[^>]*>", add))[[1]], collapse = "")
  body <- sub("^<w:rPr>", "", sub("</w:rPr>$", "", rpr))
  style <- regmatches(body, regexpr("^<w:(rStyle|rFonts)[^>]*/>(<w:rFonts[^>]*/>)?", body))
  style <- if (length(style)) style else ""
  rest <- substr(body, nchar(style) + 1L, nchar(body))
  paste0("<w:rPr>", style, lead, rest, tailpart, "</w:rPr>")
}

checkbox_xml <- function(xml, on) {
  xml <- gsub("<w:checked(?: [^>]*)?/>", "", xml, perl = TRUE)
  if (isTRUE(on)) {
    xml <- sub("(<w:default [^>]*/>)", "\\1<w:checked/>", xml, perl = TRUE)
  } else {
    xml <- sub("<w:default w:val=\"1\"/>", "<w:default w:val=\"0\"/>", xml, fixed = TRUE)
  }
  xml
}

#' Builds a <w:p> from tokens, keeping pPr and bookmarks of the prototype.
render_para <- function(proto, toks, ctx, ppr = proto$ppr) {
  blank <- function(t) t$type == "text" && !nzchar(trimws(t$text))
  while (length(toks) && blank(toks[[1]])) toks <- toks[-1L]
  while (length(toks) && blank(toks[[length(toks)]])) toks <- toks[-length(toks)]
  base <- base_rpr(proto$tokens)
  marks <- Filter(function(t) t$type == "mark", proto$tokens)
  atoms <- Filter(function(t) t$type == "atom", proto$tokens)
  keys <- vapply(atoms, function(a) as.character(a$key), "")
  body <- vapply(toks, function(t) {
    if (t$type == "text") return(token_run(t$text, apply_fmt(base, t)))
    if (t$type == "atom") {
      a <- ctx$atom(t$key, atoms, keys)
      return(if (is.na(a$on)) a$xml else checkbox_xml(a$xml, t$on))
    }
    if (t$type == "raw") return(t$xml)
    if (t$type == "footnote") return(ctx$footnote(t$body))
    ""
  }, "")
  ms <- Filter(function(t) t$pos == "start", marks)
  me <- Filter(function(t) t$pos == "end", marks)
  paste0(
    "<w:p>", ppr,
    paste(vapply(ms, `[[`, "", "xml"), collapse = ""),
    paste(body, collapse = ""),
    paste(vapply(me, `[[`, "", "xml"), collapse = ""),
    "</w:p>"
  )
}
