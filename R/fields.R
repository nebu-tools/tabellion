# Placeholder replacement that survives Word splitting text across runs.

FELD_MUSTER <- list(
  wirkstoff = "<<\\s*Wirkstoff\\s*>>",
  handelsname = "<<\\s*Handelsname\\s*>>",
  unternehmer = "<<\\s*Pharmazeutischer Unternehmer\\s*>>",
  kodierung = "<<\\s*Kodierung A.Z\\s*>>",
  anwendungsgebiet = "<<\\s*Anwendungsgebiet\\s*>>",
  stand = "<<\\s*(?i:tt\\.mm\\.jjjj|xx\\.mm\\.jjjj)\\s*>>"
)

field_map <- function(felder) {
  if (is.null(felder) || !length(felder)) return(character())
  unk <- setdiff(names(felder), names(FELD_MUSTER))
  if (length(unk)) stop("Unbekannte Felder: ", paste(unk, collapse = ", "))
  v <- unlist(felder)
  names(v) <- unlist(FELD_MUSTER[names(felder)])
  attr(v, "regex") <- TRUE
  v
}

#' Applies pattern -> replacement to the text of each innermost paragraph.
replace_map <- function(xml, map) {
  regex <- isTRUE(attr(map, "regex"))
  pats <- names(map)
  if (!regex) pats <- gsub("([.^$|()\\[\\]{}*+?\\\\])", "\\\\\\1", pats, perl = TRUE)
  orig <- xml
  xml <- as_bytes(enc2utf8(xml))
  m <- gregexpr("<w:p[ >]|<w:p/>|</w:p>|<w:t(?: [^>]*)?>[^<]*</w:t>", xml, perl = TRUE, useBytes = TRUE)[[1]]
  if (m[1] < 0) return(orig)
  st <- as.integer(m)
  en <- st + attr(m, "match.length") - 1L
  tg <- as_utf8(substring(xml, st, en))
  kind <- ifelse(startsWith(tg, "</w:p"), "c", ifelse(startsWith(tg, "<w:t"), "t", ifelse(endsWith(tg, "/>"), "e", "o")))
  stack <- integer()
  pid <- 0L
  owner <- integer(length(tg))
  for (i in seq_along(tg)) {
    k <- kind[i]
    if (k == "o") { pid <- pid + 1L; stack <- c(stack, pid) }
    else if (k == "c") stack <- stack[-length(stack)]
    else if (k == "t" && length(stack)) owner[i] <- stack[length(stack)]
  }
  ti <- which(kind == "t" & owner > 0L)
  groups <- split(ti, owner[ti])
  edits <- list()
  for (g in groups) {
    parts <- xml_unescape(sub("</w:t>$", "", sub("^<w:t[^>]*>", "", tg[g])))
    joined <- paste(parts, collapse = "")
    hit <- FALSE
    for (p in seq_along(map)) {
      if (grepl(pats[p], joined, perl = TRUE)) hit <- TRUE
    }
    if (!hit) next
    for (p in seq_along(map)) {
      repeat {
        joined <- paste(parts, collapse = "")
        h <- regexpr(pats[p], joined, perl = TRUE)
        if (h < 0) break
        a <- as.integer(h)
        b <- a + attr(h, "match.length") - 1L
        ends <- cumsum(nchar(parts))
        begins <- c(1L, ends[-length(ends)] + 1L)
        first <- TRUE
        for (q in seq_along(parts)) {
          if (ends[q] < a || begins[q] > b) next
          la <- max(a, begins[q]) - begins[q] + 1L
          lb <- min(b, ends[q]) - begins[q] + 1L
          parts[q] <- paste0(substr(parts[q], 1L, la - 1L), if (first) map[[p]] else "",
                             substr(parts[q], lb + 1L, nchar(parts[q])))
          first <- FALSE
          attr(parts, "hl") <- unique(c(attr(parts, "hl"), g[q]))
        }
      }
    }
    new <- paste0("<w:t xml:space=\"preserve\">", xml_escape(parts), "</w:t>")
    for (q in seq_along(g)) edits[[length(edits) + 1L]] <- list(i = g[q], new = new[q])
    for (j in attr(parts, "hl")) edits[[length(edits)]]$hl <- c(edits[[length(edits)]]$hl, j)
  }
  if (!length(edits)) return(orig)
  hl <- unlist(lapply(edits, `[[`, "hl"))
  idx <- vapply(edits, `[[`, 0L, "i")
  new <- vapply(edits, `[[`, "", "new")
  o <- order(idx)
  idx <- idx[o]; new <- new[o]
  pieces <- character(2L * length(idx) + 1L)
  prev <- 1L
  for (k in seq_along(idx)) {
    i <- idx[k]
    gap <- as_utf8(substr(xml, prev, st[i] - 1L))
    if (i %in% hl) gap <- drop_last_highlight(gap)
    pieces[2L * k - 1L] <- gap
    pieces[2L * k] <- new[k]
    prev <- en[i] + 1L
  }
  pieces[length(pieces)] <- as_utf8(substr(xml, prev, nchar(xml, "bytes")))
  enc2utf8(paste(pieces, collapse = ""))
}

# The highlight of a filled placeholder sits in the rPr directly before its w:t.
drop_last_highlight <- function(gap) {
  r <- gregexpr("<w:r[ >]", gap)[[1]]
  if (r[1] < 0) return(gap)
  s <- r[length(r)]
  paste0(substr(gap, 1L, s - 1L),
         sub("<w:highlight [^>]*/>", "", substr(gap, s, nchar(gap))))
}
