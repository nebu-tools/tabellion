sites_frame <- function() {
  data.frame(
    site = c("A", "B"),
    count = c(3, NA),
    stringsAsFactors = FALSE
  )
}

test_that("a data frame becomes a table", {
  src <- testthat::test_path("fixtures", "report-template.docx")
  skip_if(!file.exists(src), "sample document is missing")
  doc <- open_docx(src)
  sites <- sites_frame()
  doc <- add_table(doc, sites)
  lines <- docx_text(doc)
  expect_true("site\tcount" %in% lines)
  expect_true("A\t3" %in% lines)
  expect_true("B\t" %in% lines)
  # Captioned and labelled tables number like the engine writes them.
  doc <- add_table(doc, sites, caption = "Counts.", label = "tab-counts")
  again <- docx_text(doc)
  expect_true(any(grepl("Table [0-9]+: Counts\\.", again)))
  # A label without a caption has nothing to number.
  expect_error(add_table(doc, sites, label = "tab-counts"))
  # Anything but a data frame, and a frame without columns, is an error.
  expect_error(add_table(doc, list(a = 1)))
  expect_error(add_table(doc, "sites"))
  expect_error(add_table(doc, sites[, FALSE]))
  # A matrix stored as one column would silently lose values.
  wide <- data.frame(id = 1:2)
  wide$m <- matrix(1:4, nrow = 2)
  expect_error(add_table(doc, wide), "m")
  # A nested frame can pass the length check with the wrong rows.
  nested <- data.frame(id = 1:2)
  nested$x <- data.frame(a = 1:2, b = 3:4)
  expect_error(add_table(doc, nested), "x")
  # Without names the same guard must still fire, by position.
  nameless <- nested
  names(nameless) <- NULL
  expect_error(add_table(doc, nameless), "column 2")
})

test_that("tables read back as data frames", {
  src <- testthat::test_path("fixtures", "report-template.docx")
  skip_if(!file.exists(src), "sample document is missing")
  doc <- open_docx(src)
  doc <- add_table(doc, sites_frame())
  tables <- docx_tables(doc)
  expect_true(is.list(tables))
  expect_true(length(tables) >= 1)
  is_site_table <- vapply(
    tables,
    function(t) nrow(t) == 3 && identical(unlist(t[1, ]), c(V1 = "site", V2 = "count")),
    logical(1)
  )
  expect_true(any(is_site_table))
  grid <- tables[[which(is_site_table)[1]]]
  expect_identical(names(grid), c("V1", "V2"))
  expect_identical(unlist(grid[2, ]), c(V1 = "A", V2 = "3"))
  # An empty cell reads back empty; only merged coverage reads back NA.
  expect_identical(unlist(grid[3, ]), c(V1 = "B", V2 = ""))
  # A merged cell covers the grid: NA at the covered position.
  doc <- add_typst(doc, "#table(columns: 2, table.cell(colspan: 2)[wide], [b], [c])\n")
  wide <- docx_tables(doc)[[length(docx_tables(doc))]]
  expect_identical(unlist(wide[1, ]), c(V1 = "wide", V2 = NA_character_))
  expect_identical(unlist(wide[2, ]), c(V1 = "b", V2 = "c"))
})
