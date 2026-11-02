# Moves an edited Typst project to a new template version. Block IDs are
# positions, so a new template shifts them. Blocks with the same text keep their
# content; everything else ends up in the report.

block_key <- function(b) {
  if (b$type == "tbl") {
    cells <- unlist(lapply(b$table$rows, function(r) lapply(r$cells, function(c) {
      vapply(c$paras, function(p) tokens_plain(p$tokens), "")
    })))
    return(paste("tabelle", gsub("\\s+", " ", paste(cells, collapse = "|"))))
  }
  paste(b$style, gsub("\\s+", " ", trimws(tokens_plain(b$para$tokens))))
}

block_keys <- function(tpl) {
  ids <- Filter(function(id) block_kind(tpl$blocks[[id]]) != "toc", tpl$ids)
  keys <- vapply(ids, function(id) block_key(tpl$blocks[[id]]), "")
  occ <- stats::ave(seq_along(keys), keys, FUN = seq_along)
  stats::setNames(paste(keys, occ), ids)
}

#' Rewrites `typ` for the current template of `modul` in the directory `dir`.
gba_migrate <- function(typ, modul, dir) {
  src <- read_text(typ)
  name <- first_match("vorlage: \"([^\"]+)\"", src)
  old <- read_template(file.path(dirname(typ), name))
  on.exit(unlink(old$dir, recursive = TRUE))
  frisch <- gba_import(modul, dir)
  new <- read_template(gba_template(modul))
  on.exit(unlink(new$dir, recursive = TRUE), add = TRUE)
  ok <- block_keys(old)
  nk <- block_keys(new)
  map <- stats::setNames(names(nk)[match(ok, nk)], names(ok))

  # TOC ranges hold page numbers, so they come from the fresh import.
  ranges <- regmatches(read_text(frisch), gregexpr("#verzeichnis\\([^)]*\\)", read_text(frisch)))[[1]]
  m <- gregexpr("#verzeichnis\\([^)]*\\)", src)
  if (m[[1]][1] > 0 && length(ranges) == length(m[[1]])) regmatches(src, m) <- list(ranges)
  keep <- regmatches(src, gregexpr("#verzeichnis\\([^)]*\\)", src))[[1]]
  src <- gsub("#verzeichnis\\([^)]*\\)", "\u0001", src)

  used <- unique(regmatches(src, gregexpr("\"b[0-9]{4}\"", src))[[1]])
  used <- gsub("\"", "", used)
  lost <- used[is.na(map[used])]
  ids <- gregexpr("\"b[0-9]{4}\"", src)
  regmatches(src, ids) <- list(vapply(regmatches(src, ids)[[1]], function(q) {
    id <- gsub("\"", "", q)
    if (is.na(map[id])) sprintf("\"ALT-%s\"", id) else sprintf("\"%s\"", map[[id]])
  }, "", USE.NAMES = FALSE))
  for (r in keep) src <- sub("\u0001", r, src, fixed = TRUE)
  src <- gsub(name, basename(gba_template(modul)), src, fixed = TRUE)
  write_text(src, frisch)
  # Data, figures, and includes stay reachable by their relative paths.
  skip <- c(basename(typ), "gba.typ", name)
  extra <- setdiff(list.files(dirname(typ), all.files = TRUE, no.. = TRUE), skip)
  extra <- extra[!grepl("\\.docx$", extra)]
  file.copy(file.path(dirname(typ), extra), dir, recursive = TRUE, overwrite = TRUE)
  neu <- setdiff(names(nk), map)
  cat(sprintf("%d Blöcke übertragen, %d ohne Gegenstück, %d neu in der Vorlage.\n",
              length(used) - length(lost), length(lost), length(neu)))
  if (length(lost)) cat("Prüfen (\"ALT-...\" in der .typ):", paste(lost, collapse = ", "), "\n")
  if (length(neu)) cat("Neu in der Vorlage:", paste(neu, collapse = ", "), "\n")
  invisible(list(datei = frisch, verloren = lost, neu = neu))
}
