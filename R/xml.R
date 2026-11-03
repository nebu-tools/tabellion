# XML editing with xml2. Blocks stay strings so unchanged ones are copied byte
# for byte; a block that changes is parsed, edited as nodes, and written back.

#' Parses XML snippets in the namespaces of the document. Returns the wrapper
#' element; its children are the snippets.
xml_frag <- function(x, root_tag) {
  xml2::xml_child(xml2::read_xml(paste0(root_tag, "<w:body>", x, "</w:body></w:document>")))
}

#' First element of a snippet.
xml_node1 <- function(x, root_tag) xml2::xml_child(xml_frag(x, root_tag))

root_ns <- function(root_tag) {
  decl <- regmatches(root_tag, gregexpr("xmlns:([A-Za-z0-9]+)=\"([^\"]*)\"", root_tag))[[1]]
  stats::setNames(sub("^xmlns:[^=]+=\"(.*)\"$", "\\1", decl), sub("^xmlns:([^=]+)=.*$", "\\1", decl))
}

#' A node as XML text. Copying a node makes libxml2 repeat namespace
#' declarations that the document root already makes; those go.
node_str <- function(n, root_tag) {
  x <- as.character(n, options = character())
  for (d in regmatches(root_tag, gregexpr(" xmlns:[A-Za-z0-9]+=\"[^\"]*\"", root_tag))[[1]]) {
    x <- gsub(d, "", x, fixed = TRUE)
  }
  x
}

frag_str <- function(b, root_tag) {
  paste(vapply(xml2::xml_children(b), node_str, "", root_tag = root_tag), collapse = "")
}

xfind <- function(n, xp, ns) xml2::xml_find_all(n, xp, ns)

xhas <- function(n, xp, ns) length(xml2::xml_find_all(n, xp, ns)) > 0L

wattr <- function(n, a, ns) xml2::xml_attr(n, paste0("w:", a), ns)

set_wattrs <- function(n, vals, ns) {
  for (k in names(vals)) xml2::xml_set_attr(n, paste0("w:", k), vals[[k]], ns = ns)
  n
}

#' Adds an element and returns it. `before` names siblings it must precede, as
#' the schema orders properties.
add_el <- function(parent, name, ns, before = character(), first = FALSE) {
  kids <- xml2::xml_children(parent)
  if (first && length(kids)) {
    xml2::xml_add_sibling(kids[[1]], name, .where = "before")
    return(xml2::xml_child(parent, 1))
  }
  nm <- if (length(kids)) xml2::xml_name(kids, ns) else character()
  hit <- which(nm %in% paste0("w:", before))
  if (length(hit)) {
    xml2::xml_add_sibling(kids[[hit[1]]], name, .where = "before")
    return(xml2::xml_children(parent)[[hit[1]]])
  }
  xml2::xml_add_child(parent, name)
  kids <- xml2::xml_children(parent)
  kids[[length(kids)]]
}

#' The named property, created in schema order if missing.
pr_child <- function(parent, name, ns, order) {
  hit <- xml2::xml_find_first(parent, paste0("w:", name), ns)
  if (!inherits(hit, "xml_missing")) return(hit)
  i <- match(name, order)
  add_el(parent, paste0("w:", name), ns, before = if (i < length(order)) order[(i + 1L):length(order)] else character())
}

TCPR_ORDER <- c("cnfStyle", "tcW", "gridSpan", "hMerge", "vMerge", "tcBorders", "shd", "noWrap", "tcMar",
                "textDirection", "tcFitText", "vAlign", "hideMark", "headers", "cellIns", "cellDel", "cellMerge", "tcPrChange")
TBLPR_ORDER <- c("tblStyle", "tblpPr", "tblOverlap", "bidiVisual", "tblStyleRowBandSize", "tblStyleColBandSize",
                 "tblW", "jc", "tblCellSpacing", "tblInd", "tblBorders", "shd", "tblLayout", "tblCellMar", "tblLook",
                 "tblCaption", "tblDescription", "tblPrChange")
PPR_TAIL <- c("rPr", "sectPr", "pPrChange")

#' Run content in the document's form: U+2011 and U+00AD are elements.
add_text <- function(run, text, ns) {
  parts <- regmatches(text, gregexpr("[‑­]|[^‑­]+", text, perl = TRUE))[[1]]
  for (p in parts) {
    if (p == "‑") { xml2::xml_add_child(run, "w:noBreakHyphen"); next }
    if (p == "­") { xml2::xml_add_child(run, "w:softHyphen"); next }
    t <- add_el(run, "w:t", ns)
    xml2::xml_set_attr(t, "xml:space", "preserve")
    xml2::xml_text(t) <- p
  }
  run
}

#' Removes nodes given as a nodeset or a list.
rm_nodes <- function(x) {
  if (inherits(x, "xml_node")) x <- list(x)
  for (n in x) xml2::xml_remove(n)
  invisible()
}
