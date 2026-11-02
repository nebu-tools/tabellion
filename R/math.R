# Typst equations -> Office Math (OMML). Word shows these as native formulas.

MATH_RPR <- "<w:rPr><w:rFonts w:ascii=\"Cambria Math\" w:hAnsi=\"Cambria Math\"/></w:rPr>"

omml_run <- function(text) {
  paste0("<m:r>", MATH_RPR, "<m:t xml:space=\"preserve\">", xml_escape(text), "</m:t></m:r>")
}

omml_arg <- function(tag, node) paste0("<m:", tag, ">", omml(node), "</m:", tag, ">")

omml_attach <- function(x) {
  base <- omml_arg("e", x$base)
  if (!is.null(x$b) && !is.null(x$t)) {
    return(paste0("<m:sSubSup>", base, omml_arg("sub", x$b), omml_arg("sup", x$t), "</m:sSubSup>"))
  }
  if (!is.null(x$b)) return(paste0("<m:sSub>", base, omml_arg("sub", x$b), "</m:sSub>"))
  if (!is.null(x$t)) return(paste0("<m:sSup>", base, omml_arg("sup", x$t), "</m:sSup>"))
  omml(x$base)
}

omml_root <- function(x) {
  deg <- if (is.null(x$index)) "<m:radPr><m:degHide m:val=\"1\"/></m:radPr><m:deg/>" else omml_arg("deg", x$index)
  paste0("<m:rad>", deg, omml_arg("e", x$radicand), "</m:rad>")
}

omml_accent <- function(x) {
  sprintf("<m:acc><m:accPr><m:chr m:val=\"%s\"/></m:accPr>%s</m:acc>", xml_escape(x$accent), omml_arg("e", x$base))
}

#' One Typst math node as OMML.
omml <- function(x) {
  if (is.null(x)) return("")
  if (is.character(x)) return(omml_run(x))
  f <- x$func %||% ""
  switch(f,
    sequence = paste(vapply(x$children, omml, ""), collapse = ""),
    text = , symbol = omml_run(x$text),
    op = if (is.list(x$text)) omml(x$text) else omml_run(x$text),
    space = "",
    attach = omml_attach(x),
    frac = paste0("<m:f>", omml_arg("num", x$num), omml_arg("den", x$denom), "</m:f>"),
    root = omml_root(x),
    accent = omml_accent(x),
    lr = omml(x$body),
    styled = omml(x$child),
    stop(sprintf("Formelelement '%s' lässt sich nicht nach Word abbilden.", f))
  )
}

omml_inline <- function(eq) paste0("<m:oMath>", omml(eq$body), "</m:oMath>")

omml_block <- function(eq, ppr) {
  paste0("<w:p>", ppr, "<m:oMathPara>", omml_inline(eq), "</m:oMathPara></w:p>")
}
