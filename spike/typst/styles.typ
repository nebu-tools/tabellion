// Table styles shared between Typst documents and R scripts.
#import "@tabellion": table-style

#let compact = table-style(
  name: "Compact Data",
  font-size: 9pt,
  header: (fill: rgb("#7f6000"), color: white, bold: true),
  band-fill: rgb("#fff2cc"),
  top: 1.5pt + rgb("#7f6000"),
  bottom: 1.5pt + rgb("#7f6000"),
  inside-h: 0.5pt + rgb("#bfbfbf"),
)
