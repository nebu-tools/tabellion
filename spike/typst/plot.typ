// A small chart drawn with Typst, used as a figure image in the spike.
#set page(width: 12cm, height: 6cm, margin: 4mm)
#let data = (3, 5, 4, 7, 6, 8)
#for (i, v) in data.enumerate() {
  place(bottom + left, dx: 0.6cm + i * 1.8cm, rect(width: 1.2cm, height: v * 0.6cm, fill: rgb("#2f5496")))
}
#place(bottom + left, line(length: 11cm, stroke: 0.5pt))
