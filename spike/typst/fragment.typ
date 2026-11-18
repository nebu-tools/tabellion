#import "@tabellion": table-style, styled-table

== Results by region

The *overall* rate rose by #text(fill: rgb("#c00000"))[2.4 points] in the _second_ period,
as @tab-regions shows.#footnote[Rates are per 1,000 people and rounded.] The trend
in @fig-trend follows the model in @eq-model.

- Region North gained most.
- Regions *East* and *West* stayed flat.
  - East fell slightly.
- Region South recovered.

+ Collect the data.
+ Fit the model.

#figure(
  table(
    columns: (2fr, 1fr, 1fr, 1fr),
    table.header(
      table.cell(rowspan: 2)[Region], table.cell(colspan: 2, fill: rgb("#d9e2f3"))[Period], [Change],
      [A], [B], [%],
    ),
    [North], [12.1], [14.9], table.cell(fill: rgb("#e2efda"))[+23],
    [East], [10.4], [10.2], [−2],
    [South], [9.8], [11.0], [+12],
  ),
  caption: [Rates by region and period],
) <tab-regions>

#figure(
  image("plot.png", width: 12cm),
  caption: [Trend of the overall rate],
) <fig-trend>

$ hat(y)_i = beta_0 + sum_(k=1)^K beta_k x_(i k) + epsilon_i, quad epsilon_i tilde cal(N)(0, sigma^2) $ <eq-model>

Inline math such as $sqrt(x^2 + 1) / 2$ stays an equation in Word.

#rect(width: 4cm, height: 1cm, fill: rgb("#fde9d9"))[A drawn box]

#let compact = table-style(
  name: "Compact Data",
  font-size: 9pt,
  header: (fill: rgb("#7f6000"), color: white, bold: true),
  band-fill: rgb("#fff2cc"),
  top: 1.5pt + rgb("#7f6000"),
  bottom: 1.5pt + rgb("#7f6000"),
  inside-h: 0.5pt + rgb("#bfbfbf"),
)

#figure(
  styled-table(compact, columns: 3, table.header[Item][Count][Share], [Alpha], [120], [40 %], [Beta], [90], [30 %], [Gamma], [90], [30 %]),
  caption: [A table in a style defined in Typst],
) <tab-compact>
