# Typst invocation and conversion of Typst content JSON to tokens.

typst_bin <- function() {
  b <- Sys.getenv("GBA_TYPST", "typst")
  if (!nzchar(Sys.which(b))) stop("typst nicht gefunden (GBA_TYPST setzen).")
  b
}

typst_query <- function(file) {
  out <- tempfile(fileext = ".json")
  err <- tempfile()
  st <- system2(typst_bin(), c(
    "eval", shQuote("query(<gba>)"), "--in", shQuote(file),
    "--root", shQuote(dirname(normalizePath(file)))
  ), stdout = out, stderr = err)
  if (st != 0L) stop("typst: ", paste(readLines(err), collapse = "\n"))
  items <- jsonlite::fromJSON(read_text(out), simplifyVector = FALSE)
  unnest_items(lapply(items, `[[`, "value"))
}

count_nested <- function(x) {
  if (!is.list(x)) return(0L)
  x$vorschau <- NULL
  n <- if (identical(x$func, "metadata") && identical(x$label, "<gba>")) 1L else 0L
  n + sum(vapply(x, count_nested, 0L))
}

# query() lists a block nested in an antwort body twice: inside the body and
# on its own. The body wins, the flat copies are skipped.
unnest_items <- function(items) {
  out <- list()
  i <- 1L
  while (i <= length(items)) {
    it <- items[[i]]
    out[[length(out) + 1L]] <- it
    i <- i + 1L + count_nested(it)
  }
  out
}

plain_text_tok <- function(text, fmt) c(list(type = "text", text = text, rpr = ""), fmt)

no_fmt <- list(b = FALSE, i = FALSE, u = FALSE, sub = FALSE, sup = FALSE)

#' Typst content (parsed JSON) -> flat token list. "pb" tokens split paragraphs;
#' block-level nodes come back as "block" tokens.
content_tokens <- function(x, fmt = no_fmt, where = "") {
  if (is.null(x)) return(list())
  if (is.character(x)) return(list(plain_text_tok(x, fmt)))
  if (is.numeric(x)) return(list(plain_text_tok(format(x, trim = TRUE), fmt)))
  f <- x$func
  rec <- function(n, ff = fmt) content_tokens(n, ff, where)
  with_fmt <- function(k) { ff <- fmt; ff[[k]] <- TRUE; rec(x$body, ff) }
  switch(f %||% "",
    sequence = do.call(base::c, c(list(list()), lapply(x$children, rec))),
    text = , raw = list(plain_text_tok(x$text, fmt)),
    symbol = list(plain_text_tok(x$text, fmt)),
    space = list(plain_text_tok(" ", fmt)),
    linebreak = list(list(type = "raw", xml = "<w:r><w:br/></w:r>")),
    parbreak = list(list(type = "pb")),
    strong = with_fmt("b"),
    emph = with_fmt("i"),
    underline = with_fmt("u"),
    sub = with_fmt("sub"),
    super = with_fmt("sup"),
    smartquote = list(plain_text_tok(if (isTRUE(x$double)) "\"" else "'", fmt)),
    link = rec(x$body),
    styled = rec(x$child),
    footnote = list(list(type = "footnote", body = x$body)),
    equation = rec(x$body),
    attach = c(rec(x$base),
      if (!is.null(x$b)) { ff <- fmt; ff$sub <- TRUE; rec(x$b, ff) },
      if (!is.null(x$t)) { ff <- fmt; ff$sup <- TRUE; rec(x$t, ff) }),
    item = list(list(type = "block", node = x)),
    heading = , table = , figure = , image = list(list(type = "block", node = x)),
    metadata = {
      v <- x$value
      if (!is.null(v[["gba-atom"]])) {
        list(list(type = "atom", key = v[["gba-atom"]], on = isTRUE(v$on)))
      } else if (identical(x$label, "<gba>")) {
        list(list(type = "block", node = v))
      } else {
        stop("Unbekanntes metadata-Element in ", where)
      }
    },
    stop(sprintf("Typst-Element '%s' in %s lässt sich nicht nach Word abbilden.", f, where))
  )
}

split_paragraphs <- function(toks) {
  out <- list()
  cur <- list()
  for (t in toks) {
    if (identical(t$type, "pb")) {
      out[[length(out) + 1L]] <- cur
      cur <- list()
    } else if (identical(t$type, "block")) {
      if (length(cur)) out[[length(out) + 1L]] <- cur
      out[[length(out) + 1L]] <- list(t)
      cur <- list()
    } else {
      cur <- c(cur, list(t))
    }
  }
  if (length(cur) || !length(out)) out[[length(out) + 1L]] <- cur
  is_empty <- vapply(out, function(p) {
    !length(p) || (all(vapply(p, `[[`, "", "type") == "text") && !nzchar(trimws(tokens_plain(p))))
  }, NA)
  if (all(is_empty)) return(list(list()))
  out[!is_empty]
}
