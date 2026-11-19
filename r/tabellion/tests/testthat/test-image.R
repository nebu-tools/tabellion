test_that("pictures append with captions", {
  src <- testthat::test_path("fixtures", "report-template.docx")
  png <- testthat::test_path("fixtures", "wren.png")
  skip_if(!file.exists(src) || !file.exists(png), "sample files are missing")

  doc <- open_docx(src)
  blocks_before <- nrow(docx_blocks(doc))
  doc <- add_image(doc, png)
  expect_equal(nrow(docx_blocks(doc)), blocks_before + 1)

  doc <- add_image(doc, png, caption = "A wren.", label = "fig-wren", alt = "A wren alt")
  lines <- docx_text(doc)
  expect_true(any(grepl("Figure [0-9]+: A wren\\.", lines)))
  # The alternative text survives into the drawing.
  out <- tempfile(fileext = ".docx")
  save_docx(doc, out)
  xml <- paste(readLines(unz(out, "word/document.xml"), warn = FALSE), collapse = "\n")
  expect_true(grepl('descr="A wren alt"', xml, fixed = TRUE))

  # A label without a caption has nothing to number.
  expect_error(add_image(doc, png, label = "fig-wren"))
  # A missing file never reaches Rust; unknown bytes are no picture.
  expect_error(add_image(doc, file.path("no", "such.png")))
  bad <- tempfile(fileext = ".png")
  writeBin(charToRaw("not a picture"), bad)
  expect_error(add_image(doc, bad))
})
