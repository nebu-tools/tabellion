# Byte-stable slicing of WordprocessingML.
#
# xml2 cannot give us the verbatim bytes of a node, and re-serialising would
# break the promise that untouched template parts survive unchanged. So
# structure is found by a tag scanner on the original string, and xml2 is only
# used to *read* fragments.

TAG_RE <- "<(?:[^>\"']|\"[^\"]*\"|'[^']*')*+>"

# substring() on UTF-8 walks from the start of the string each time, which is
# quadratic here. Declared as latin1 every byte is one character, so offsets
# stay plain byte offsets; as_utf8() restores the real encoding afterwards.
as_bytes <- function(x) {
  Encoding(x) <- "latin1"
  x
}

as_utf8 <- function(x) {
  Encoding(x) <- "UTF-8"
  x
}

scan_tags <- function(x) {
  m <- gregexpr(TAG_RE, x, perl = TRUE, useBytes = TRUE)[[1]]
  if (m[1] < 0) {
    return(data.frame(start = integer(), end = integer(), depth = integer()))
  }
  start <- as.integer(m)
  end <- start + attr(m, "match.length") - 1L
  tags <- substring(x, start, end)
  second <- substr(tags, 2, 2)
  closing <- second == "/"
  special <- second == "?" | second == "!"
  selfclose <- substr(tags, nchar(tags) - 1L, nchar(tags) - 1L) == "/"
  delta <- ifelse(closing, -1L, ifelse(selfclose | special, 0L, 1L))
  data.frame(
    start = start, end = end, tag = tags, delta = delta,
    depth_after = cumsum(delta), stringsAsFactors = FALSE
  )
}

#' Verbatim children of the root element of `x` (or of the element whose open
#' tag ends at the first depth-1 position). Returns open tag, children, close.
xml_slice <- function(x) {
  x <- as_bytes(enc2utf8(x))
  tg <- scan_tags(x)
  first <- which(tg$delta == 1L)[1]
  base <- tg$depth_after[first]
  last <- nrow(tg)
  while (tg$delta[last] != -1L) last <- last - 1L
  inner <- tg[(first + 1L):(last - 1L), , drop = FALSE]
  if (first + 1L > last - 1L) {
    return(list(
      open = as_utf8(substr(x, 1L, tg$end[first])), children = character(),
      close = as_utf8(substr(x, tg$start[last], nchar(x)))
    ))
  }
  before <- inner$depth_after - inner$delta
  starts <- inner$start[before == base & inner$delta >= 0L]
  ends <- inner$end[inner$depth_after == base & inner$delta <= 0L]
  list(
    open = as_utf8(substr(x, 1L, tg$end[first])),
    children = as_utf8(substring(x, starts, ends)),
    close = as_utf8(substr(x, tg$start[last], nchar(x)))
  )
}

elem_name <- function(x) {
  sub("^<([^ />]+).*$", "\\1", substr(x, 1L, 60L), perl = TRUE)
}

xml_escape <- function(x) {
  x <- gsub("&", "&amp;", x, fixed = TRUE)
  x <- gsub("<", "&lt;", x, fixed = TRUE)
  gsub(">", "&gt;", x, fixed = TRUE)
}

xml_unescape <- function(x) {
  x <- gsub("&lt;", "<", x, fixed = TRUE)
  x <- gsub("&gt;", ">", x, fixed = TRUE)
  x <- gsub("&quot;", "\"", x, fixed = TRUE)
  x <- gsub("&apos;", "'", x, fixed = TRUE)
  gsub("&amp;", "&", x, fixed = TRUE)
}

read_text <- function(path) {
  x <- readBin(path, "raw", file.size(path))
  x <- rawToChar(x)
  Encoding(x) <- "UTF-8"
  enc2utf8(x)
}

write_text <- function(x, path) {
  writeBin(charToRaw(enc2utf8(x)), path)
}
