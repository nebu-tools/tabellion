# docx template -> Typst source.

#' Escapes literal text for Typst markup.
typst_escape <- function(x) {
  x <- gsub("[ \t\r\n]+", " ", x)
  x <- gsub("([\\\\#$*_`~<>@=/'\"\\[\\]])", "\\\\\\1", x, perl = TRUE)
  x <- gsub("-(?=[-?0-9])", "\\\\-", x, perl = TRUE)
  x <- gsub("\\.(?=\\.)", "\\\\.", x, perl = TRUE)
  x <- gsub(":(?=\\\\/)", "\\\\:", x, perl = TRUE)
  x <- gsub("^([+-])", "\\\\\\1", x, perl = TRUE)
  x <- gsub("^(\\d+)\\.", "\\1\\\\.", x, perl = TRUE)
  x <- gsub(" ", "~", x, fixed = TRUE)
  x <- gsub("­", "\\u{ad}", x, fixed = TRUE)
  x
}

wrap_fmt <- function(s, code) {
  for (ch in rev(strsplit(code, "")[[1]])) {
    fn <- switch(ch, b = "strong", i = "emph", u = "underline", v = "sub", "^" = "super")
    s <- paste0("#", fn, "[", s, "]")
  }
  s
}

#' Typst markup for one paragraph's tokens.
tokens_typst <- function(toks) {
  segs <- normalise_tokens(toks)
  atoms <- Filter(function(t) t$type == "atom", toks)
  byk <- function(k) Filter(function(a) a$key == k, atoms)[[1]]
  out <- vapply(segs, function(s) {
    if (s$kind == "text") return(wrap_fmt(typst_escape(s$text), s$code))
    a <- byk(s$key)
    if (!is.na(a$on)) return(sprintf("#kreuz(%d, %s)", a$key, tolower(a$on)))
    sprintf("#feld(%d)[%s]", a$key, typst_escape(trimws(a$cached)))
  }, "")
  res <- ""
  for (i in seq_along(out)) {
    # A hash expression would otherwise swallow a following "(", "[" or ".".
    if (grepl("^[.(\\[]", out[i]) && grepl("^#", out[max(i - 1L, 1L)])) res <- paste0(res, ";")
    res <- paste0(res, out[i])
  }
  res
}

cell_typst <- function(cell) {
  paste0("[", paste(vapply(cell$paras, function(p) tokens_typst(p$tokens), ""), collapse = "\n\n"), "]")
}

table_typst <- function(id, tbl) {
  rows <- vapply(tbl$rows, function(r) {
    paste0("  (", paste(vapply(r$cells, cell_typst, ""), collapse = ", "), ",),")
  }, "")
  paste0("#tabelle(\"", id, "\",\n", paste(rows, collapse = "\n"), "\n)")
}

heading_level <- function(style) {
  as.integer(sub("^berschrift", "", style))
}

guess_module <- function(path) {
  b <- basename(path)
  m <- regmatches(b, regexpr("Modul_?K?[0-9]", b))
  if (!length(m)) return(list(kapitel = 1L, praefix = ""))
  list(kapitel = as.integer(sub(".*([0-9])$", "\\1", m)), praefix = if (grepl("K", m)) "K" else "")
}

#' Generates a Typst project for a template or any docx based on one.
#'
#' `modul` is a module name or the path of a docx. Compiling the result without changes yields a docx whose parts are
#' byte-identical to the template.
gba_import <- function(modul, dir) {
  template <- if (file.exists(modul)) modul else gba_template(modul)
  dir.create(dir, recursive = TRUE, showWarnings = FALSE)
  tpl <- read_template(template)
  on.exit(unlink(tpl$dir, recursive = TRUE))
  tname <- basename(template)
  file.copy(template, file.path(dir, tname), overwrite = TRUE)
  file.copy(file.path(gba_root(), "typst", c("gba.typ", "methoden.typ")), dir, overwrite = TRUE)
  mod <- guess_module(template)
  out <- c(
    sprintf("// Erzeugt aus %s", tname),
    "// Blöcke tragen die Kennung ihres Gegenstücks in der Vorlage. Unveränderte",
    "// Blöcke werden bytegenau übernommen, geänderte im Stil der Vorlage neu gesetzt.",
    "#import \"gba.typ\": *",
    "#show: dossier.with(",
    sprintf("  vorlage: \"%s\",", tname),
    sprintf("  pruefsumme: \"%s\",", unname(tools::sha256sum(template))),
    sprintf("  kapitel: %d, praefix: \"%s\",", mod$kapitel, mod$praefix),
    "  felder: (:",
    "    // wirkstoff: \"...\", handelsname: \"...\", unternehmer: \"...\",",
    "    // kodierung: \"A\", anwendungsgebiet: \"...\", stand: \"01.12.2026\",",
    "  ),",
    ")",
    ""
  )
  ids <- tpl$ids
  i <- 1L
  while (i <= length(ids)) {
    id <- ids[i]
    b <- tpl$blocks[[id]]
    kind <- block_kind(b)
    if (kind == "toc") {
      j <- i
      while (j < length(ids) && block_kind(tpl$blocks[[ids[j + 1L]]]) == "toc") j <- j + 1L
      out <- c(out, sprintf("#verzeichnis(\"%s\", \"%s\")", id, ids[j]), "")
      i <- j + 1L
      next
    }
    line <- switch(kind,
      tabelle = table_typst(id, b$table),
      antwort = sprintf("#antwort(\"%s\")", id),
      fest = sprintf("#fest(\"%s\")", id),
      heading = sprintf("#ueberschrift(%d, \"%s\")[%s]", heading_level(b$style), id, tokens_typst(b$para$tokens)),
      "heading-frei" = sprintf("#ueberschrift-frei(1, \"%s\")[%s]", id, tokens_typst(b$para$tokens)),
      sprintf("#%s(\"%s\")[%s]", kind, id, tokens_typst(b$para$tokens))
    )
    out <- c(out, line, "")
    i <- i + 1L
  }
  target <- file.path(dir, sub("\\.docx$", ".typ", tname))
  write_text(paste0(paste(out, collapse = "\n"), "\n"), target)
  invisible(target)
}
