test_that("images tabulate with sizes and text", {
  src <- testthat::test_path("fixtures", "report-template.docx")
  png <- testthat::test_path("fixtures", "wren.png")
  skip_if(!file.exists(src) || !file.exists(png), "sample files are missing")

  doc <- open_docx(src)
  expect_equal(nrow(docx_images(doc)), 0)
  doc <- add_image(doc, png, alt = "A wren alt")
  images <- docx_images(doc)
  expect_s3_class(images, "data.frame")
  expect_equal(nrow(images), 1)
  expect_identical(
    names(images),
    c("index", "placement", "relationship", "part", "link",
      "content_type", "width", "height", "name", "title", "alt")
  )
  expect_identical(images$placement, "inline")
  expect_identical(images$content_type, "image/png")
  expect_true(grepl("^word/media/", images$part))
  expect_true(is.na(images$link))
  expect_true(images$width > 0 && images$height > 0)
  expect_identical(images$alt, "A wren alt")
})

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
