test_that("a document round-trips through R", {
  src <- testthat::test_path("fixtures", "report-template.docx")
  skip_if(!file.exists(src), "sample document is missing")

  doc <- open_docx(src)
  expect_s3_class(doc, "tabellion_doc")

  blocks <- docx_blocks(doc)
  expect_s3_class(blocks, "data.frame")
  expect_true(nrow(blocks) > 0)
  expect_identical(
    names(blocks),
    c("id", "kind", "style", "level", "text")
  )
  expect_true(all(blocks$kind %in% c(
    "paragraph", "table", "content-control",
    "section", "bookmark", "other"
  )))

  before <- docx_text(doc)
  expect_true(length(before) > 0)

  doc <- add_paragraph(doc, "A wren sang.")
  doc <- add_typst(doc, "= Results\n\nCounted.\n")
  after <- docx_text(doc)
  expect_true(any(grepl("A wren sang.", after, fixed = TRUE)))
  expect_true(any(grepl("Counted.", after, fixed = TRUE)))


  out <- tempfile(fileext = ".docx")
  expect_silent(invisible(save_docx(doc, out)))
  expect_true(file.exists(out))
  doc2 <- open_docx(out)
  expect_identical(docx_text(doc2), after)

  # Embedded line breaks split the output; a terminal line break
  # adds no empty element.
  doc <- add_paragraph(doc, "first\nsecond")
  split <- docx_text(doc)
  expect_true(all(c("first", "second") %in% split))
  expect_identical(tail(split, 1), "second")
})

test_that("field results refresh after edits", {
  src <- testthat::test_path("fixtures", "report-template.docx")
  skip_if(!file.exists(src), "sample document is missing")
  doc <- open_docx(src)
  sites <- data.frame(site = c("A", "B"), count = c(3, NA))
  doc <- add_table(doc, sites, caption = "Counts.", label = "tab-counts")
  before <- docx_text(doc)
  expect_true(any(grepl("Table [0-9]+: Counts\\.", before)))
  # A refresh on fresh results changes nothing and keeps the handle.
  doc <- update_fields(doc)
  expect_s3_class(doc, "tabellion_doc")
  expect_identical(docx_text(doc), before)
})

test_that("missing files and bad content are R errors", {
  expect_error(open_docx(file.path("no", "such.docx")))
  src <- testthat::test_path("fixtures", "report-template.docx")
  skip_if(!file.exists(src), "sample document is missing")
  doc <- open_docx(src)
  expect_error(add_typst(doc, "#grid(columns: 2, [a])\n"))
})

test_that("only this package's handles are accepted", {
  src <- testthat::test_path("fixtures", "report-template.docx")
  skip_if(!file.exists(src), "sample document is missing")
  doc <- open_docx(src)
  expect_error(docx_text(NULL))
  expect_error(docx_text("report.docx"))
  expect_error(docx_text(42L))
  expect_error(docx_blocks(list()))
  # A real foreign external pointer is an R error, never a crash.
  skip_if_not_installed("curl")
  foreign <- curl::new_handle()
  expect_identical(typeof(foreign), "externalptr")
  expect_error(docx_text(foreign))
  expect_error(docx_blocks(foreign))
  # Validation looks at the handle itself, not its class.
  other <- doc
  class(other) <- "other"
  expect_true(length(docx_text(other)) > 0)
})
