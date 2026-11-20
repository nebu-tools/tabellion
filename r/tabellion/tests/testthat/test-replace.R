test_that("text is replaced and revisions settle", {
  src <- testthat::test_path("fixtures", "report-template.docx")
  skip_if(!file.exists(src), "sample document is missing")

  doc <- open_docx(src)
  doc <- add_paragraph(doc, "A draft plan.")
  doc <- replace_text(doc, "draft", "final")
  expect_true(any(grepl("A final plan.", docx_text(doc), fixed = TRUE)))

  doc <- add_paragraph(doc, "A draft tally.")
  doc <- replace_text(doc, "draft", "final", author = "Lark")
  # Another author's accept settles nothing and keeps the handle.
  doc <- accept_revisions(doc, author = "Wren")
  expect_s3_class(doc, "tabellion_doc")
  doc <- accept_revisions(doc, author = "Lark")
  expect_true(any(grepl("A final tally.", docx_text(doc), fixed = TRUE)))

  doc <- add_paragraph(doc, "A draft sketch.")
  doc <- replace_text(doc, "draft", "final", author = "Lark")
  doc <- reject_revisions(doc)
  expect_true(any(grepl("A draft sketch.", docx_text(doc), fixed = TRUE)))

  expect_error(replace_text(doc, "nightingale", "thrush"))
  expect_error(replace_text(doc, "", "final"))
})
