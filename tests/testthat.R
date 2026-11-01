# rv run tests/testthat.R
library(testthat)
root <- normalizePath(".")
options(gba.root = root)
for (f in list.files("R", "\\.R$", full.names = TRUE)) source(f)
gba_fetch()
test_dir("tests/testthat", stop_on_failure = TRUE, reporter = "summary")
