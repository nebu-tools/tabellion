# Citations of the Word add-in in addin/. A citation is an inline content
# control tagged `zitat:<id>`, a reference list a block content control tagged
# `literaturverzeichnis`. The cited works and each citation's items live in a
# custom XML part, see addin/src/model.js.

ZITAT_NS <- "urn:newga:zitate:1"
ZITAT_XP <- "descendant-or-self::w:sdt[w:sdtPr/w:tag[starts-with(@w:val, 'zitat:')]]"
LISTE_XP <- "descendant-or-self::w:sdt[w:sdtPr/w:tag[@w:val = 'literaturverzeichnis']]"

#' The add-in's store from an unpacked docx: list(quellen, zitate).
zitate_store <- function(dir) {
  for (f in list.files(file.path(dir, "customXml"), "^item[0-9]+\\.xml$", full.names = TRUE)) {
    s <- read_text(f)
    if (!grepl(sprintf("<zitate xmlns=\"%s\"", ZITAT_NS), s, fixed = TRUE)) next
    return(jsonlite::fromJSON(xml2::xml_text(xml2::read_xml(s)), simplifyVector = FALSE))
  }
  list(quellen = list(), zitate = list())
}

#' Citations with their keys and text, reference lists, and the library size.
zitate_outline <- function(tpl) {
  ns <- tpl$ns
  store <- zitate_store(tpl$dir)
  zitate <- list()
  listen <- list()
  for (id in tpl$ids) {
    if (!grepl("zitat:|literaturverzeichnis", tpl$blocks[[id]]$xml)) next
    n <- blk(tpl, id)
    for (s in xfind(n, ZITAT_XP, ns)) {
      tag <- wattr(xml2::xml_find_first(s, "w:sdtPr/w:tag", ns), "val", ns)
      items <- store$zitate[[sub("^zitat:", "", tag)]]
      text <- paste(xml2::xml_text(xfind(s, "w:sdtContent//w:t", ns)), collapse = "")
      zitate[[length(zitate) + 1L]] <- list(block = id, schluessel = I(vapply(items, function(i) i$id, "")), text = text)
    }
    if (xhas(n, LISTE_XP, ns)) listen[[length(listen) + 1L]] <- list(block = id)
  }
  list(zitate = zitate, literaturverzeichnisse = listen, quellen = length(store$quellen))
}
