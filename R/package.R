# Writes the docx container.

write_package <- function(tpl, ctx, xml, felder, out) {
  work <- tempfile("gba-out-")
  dir.create(work)
  on.exit(unlink(work, recursive = TRUE))
  file.copy(list.files(tpl$dir, all.files = TRUE, no.. = TRUE, full.names = TRUE), work, recursive = TRUE)
  write_text(xml, file.path(work, "word/document.xml"))

  if (length(ctx$fn_new)) {
    fx <- sub("</w:footnotes>", paste0(paste(ctx$fn_new, collapse = ""), "</w:footnotes>"), ctx$fn_xml, fixed = TRUE)
    write_text(fx, file.path(work, "word/footnotes.xml"))
  }
  if (length(felder)) {
    for (f in list.files(file.path(work, "word"), "^(header|footer|footnotes)[0-9]*\\.xml$", full.names = TRUE)) {
      s <- read_text(f)
      r <- replace_map(s, felder)
      if (!identical(r, s)) write_text(r, f)
    }
  }
  order <- tpl$order
  if (length(ctx$media)) {
    dir.create(file.path(work, "word/media"), showWarnings = FALSE)
    rp <- file.path(work, "word/_rels/document.xml.rels")
    rel <- read_text(rp)
    ctp <- file.path(work, "[Content_Types].xml")
    ct <- read_text(ctp)
    add <- character()
    for (m in ctx$media) {
      file.copy(m$path, file.path(work, "word/media", m$name))
      add <- c(add, sprintf("<Relationship Id=\"%s\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"media/%s\"/>", m$rid, m$name))
      order <- c(order, file.path("word/media", m$name))
      if (!grepl(sprintf("Extension=\"%s\"", m$ext), ct, fixed = TRUE)) {
        ct <- sub("<Default ", sprintf("<Default Extension=\"%s\" ContentType=\"image/%s\"/><Default ", m$ext, m$ext), ct, fixed = TRUE)
      }
    }
    write_text(sub("</Relationships>", paste0(paste(add, collapse = ""), "</Relationships>"), rel, fixed = TRUE), rp)
    write_text(ct, ctp)
  }
  target <- file.path(normalizePath(dirname(out), mustWork = TRUE), basename(out))
  unlink(target)
  old <- setwd(work)
  on.exit(setwd(old), add = TRUE, after = FALSE)
  lst <- tempfile()
  writeLines(order, lst)
  st <- system2("zip", c("-X", "-q", shQuote(target), "-@"), stdin = lst)
  if (st != 0L) stop("zip fehlgeschlagen")
  invisible(target)
}
