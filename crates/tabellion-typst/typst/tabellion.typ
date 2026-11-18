// Helpers for Typst written for Word. Everything here also compiles with
// plain Typst, so the same source gives a PDF preview.

// A table style as data. tabellion writes it into the document as a real
// Word table style; `apply-table-style` gives the same look in Typst.
//
//   #let compact = table-style(name: "Compact", header: (fill: rgb("#1f3864"), color: white, bold: true),
//                              band-fill: luma(240), top: 1pt + rgb("#1f3864"), bottom: 1pt + rgb("#1f3864"))
#let table-style(
  name: none,
  font-size: none,
  font: none,
  top: none,
  bottom: none,
  left: none,
  right: none,
  inside-h: none,
  inside-v: none,
  padding: none,
  header: (:),
  band-fill: none,
  first-column: none,
) = {
  assert(name != none, message: "a table style needs a name")
  let d = (name: name, header: header)
  for (k, v) in (font-size-pt: font-size, font: font, top: top, bottom: bottom, left: left, right: right,
    inside-h: inside-h, inside-v: inside-v, padding-pt: padding, band-fill: band-fill, first-column: first-column) {
    if v != none { d.insert(k, v) }
  }
  d
}

// Marks a table with a style for tabellion, and draws it in that style.
#let styled-table(style, ..args) = {
  let h = style.at("header", default: (:))
  let band = style.at("band-fill", default: none)
  [#metadata((tabellion-table-style: style))<tabellion-table-style>]
  table(
    fill: (x, y) => if y == 0 { h.at("fill", default: none) } else if band != none and calc.odd(y) { band },
    stroke: none,
    ..args,
  )
}
