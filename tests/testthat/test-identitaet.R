test_that("unveraenderte Quelle reproduziert jede Vorlage bytegenau", {
  for (m in module) {
    b <- bauen(m)
    expect_identical(teile(b$out), teile(gba_template(m)), info = m)
    expect_equal(b$rep$stats$geaendert, 0L, info = m)
  }
})
