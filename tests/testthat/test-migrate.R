test_that("migrate überträgt Inhalte auf eine neue Vorlagenversion", {
  dir <- tempfile("gba-alt-")
  typ <- gba_import("modul2", dir)
  name <- basename(gba_template("modul2"))
  src <- read_text(typ)
  id <- regmatches(src, regexpr("#antwort\\(\"b[0-9]+\"\\)", src))
  write_text(sub(id, paste0(id, "[Mein Text]"), src, fixed = TRUE), typ)

  # The old project uses a copy of the template with one extra leading block,
  # so every block of the current template sits one position later in it.
  d <- tempfile(); dir.create(d)
  utils::unzip(file.path(dir, name), exdir = d)
  x <- read_text(file.path(d, "word/document.xml"))
  write_text(sub("<w:body>", "<w:body><w:p><w:r><w:t>Alt</w:t></w:r></w:p>", x, fixed = TRUE), file.path(d, "word/document.xml"))
  old <- setwd(d); on.exit(setwd(old))
  unlink(file.path(dir, name))
  zip_ok <- system2("zip", c("-X", "-q", "-r", shQuote(file.path(dir, name)), "."))
  setwd(old)
  expect_equal(zip_ok, 0L)
  src <- read_text(typ)
  ids <- gregexpr("\"b[0-9]{4}\"", src)
  regmatches(src, ids) <- list(vapply(regmatches(src, ids)[[1]], function(q) {
    sprintf("\"b%04d\"", as.integer(gsub("[^0-9]", "", q)) + 1L)
  }, "", USE.NAMES = FALSE))
  write_text(src, typ)

  writeLines("[]", file.path(dir, "ergebnisse.json"))
  ziel <- tempfile("gba-neu-")
  capture.output(r <- gba_migrate(typ, "modul2", ziel))
  neu <- read_text(r$datei)
  expect_match(neu, "[Mein Text]", fixed = TRUE)
  expect_match(neu, id, fixed = TRUE)
  expect_length(r$verloren, 0L)
  expect_match(neu, sprintf("vorlage: \"%s\"", name), fixed = TRUE)
  expect_true(file.exists(file.path(ziel, "ergebnisse.json")))
  out <- file.path(ziel, "out.docx")
  capture.output(rep <- gba_build(r$datei, out = out))
  expect_length(rep$fehlend, 0L)
  expect_match(dokument(out), ">Mein Text</w:t>", fixed = TRUE)
})

test_that("Geänderter Tabellentext in einer neuen Vorlage bleibt nicht unbemerkt", {
  dir <- tempfile("gba-alt-")
  gba_import("modul2", dir)
  tpl <- read_template(gba_template("modul2"))
  on.exit(unlink(tpl$dir, recursive = TRUE))
  id <- Filter(function(i) tpl$blocks[[i]]$type == "tbl", tpl$ids)[[1]]
  b <- tpl$blocks[[id]]
  anders <- b
  anders$table$rows[[length(b$table$rows)]]$cells[[1]]$paras[[1]]$tokens <-
    list(plain_text_tok("geändert", no_fmt))
  expect_false(identical(block_key(b), block_key(anders)))
})
