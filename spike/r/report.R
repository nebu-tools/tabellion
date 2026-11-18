# A report built from data with the tabellion R package: tables in three
# kinds of style, a plot as a figure, and Typst text that refers to both.
#
#   R CMD INSTALL r/tabellion && Rscript spike/r/report.R
library(tabellion)

doc <- open_docx("samples/report-template.docx")

rates <- data.frame(
  Region = c("North", "East", "South", "West"),
  n = c(1204L, 987L, 1432L, 776L),
  Mean = c(12.134, 10.421, 9.807, 11.25),
  Change = c(0.231, -0.021, 0.118, 0.040),
  check.names = FALSE
)

# 1. A table style the template already has.
add_table(doc, rates, style = "Report Table", digits = 2,
          at = at(placeholder = "results"),
          caption = "Rates by region, template style", label = "tab-template")

# 2. A table style defined in Typst (spike/typst/styles.typ).
compact <- typst_table_style("spike/typst/styles.typ", "compact")
add_table(doc, rates, style = compact, digits = 2,
          at = at(heading = "Methods"), mode = "before",
          caption = "Rates by region, style defined in Typst", label = "tab-typst")

# 3. A table style defined in R.
house <- table_style("House Green",
  header = list(fill = "#385723", color = "#FFFFFF", bold = TRUE),
  band_fill = "#E2EFDA", top = rule(1, "385723"), bottom = rule(1, "385723"),
  inside_h = rule(0.25, "A9D08E"))
add_table(doc, rates, style = house, digits = 1,
          at = at(heading = "Methods"), mode = "before",
          caption = "Rates by region, style defined in R", label = "tab-r")

# A plot as a figure with a numbered caption.
add_plot(doc, function() {
  par(mar = c(4, 4, 1, 1))
  barplot(rates$Mean, names.arg = rates$Region, col = "#2f5496", ylab = "Mean rate")
}, width = 14, height = 7, caption = "Mean rate by region", label = "fig-rates",
   at = at(heading = "Methods"), mode = "before")

# Typst generated in R, with cross-references Word keeps up to date.
best <- rates$Region[which.max(rates$Mean)]
add_typst(doc, sprintf(
  "The highest mean rate was in *%s* (%s). @tab-template, @tab-typst and @tab-r show the same data in three styles; @fig-rates plots it.",
  best, format(max(rates$Mean), digits = 3)),
  at = at(heading = "Methods"), mode = "before")

# A tracked edit by a named author.
add_typst(doc, "_This paragraph was added as a tracked change._", at = at(bookmark = "summary"), author = "Analyst")

save_docx(doc, "spike/out/r-report.docx")

# Reading back.
back <- open_docx("spike/out/r-report.docx")
print(back)
str(docx_tables(back)[[1]])
print(subset(docx_blocks(back), kind == "paragraph" & grepl("highest", text))$text)
