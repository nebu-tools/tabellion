# Writes the field results Word would store when saving: table and figure
# numbers, cross references and the text of all tables of contents. Page numbers
# need a layout engine; TOC entries keep the last known page until F9 in Word.

para_text <- function(x) {
  x <- gsub("<w:instrText.*?</w:instrText>", "", x, perl = TRUE)
  x <- gsub("<w:noBreakHyphen/>", "‑", x, fixed = TRUE)
  x <- gsub("<w:softHyphen/>", "­", x, fixed = TRUE)
  x <- gsub("<w:tab/>", "\t", x, fixed = TRUE)
  x <- gsub("<w:br/>", " ", x, fixed = TRUE)
  xml_unescape(gsub("<[^>]+>", "", x))
}

first_match <- function(pat, x, group = 1L) {
  m <- regmatches(x, regexec(pat, x, perl = TRUE))[[1]]
  if (length(m) > group) m[group + 1L] else NA_character_
}

# ---- styles and numbering ----------------------------------------------------

read_styles <- function(dir) {
  s <- read_text(file.path(dir, "word/styles.xml"))
  items <- regmatches(s, gregexpr("<w:style [^>]*w:type=\"paragraph\".*?</w:style>", s, perl = TRUE))[[1]]
  out <- lapply(items, function(x) list(
    id = first_match("w:styleId=\"([^\"]+)\"", x),
    name = first_match("<w:name w:val=\"([^\"]+)\"", x),
    based = first_match("<w:basedOn w:val=\"([^\"]+)\"", x),
    num = first_match("<w:numId w:val=\"([0-9]+)\"", x),
    lvl = first_match("<w:numPr>(?:(?!</w:numPr>).)*<w:ilvl w:val=\"([0-9]+)\"", x),
    outline = first_match("<w:outlineLvl w:val=\"([0-9]+)\"", x)
  ))
  names(out) <- vapply(out, `[[`, "", "id")
  out
}

style_prop <- function(st, id, prop) {
  while (!is.na(id) && !is.null(st[[id]])) {
    v <- st[[id]][[prop]]
    if (!is.na(v)) return(v)
    id <- st[[id]]$based
  }
  NA_character_
}

parse_levels <- function(x) {
  ls <- regmatches(x, gregexpr("<w:lvl .*?</w:lvl>", x, perl = TRUE))[[1]]
  out <- list()
  for (l in ls) {
    i <- as.integer(first_match("w:ilvl=\"([0-9]+)\"", l))
    out[[i + 1L]] <- list(
      start = as.integer(first_match("<w:start w:val=\"([0-9]+)\"", l)),
      fmt = first_match("<w:numFmt w:val=\"([^\"]+)\"", l),
      text = first_match("<w:lvlText w:val=\"([^\"]*)\"", l),
      style = first_match("<w:pStyle w:val=\"([^\"]+)\"", l)
    )
  }
  out
}

#' Level definitions per numId. An abstractNum may only link to a numbering
#' style, and a num may redefine levels through lvlOverride.
read_numbering <- function(dir) {
  s <- read_text(file.path(dir, "word/numbering.xml"))
  st <- read_text(file.path(dir, "word/styles.xml"))
  abs <- regmatches(s, gregexpr("<w:abstractNum .*?</w:abstractNum>", s, perl = TRUE))[[1]]
  names(abs) <- vapply(abs, function(a) first_match("w:abstractNumId=\"([0-9]+)\"", a), "", USE.NAMES = FALSE)
  nums <- regmatches(s, gregexpr("<w:num .*?</w:num>", s, perl = TRUE))[[1]]
  names(nums) <- vapply(nums, function(n) first_match("w:numId=\"([0-9]+)\"", n), "", USE.NAMES = FALSE)
  abstract_of <- function(n) first_match("<w:abstractNumId w:val=\"([0-9]+)\"", n)
  resolve <- function(ab) {
    link <- first_match("<w:numStyleLink w:val=\"([^\"]+)\"", abs[[ab]])
    if (is.na(link)) return(ab)
    id <- first_match(sprintf("w:styleId=\"%s\".*?<w:numId w:val=\"([0-9]+)\"", link), st)
    abstract_of(nums[[id]])
  }
  out <- lapply(nums, function(n) {
    ab <- resolve(abstract_of(n))
    lv <- parse_levels(abs[[ab]])
    over <- parse_levels(n)
    for (i in seq_along(over)) if (!is.null(over[[i]])) lv[[i]] <- over[[i]]
    list(abstract = ab, levels = lv)
  })
  out
}

fmt_count <- function(n, fmt) {
  if (is.na(n)) return("")
  if (identical(fmt, "upperLetter")) return(LETTERS[n])
  if (identical(fmt, "lowerLetter")) return(letters[n])
  as.character(n)
}

# Number strings of paragraph-style numbering, in document order.
new_counter <- function(st, nm) {
  env <- new.env(parent = emptyenv())
  env$c <- list()
  function(style, para) {
    ppr <- first_match("(<w:pPr>.*?</w:pPr>)", para)
    if (is.na(ppr)) ppr <- ""
    num <- first_match("<w:numPr>(?:(?!</w:numPr>).)*<w:numId w:val=\"([0-9]+)\"", ppr)
    direct <- !is.na(num)
    if (!direct) num <- style_prop(st, style, "num")
    if (is.na(num) || num == "0") return("")
    def <- nm[[num]]
    if (is.null(def)) return("")
    ab <- def$abstract
    lv <- def$levels
    i <- if (direct) as.integer(first_match("<w:ilvl w:val=\"([0-9]+)\"", ppr)) else NA_integer_
    if (is.na(i)) {
      linked <- which(vapply(lv, function(l) identical(l$style, style), NA))
      i <- if (length(linked)) linked[1] - 1L else as.integer(style_prop(st, style, "lvl"))
    }
    if (is.na(i)) i <- 0L
    if (i >= length(lv)) return("")
    cur <- env$c[[ab]] %||% rep(NA_integer_, length(lv))
    cur[i + 1L] <- if (is.na(cur[i + 1L])) lv[[i + 1L]]$start else cur[i + 1L] + 1L
    if (i + 1L < length(cur)) cur[(i + 2L):length(cur)] <- NA_integer_
    env$c[[ab]] <- cur
    out <- lv[[i + 1L]]$text
    if (is.na(out)) return("")
    for (k in seq_len(i + 1L)) {
      v <- if (is.na(cur[k])) lv[[k]]$start else cur[k]
      out <- gsub(paste0("%", k), fmt_count(v, lv[[k]]$fmt), out, fixed = TRUE)
    }
    out
  }
}

# ---- simple fields -----------------------------------------------------------

t_xml <- function(value) {
  # U+2011 and U+00AD are elements in runs, not characters.
  v <- gsub("\u2011", "</w:t><w:noBreakHyphen/><w:t xml:space=\"preserve\">", xml_escape(value), fixed = TRUE)
  v <- gsub("\u00AD", "</w:t><w:softHyphen/><w:t xml:space=\"preserve\">", v, fixed = TRUE)
  paste0("<w:t xml:space=\"preserve\">", v, "</w:t>")
}

set_cached <- function(xml, s, e, value) {
  # Puts `value` in the first w:t of xml[s:e] and empties the others.
  seg <- gsub("<w:(noBreakHyphen|softHyphen)/>", "", as_utf8(substr(xml, s, e)), perl = TRUE)
  m <- gregexpr("<w:t(?: [^>]*)?>[^<]*</w:t>", seg, perl = TRUE)[[1]]
  if (m[1] < 0) return(xml)
  new <- rep("<w:t></w:t>", length(m))
  new[1] <- t_xml(value)
  regmatches(seg, list(m)) <- list(new)
  enc2utf8(paste0(as_utf8(substr(xml, 1L, s - 1L)), seg, as_utf8(substr(xml, e + 1L, nchar(xml, "bytes")))))
}

#' Applies f(code) to every field of a kind in a block. Word may split one
#' instruction over several runs, so the code is joined first.
map_fields <- function(xml, kind, f) {
  x <- as_bytes(xml)
  head <- paste0("^\\s*", kind, "\\b")
  edits <- list()
  begins <- gregexpr("w:fldCharType=\"begin\"", x, fixed = TRUE, useBytes = TRUE)[[1]]
  for (b in begins[begins > 0]) {
    rest <- substr(x, b, nchar(x, "bytes"))
    sep <- regexpr("w:fldCharType=\"separate\"", rest, fixed = TRUE, useBytes = TRUE)
    if (sep < 0) next
    instr <- substr(rest, 1L, sep)
    ts <- regmatches(instr, gregexpr("<w:instrText[^>]*>[^<]*", instr, useBytes = TRUE))[[1]]
    code <- paste(sub("<[^>]*>", "", ts), collapse = "")
    if (!grepl(head, code, perl = TRUE, useBytes = TRUE)) next
    end <- regexpr("w:fldCharType=\"end\"", rest, fixed = TRUE, useBytes = TRUE)
    if (end < sep) next
    v <- f(trimws(sub(head, "", as_utf8(code), perl = TRUE)))
    if (!is.null(v)) edits[[length(edits) + 1L]] <- list(s = b + sep - 1L, e = b + end - 2L, v = v)
  }
  for (ed in rev(edits)) {
    x <- as_bytes(set_cached(x, ed$s, ed$e, ed$v))
  }
  xml <- as_utf8(x)
  re2 <- sprintf("<w:fldSimple w:instr=\"\\s*%s\\b[^\"]*\"[^>]*>.*?</w:fldSimple>", kind)
  m <- gregexpr(re2, xml, perl = TRUE)[[1]]
  if (m[1] > 0) {
    hit <- regmatches(xml, list(m))[[1]]
    for (i in seq_along(hit)) {
      code <- trimws(sub(paste0("^\\s*", kind), "", first_match("w:instr=\"([^\"]*)\"", hit[i]), perl = TRUE))
      v <- f(code)
      if (!is.null(v)) hit[i] <- set_cached(hit[i], 1L, nchar(hit[i]), v)
    }
    regmatches(xml, list(m)) <- list(hit)
  }
  xml
}

# ---- bookmarks ---------------------------------------------------------------

bookmark_table <- function(blocks, numbers) {
  out <- list()
  for (i in seq_along(blocks)) {
    b <- blocks[i]
    if (!grepl("<w:bookmarkStart", b, fixed = TRUE)) next
    paras <- regmatches(b, gregexpr("<w:p[ >].*?</w:p>", b, perl = TRUE))[[1]]
    for (p in paras) {
      st <- regmatches(p, gregexpr("<w:bookmarkStart [^>]*>", p))[[1]]
      for (t in st) {
        nm <- first_match("w:name=\"([^\"]+)\"", t)
        id <- first_match("w:id=\"([0-9]+)\"", t)
        from <- regexpr(t, p, fixed = TRUE) + nchar(t)
        rest <- substr(p, from, nchar(p))
        e <- regexpr(sprintf("<w:bookmarkEnd w:id=\"%s\"/>", id), rest)
        seg <- if (e > 0) substr(rest, 1L, e - 1L) else rest
        out[[nm]] <- list(text = para_text(seg), number = numbers[i], block = i,
                          footnote = first_match("<w:footnoteReference [^>]*w:id=\"([0-9]+)\"", seg))
      }
    }
  }
  out
}

# ---- tables of contents ------------------------------------------------------

toc_spec <- function(code, st) {
  cap <- first_match("\\\\c \"([^\"]+)\"", code)
  if (!is.na(cap)) return(list(caption = cap))
  tt <- first_match("\\\\t \"([^\"]+)\"", code)
  # K3: \\t with a style name and no level lists the paragraphs of that style.
  if (!is.na(tt) && !grepl(";", tt, fixed = TRUE)) {
    id <- Filter(function(i) identical(st[[i]]$name, tt), names(st))
    return(list(style = id))
  }
  lo <- as.integer(first_match("\\\\o \"([0-9]+)-", code))
  hi <- as.integer(first_match("\\\\o \"[0-9]+-([0-9]+)", code))
  map <- list()
  if (!is.na(tt)) {
    p <- strsplit(tt, ";", fixed = TRUE)[[1]]
    for (i in seq(1L, length(p) - 1L, by = 2L)) map[[trimws(p[i])]] <- as.integer(p[i + 1L])
  }
  lv <- list()
  ids <- names(st)
  # \\o takes everything at these outline levels, also styles derived from a
  # heading such as the K-module annex headings, which inherit the level.
  for (id in ids) {
    ol <- as.integer(style_prop(st, id, "outline"))
    if (!is.na(ol) && ol + 1L >= lo && ol + 1L <= hi) lv[[id]] <- ol + 1L
  }
  for (id in ids) {
    nm <- st[[id]]$name
    h <- first_match("^heading ([0-9])$", nm)
    alt <- if (!is.na(h)) paste("Überschrift", h) else nm
    for (k in c(nm, alt)) if (!is.null(map[[k]])) lv[[id]] <- map[[k]]
  }
  list(levels = lv)
}

# Instructions can be split over runs, so this reads the joined text.
is_seq <- function(block, id) {
  if (grepl(paste0("w:instr=\"\\s*SEQ ", id), block)) return(TRUE)
  ts <- regmatches(block, gregexpr("<w:instrText[^>]*>[^<]*", block))[[1]]
  grepl(paste0("SEQ ", id), paste(sub("<[^>]*>", "", ts), collapse = ""), fixed = TRUE)
}

TOC_LEER <- "Es konnten keine Einträge für ein Abbildungsverzeichnis gefunden werden."

entry_para <- function(protos, lvl_style, anchor, number, title, page) {
  has_num <- nzchar(number)
  key <- paste(lvl_style, has_num)
  p <- protos[[key]] %||% protos[[paste(lvl_style, !has_num)]] %||% protos[[1]]
  a <- first_match("w:anchor=\"([^\"]+)\"", p)
  p <- gsub(a, anchor, p, fixed = TRUE)
  m <- gregexpr("<w:t(?: [^>]*)?>[^<]*</w:t>", p, perl = TRUE)[[1]]
  vals <- if (has_num && length(m) >= 3L) {
    c(number, title, rep("", length(m) - 3L), page)
  } else {
    c(if (has_num) paste(number, title) else title, rep("", length(m) - 2L), page)
  }
  p <- gsub("<w:(noBreakHyphen|softHyphen)/>", "", p, perl = TRUE)
  m <- gregexpr("<w:t(?: [^>]*)?>[^<]*</w:t>", p, perl = TRUE)[[1]]
  regmatches(p, list(m)) <- list(vapply(vals, t_xml, "", USE.NAMES = FALSE))
  gsub(" w14:(paraId|textId)=\"[^\"]*\"", "", p, perl = TRUE)
}

ensure_toc_mark <- function(block, ctx) {
  nm <- first_match("<w:bookmarkStart [^>]*w:name=\"(_Toc[^\"]+)\"", block)
  if (!is.na(nm)) return(list(block = block, name = nm))
  id <- ctx$next_id()
  nm <- sprintf("_Toc9%08d", id)
  s <- sprintf("<w:bookmarkStart w:id=\"%d\" w:name=\"%s\"/>", id, nm)
  e <- sprintf("<w:bookmarkEnd w:id=\"%d\"/>", id)
  if (grepl("</w:pPr>", block, fixed = TRUE)) {
    block <- sub("</w:pPr>", paste0("</w:pPr>", s), block, fixed = TRUE)
  } else {
    block <- sub("(<w:p(?: [^>]*)?>)", paste0("\\1", s), block, perl = TRUE)
  }
  block <- sub("</w:p>$", paste0(e, "</w:p>"), block, perl = TRUE)
  list(block = block, name = nm)
}

#' One entry per style, with and without a number, to copy from.
toc_protos <- function(paras) {
  protos <- list()
  for (x in paras) {
    h <- regmatches(x, regexpr("<w:hyperlink .*</w:hyperlink>", x, perl = TRUE))
    if (!length(h)) next
    ts <- regmatches(x, gregexpr("<w:t(?: [^>]*)?>[^<]*</w:t>", x, perl = TRUE))[[1]]
    ppr <- first_match("(<w:pPr>.*?</w:pPr>)", x)
    key <- paste(first_match("<w:pStyle w:val=\"([^\"]+)\"", x), length(ts) >= 3L)
    if (is.null(protos[[key]])) protos[[key]] <- paste0("<w:p>", if (is.na(ppr)) "" else ppr, h, "</w:p>")
  }
  protos
}

rebuild_tocs <- function(blocks, styles, numbers, ctx) {
  is_toc_style <- grepl(TOC_RE, styles)
  starts <- which(grepl("<w:instrText[^>]*>\\s*TOC\\b", blocks, perl = TRUE))
  # A list that starts empty has no entry to copy, so it borrows from another.
  fallback <- toc_protos(blocks[is_toc_style])
  for (s in rev(starts)) {
    code <- trimws(first_match("<w:instrText[^>]*>\\s*(TOC[^<]*)<", blocks[s]))
    e <- s
    while (e < length(blocks) && is_toc_style[e + 1L]) e <- e + 1L
    old <- blocks[s:e]
    spec <- toc_spec(code, ctx$st)
    pages <- list()
    for (x in old) {
      a <- first_match("w:anchor=\"([^\"]+)\"", x)
      if (is.na(a)) next
      ts <- regmatches(x, gregexpr("<w:t(?: [^>]*)?>[^<]*</w:t>", x, perl = TRUE))[[1]]
      pages[[a]] <- gsub("<[^>]+>", "", ts[length(ts)])
    }
    protos <- toc_protos(old)
    empty <- !length(protos)
    if (empty) protos <- fallback
    if (!length(protos)) next
    wanted <- list()
    last_page <- "1"
    for (i in seq_along(blocks)) {
      if (i >= s && i <= e) next
      if (is_toc_style[i] || !startsWith(blocks[i], "<w:p")) next
      lvl <- NA_integer_
      if (!is.null(spec$caption)) {
        if (is_seq(blocks[i], spec$caption)) lvl <- 0L
      } else if (!is.null(spec$style)) {
        if (styles[i] %in% spec$style) lvl <- 0L
      } else if (!is.null(spec$levels[[styles[i]]])) {
        lvl <- spec$levels[[styles[i]]]
      }
      if (is.na(lvl)) next
      title <- trimws(para_text(blocks[i]))
      if (!nzchar(title)) next
      r <- ensure_toc_mark(blocks[i], ctx)
      blocks[i] <- r$block
      pg <- pages[[r$name]] %||% last_page
      last_page <- pg
      sty <- if (lvl == 0L) "Abbildungsverzeichnis" else paste0("Verzeichnis", lvl)
      wanted[[length(wanted) + 1L]] <- entry_para(protos, sty, r$name, numbers[i], title, pg)
    }
    if (empty && !length(wanted)) next
    pre <- first_match("^<w:p(?: [^>]*)?>(?:<w:pPr>.*?</w:pPr>)?(.*?)<w:hyperlink ", old[1])
    # The TOC field usually ends in a trailing paragraph without an entry; that
    # paragraph carries the closing fldChar and has to stay.
    last <- old[length(old)]
    if (empty) {
      # The only paragraph holds Word's empty message between the field marks.
      pre <- first_match("^<w:p(?: [^>]*)?>(?:<w:pPr>.*?</w:pPr>)?(.*?w:fldCharType=\"separate\"/></w:r>)", old[1])
      post <- first_match("(<w:r(?: [^>]*)?>(?:(?!</w:r>).)*w:fldCharType=\"end\"/></w:r>)</w:p>$", last)
      keep <- character()
    } else if (grepl("<w:hyperlink ", last, fixed = TRUE)) {
      post <- first_match("</w:hyperlink>((?:(?!</w:hyperlink>).)*)</w:p>$", last)
      keep <- character()
    } else {
      post <- ""
      keep <- last
    }
    if (!length(wanted)) {
      ppr <- first_match("(<w:pPr>.*?</w:pPr>)", old[1])
      wanted <- list(paste0("<w:p>", if (is.na(ppr)) "" else ppr, pre, "<w:r><w:rPr><w:b/><w:bCs/></w:rPr>",
                            t_xml(TOC_LEER), "</w:r>", post, "</w:p>"))
      pre <- ""
      post <- ""
    }
    n <- length(wanted)
    wanted[[1]] <- sub("(<w:hyperlink )", paste0(gsub("\\", "\\\\", pre, fixed = TRUE), "\\1"), wanted[[1]], perl = TRUE)
    wanted[[n]] <- sub("</w:p>$", paste0(gsub("\\", "\\\\", post, fixed = TRUE), "</w:p>"), wanted[[n]], perl = TRUE)
    wanted <- c(wanted, keep)
    n <- length(wanted)
    blocks <- c(blocks[seq_len(s - 1L)], unlist(wanted), if (e < length(blocks)) blocks[(e + 1L):length(blocks)])
    styles <- c(styles[seq_len(s - 1L)], rep("Verzeichnis1", n), if (e < length(styles)) styles[(e + 1L):length(styles)])
    numbers <- c(numbers[seq_len(s - 1L)], rep("", n), if (e < length(numbers)) numbers[(e + 1L):length(numbers)])
    is_toc_style <- grepl(TOC_RE, styles)
  }
  blocks
}

#' Field results for a composed document body.
resolve_fields <- function(body, ctx) {
  dir <- ctx$tpl$dir
  ctx$st <- read_styles(dir)
  counter <- new_counter(ctx$st, read_numbering(dir))
  blocks <- xml_slice(paste0("<w:body>", body, "</w:body>"))$children
  styles <- vapply(blocks, function(b) {
    if (!startsWith(b, "<w:p")) return("")
    v <- first_match("^<w:p(?: [^>]*)?><w:pPr><w:pStyle w:val=\"([^\"]+)\"", substr(b, 1L, 400L))
    if (is.na(v)) "" else v
  }, "", USE.NAMES = FALSE)
  numbers <- vapply(seq_along(blocks), function(i) {
    if (!nzchar(styles[i]) || grepl(TOC_RE, styles[i])) return("")
    counter(styles[i], substr(blocks[i], 1L, 800L))
  }, "")

  has_field <- grepl("<w:instrText|<w:fldSimple", blocks)
  seqs <- list()
  for (i in which(has_field)) {
    blocks[i] <- map_fields(blocks[i], "SEQ", function(code) {
      id <- strsplit(code, "\\s+")[[1]][1]
      seqs[[id]] <<- (seqs[[id]] %||% 0L) + 1L
      as.character(seqs[[id]])
    })
  }
  bm <- bookmark_table(blocks, numbers)
  # Numbers are not part of a bookmark's text; \r and friends ask for them.
  for (i in which(has_field & !grepl(TOC_RE, styles))) {
    blocks[i] <- map_fields(blocks[i], "REF", function(code) {
      nm <- strsplit(code, "\\s+")[[1]][1]
      t <- bm[[nm]]
      if (is.null(t)) return(NULL)
      # Word writes 0 for a number reference to an unnumbered paragraph.
      if (grepl("\\\\[rnw]\\b", code)) return(if (nzchar(t$number)) t$number else "0")
      t$text
    })
  }
  fn <- regmatches(body, gregexpr("<w:footnoteReference [^>]*w:id=\"[0-9]+\"", body))[[1]]
  fn <- sub(".*w:id=\"([0-9]+)\"", "\\1", fn)
  for (i in which(has_field)) {
    blocks[i] <- map_fields(blocks[i], "NOTEREF", function(code) {
      t <- bm[[strsplit(code, "\\s+")[[1]][1]]]
      if (is.null(t) || is.na(t$footnote)) return(NULL)
      as.character(match(t$footnote, fn))
    })
  }
  paste(rebuild_tocs(blocks, styles, numbers, ctx), collapse = "")
}
