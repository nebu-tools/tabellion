# Reads a G-BA .docx template into a block model.
#
# A block is one top-level child of <w:body> and keeps its verbatim XML. The
# model is derived deterministically from the docx, so ids and atom numbers
# in a generated .typ file stay valid without any sidecar file.

read_template <- function(path) {
  dir <- tempfile("gba-tpl-")
  dir.create(dir)
  order <- utils::unzip(path, list = TRUE)$Name
  utils::unzip(path, exdir = dir)
  xml <- read_text(file.path(dir, "word/document.xml"))

  raw <- as_bytes(xml)
  body_open <- regexpr("<w:body>", raw, fixed = TRUE, useBytes = TRUE)
  body_close <- regexpr("</w:body>", raw, fixed = TRUE, useBytes = TRUE)
  root_tag <- as_utf8(regmatches(raw, regexpr("<w:document[^>]*>", raw, useBytes = TRUE)))
  head <- as_utf8(substr(raw, 1L, body_open + attr(body_open, "match.length") - 1L))
  tail <- as_utf8(substr(raw, body_close, nchar(raw, "bytes")))
  sl <- xml_slice(paste0(
    "<w:body>",
    as_utf8(substr(raw, body_open + attr(body_open, "match.length"), body_close - 1L)),
    "</w:body>"
  ))
  kids <- sl$children
  nm <- elem_name(kids)

  final_sect <- ""
  if (length(kids) && nm[length(kids)] == "w:sectPr") {
    final_sect <- kids[length(kids)]
    kids <- kids[-length(kids)]
    nm <- nm[-length(nm)]
  }

  blocks <- list()
  trail <- character()
  for (i in seq_along(kids)) {
    if (nm[i] %in% c("w:p", "w:tbl")) {
      blocks[[length(blocks) + 1L]] <- list(xml = kids[i], type = sub("w:", "", nm[i]), trail = "")
    } else if (length(blocks)) {
      n <- length(blocks)
      blocks[[n]]$trail <- paste0(blocks[[n]]$trail, kids[i])
    } else {
      trail <- c(trail, kids[i])
    }
  }
  ids <- sprintf("b%04d", seq_along(blocks))
  names(blocks) <- ids

  tpl <- list(
    path = path, dir = dir, order = order, head = head, tail = tail,
    root_tag = root_tag, lead = paste(trail, collapse = ""),
    final_sect = final_sect, blocks = blocks, ids = ids
  )
  tpl$blocks <- lapply(tpl$blocks, analyse_block, tpl = tpl)
  tpl$styles <- style_ids(file.path(dir, "word/styles.xml"))
  tpl
}

style_ids <- function(path) {
  s <- read_text(path)
  m <- regmatches(s, gregexpr("w:styleId=\"[^\"]+\"", s))[[1]]
  gsub("w:styleId=\"|\"", "", m)
}

analyse_block <- function(b, tpl) {
  st <- new.env(parent = emptyenv())
  st$n <- 0L
  if (b$type == "p") {
    b$para <- parse_para(b$xml, tpl$root_tag, st)
    b$style <- b$para$style
  } else {
    b$table <- parse_table(b$xml, tpl$root_tag, st)
    b$style <- ""
  }
  b
}

parse_table <- function(xml, root_tag, st) {
  sl <- xml_slice(xml)
  nm <- elem_name(sl$children)
  head <- paste(sl$children[!nm %in% "w:tr"], collapse = "")
  rows <- lapply(sl$children[nm == "w:tr"], function(tr) {
    rs <- xml_slice(tr)
    rn <- elem_name(rs$children)
    cells <- lapply(rs$children[rn == "w:tc"], function(tc) {
      cs <- xml_slice(tc)
      cn <- elem_name(cs$children)
      paras <- lapply(cs$children[cn == "w:p"], parse_para, root_tag = root_tag, st = st)
      list(
        xml = tc, open = cs$open, close = cs$close,
        tcpr = paste(cs$children[cn != "w:p"], collapse = ""),
        paras = paras, other = any(!cn %in% c("w:p", "w:tcPr", "w:bookmarkStart", "w:bookmarkEnd"))
      )
    })
    list(
      xml = tr, open = rs$open, close = rs$close, cells = cells,
      pre = paste(rs$children[seq_len(match("w:tc", rn, nomatch = 1L) - 1L)], collapse = ""),
      post = if (any(rn == "w:tc")) paste(rs$children[seq_along(rn) > max(which(rn == "w:tc"))], collapse = "") else "",
      header = grepl("<w:tblHeader", paste(rs$children[rn == "w:trPr"], collapse = ""), fixed = TRUE)
    )
  })
  list(xml = xml, open = sl$open, close = sl$close, head = head, rows = rows)
}

# ---- block classification --------------------------------------------------

TOC_RE <- "^(Verzeichnis[1-9]|Abbildungsverzeichnis)$"

block_kind <- function(b) {
  if (b$type == "tbl") return("tabelle")
  st <- b$style
  txt <- tokens_plain(b$para$tokens)
  if (grepl("^<<\\s*Angaben.*>>$", trimws(txt))) return("antwort")
  if (grepl(TOC_RE, st)) return("toc")
  if (grepl("^berschrift[1-9]$", st)) return("heading")
  if (grepl("^berschrift|^Anhang[0-9]?Q$", st)) return("heading-frei")
  if (grepl("^Fragestellung", st)) return("frage")
  if (grepl("^Erl", st)) return("erlaeuterung")
  if (grepl("Beschriftung", st)) return("beschriftung")
  "absatz"
}
