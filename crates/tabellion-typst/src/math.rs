//! Typst math as Office Math.

use nebu_docx::content::Math;
use typst_syntax::ast::{self, AstNode, MathTextKind};

use super::{Cx, Error, line_of, unsupported, unsupported_at};

use super::symbols;

/// How to shape a run: bold and upright stick; italics are the default.
#[derive(Clone, Copy, Default)]
struct Face {
    upright: bool,
    bold: bool,
}

/// A n-ary operator with limits below and above it in display math.
fn nary(op: &str, sub: Option<&str>, sup: Option<&str>) -> String {
    let mut pr = format!("<m:naryPr><m:chr m:val=\"{op}\"/><m:limLoc m:val=\"undOvr\"/>");
    if sub.is_none() {
        pr.push_str("<m:subHide m:val=\"1\"/>");
    }
    if sup.is_none() {
        pr.push_str("<m:supHide m:val=\"1\"/>");
    }
    pr.push_str("</m:naryPr>");
    // The schema wants both slots even when one hides. The operand
    // itself is a following sibling: repeating the glyph in `m:e`
    // would render it twice.
    let sub = sub.map_or("<m:sub/>".to_string(), |s| format!("<m:sub>{s}</m:sub>"));
    let sup = sup.map_or("<m:sup/>".to_string(), |s| format!("<m:sup>{s}</m:sup>"));
    format!("<m:nary>{pr}{sub}{sup}<m:e/></m:nary>")
}

/// The n-ary operator of a base expression, if it takes display limits.
/// Dotted names resolve through the symbol table first.
fn nary_op(expr: &ast::Expr) -> Option<&'static str> {
    const OPS: [&str; 7] = ["∑", "∏", "∐", "⋀", "⋁", "⋂", "⋃"];
    let text = match expr {
        ast::Expr::MathIdent(id) => symbols::lookup(id.as_str())?,
        ast::Expr::MathFieldAccess(access) => symbols::lookup(&dotted_name(*access))?,
        ast::Expr::MathText(t) => match t.get() {
            MathTextKind::Grapheme(s) => s.as_str(),
            _ => return None,
        },
        _ => return None,
    };
    OPS.iter().find(|&&op| op == text).copied()
}

/// An equation as one `m:oMath` element.
pub(crate) fn convert_equation(cx: &Cx, eq: ast::Equation, display: bool) -> Result<Math, Error> {
    let inner = convert_math(cx, eq.body(), Face::default(), display)?;
    Ok(Math::omml(&format!("<m:oMath>{inner}</m:oMath>")))
}

/// Math content as Office Math without the outer element.
fn convert_math(cx: &Cx, math: ast::Math, face: Face, display: bool) -> Result<String, Error> {
    let mut out = String::new();
    for expr in math.exprs() {
        // Typst ignores whitespace in math.
        if matches!(expr, ast::Expr::Space(_)) {
            continue;
        }
        out.push_str(&convert_math_expr(cx, expr, face, display)?);
    }
    Ok(out)
}

/// One math expression as Office Math.
fn convert_math_expr(cx: &Cx, expr: ast::Expr, face: Face, display: bool) -> Result<String, Error> {
    match expr {
        ast::Expr::MathText(t) => Ok(run(&math_text(t), face)),
        ast::Expr::MathIdent(id) => {
            let name = id.as_str().to_string();
            let text = symbols::lookup(&name).unwrap_or(name.as_str()).to_string();
            let mut face = face;
            // Multi-letter identifiers set upright, as in Typst; a lone
            // letter or symbol keeps the math italics.
            if text.chars().count() > 1 {
                face.upright = true;
            }
            Ok(run(&text, face))
        }
        ast::Expr::MathShorthand(s) => Ok(run(&s.get().to_string(), face)),
        ast::Expr::MathFieldAccess(field) => {
            let name = dotted_name(field);
            match symbols::lookup(&name) {
                Some(text) => Ok(run(text, face)),
                None => Err(unsupported(
                    cx,
                    expr,
                    &format!("an unknown symbol `{name}`"),
                )),
            }
        }
        ast::Expr::MathAttach(attach) => {
            let base = convert_math_expr(cx, attach.base(), face, display)?;
            // Primes join any explicit superscript first: Typst sets
            // the prime before the superscript in the same slot, and a
            // subscript with any superscript shares one object.
            // Scripts set in a smaller size, so they never take
            // display limits.
            let mut sup = match attach.top() {
                Some(top) => Some(convert_math_expr(cx, top, face, false)?),
                None => None,
            };
            if let Some(primes) = attach.primes() {
                let mark = match primes.count() {
                    1 => "\u{2032}".to_string(),
                    2 => "\u{2033}".to_string(),
                    3 => "\u{2034}".to_string(),
                    n => "\u{2032}".repeat(n),
                };
                let primed = run(&mark, face);
                sup = Some(match sup {
                    Some(s) => format!("{primed}{s}"),
                    None => primed,
                });
            }
            match (attach.bottom(), sup) {
                (Some(sub), Some(sup)) => {
                    let sub = convert_math_expr(cx, sub, face, false)?;
                    if display
                        && attach.primes().is_none()
                        && let Some(op) = nary_op(&attach.base())
                    {
                        return Ok(nary(op, Some(&sub), Some(&sup)));
                    }
                    Ok(format!(
                        "<m:sSubSup><m:e>{base}</m:e><m:sub>{sub}</m:sub><m:sup>{sup}</m:sup></m:sSubSup>"
                    ))
                }
                (Some(sub), None) => {
                    let sub = convert_math_expr(cx, sub, face, false)?;
                    if display
                        && attach.primes().is_none()
                        && let Some(op) = nary_op(&attach.base())
                    {
                        return Ok(nary(op, Some(&sub), None));
                    }
                    Ok(format!(
                        "<m:sSub><m:e>{base}</m:e><m:sub>{sub}</m:sub></m:sSub>"
                    ))
                }
                (None, Some(sup)) => {
                    if display
                        && attach.primes().is_none()
                        && let Some(op) = nary_op(&attach.base())
                    {
                        return Ok(nary(op, None, Some(&sup)));
                    }
                    Ok(format!(
                        "<m:sSup><m:e>{base}</m:e><m:sup>{sup}</m:sup></m:sSup>"
                    ))
                }
                (None, None) => Ok(base),
            }
        }
        ast::Expr::MathFrac(frac) => {
            let num = convert_math_expr(cx, frac.num(), face, false)?;
            let den = convert_math_expr(cx, frac.denom(), face, false)?;
            Ok(format!(
                "<m:f><m:num>{num}</m:num><m:den>{den}</m:den></m:f>"
            ))
        }
        ast::Expr::MathRoot(root) => {
            let radicand = convert_math_expr(cx, root.radicand(), face, display)?;
            match root.index() {
                // A bare square root hides an empty degree, which the
                // schema still requires.
                None => Ok(format!(
                    "<m:rad><m:radPr><m:degHide m:val=\"1\"/></m:radPr><m:deg/><m:e>{radicand}</m:e></m:rad>"
                )),
                Some(n) => Ok(format!(
                    "<m:rad><m:deg>{}</m:deg><m:e>{radicand}</m:e></m:rad>",
                    run(&n.to_string(), face)
                )),
            }
        }
        ast::Expr::MathDelimited(delimited) => {
            let open = delim_char(cx, delimited.open())?;
            let close = delim_char(cx, delimited.close())?;
            let body = convert_math(cx, delimited.body(), face, display)?;
            Ok(format!(
                "<m:d><m:dPr><m:begChr m:val=\"{open}\"/><m:endChr m:val=\"{close}\"/></m:dPr><m:e>{body}</m:e></m:d>"
            ))
        }
        ast::Expr::MathCall(call) => convert_math_call(cx, call, face, display),
        ast::Expr::MathPrimes(_) => Err(unsupported(cx, expr, "a prime without a base")),
        ast::Expr::MathAlignPoint(_) => Err(unsupported(cx, expr, "an aligned equation")),
        ast::Expr::Linebreak(_) => Err(unsupported(cx, expr, "a multiline equation")),
        ast::Expr::Math(m) => convert_math(cx, m, face, display),
        ast::Expr::ContentBlock(block) => convert_math_group(cx, block, face, display),
        ast::Expr::Parenthesized(inner) => convert_math_expr(cx, inner.expr(), face, display),
        ast::Expr::Str(s) => Ok(run(
            &s.get(),
            Face {
                upright: true,
                ..face
            },
        )),
        ast::Expr::Int(n) => Ok(run(&n.get().to_string(), face)),
        ast::Expr::Float(n) => Ok(run(&n.get().to_string(), face)),
        _ => Err(unsupported(
            cx,
            expr,
            &format!("{} in math", crate::describe(expr)),
        )),
    }
}

/// A math call: fractions, roots, styled and literal text.
fn convert_math_call(
    cx: &Cx,
    call: ast::MathCall,
    face: Face,
    display: bool,
) -> Result<String, Error> {
    let name = match call.callee() {
        ast::MathAccess::MathIdent(id) => id.as_str().to_string(),
        _ => {
            return Err(unsupported_at(cx, call.span(), "a method call in math"));
        }
    };
    // Matrices read their own arguments: cells are positional and `;`
    // starts a new row.
    if matches!(
        name.as_str(),
        "mat" | "pmat" | "bmat" | "Bmat" | "vmat" | "Vmat"
    ) {
        return convert_matrix(cx, call, &name, face);
    }
    let mut args = Vec::new();
    for item in call.args().arg_items() {
        if item.ends_in_semicolon {
            return Err(unsupported_at(
                cx,
                call.span(),
                &format!("`{name}` over rows"),
            ));
        }
        match item.arg {
            ast::Arg::Pos(expr) => args.push(expr),
            _ => {
                return Err(unsupported_at(
                    cx,
                    call.span(),
                    &format!("`{name}` with named arguments"),
                ));
            }
        }
    }
    match name.as_str() {
        "frac" => match args.as_slice() {
            [num, den] => {
                let num = math_arg(cx, *num, face, false)?;
                let den = math_arg(cx, *den, face, false)?;
                Ok(format!(
                    "<m:f><m:num>{num}</m:num><m:den>{den}</m:den></m:f>"
                ))
            }
            _ => Err(unsupported_at(
                cx,
                call.span(),
                "a fraction without two parts",
            )),
        },
        "sqrt" => match args.as_slice() {
            [body] => {
                let body = math_arg(cx, *body, face, display)?;
                Ok(format!(
                    "<m:rad><m:radPr><m:degHide m:val=\"1\"/></m:radPr><m:deg/><m:e>{body}</m:e></m:rad>"
                ))
            }
            _ => Err(unsupported_at(cx, call.span(), "a root without a radicand")),
        },
        "root" => match args.as_slice() {
            [index, body] => {
                let index = math_arg(cx, *index, face, display)?;
                let body = math_arg(cx, *body, face, display)?;
                Ok(format!(
                    "<m:rad><m:deg>{index}</m:deg><m:e>{body}</m:e></m:rad>"
                ))
            }
            _ => Err(unsupported_at(cx, call.span(), "a root without an index")),
        },
        "lr" => match args.as_slice() {
            [body] => match body {
                ast::Expr::MathDelimited(_) => convert_math_expr(cx, *body, face, display),
                _ => Err(unsupported_at(cx, call.span(), "a bare sided delimiter")),
            },
            _ => Err(unsupported_at(cx, call.span(), "`lr` with options")),
        },
        "text" => match args.as_slice() {
            [ast::Expr::Str(s)] => Ok(run(
                &s.get(),
                Face {
                    upright: true,
                    ..face
                },
            )),
            [body] => convert_math_expr(
                cx,
                *body,
                Face {
                    upright: true,
                    ..face
                },
                display,
            ),
            _ => Err(unsupported_at(cx, call.span(), "text without content")),
        },
        "bold" => match args.as_slice() {
            [body] => convert_math_expr(cx, *body, Face { bold: true, ..face }, display),
            _ => Err(unsupported_at(cx, call.span(), "bold without content")),
        },
        "upright" => match args.as_slice() {
            [body] => convert_math_expr(
                cx,
                *body,
                Face {
                    upright: true,
                    ..face
                },
                display,
            ),
            _ => Err(unsupported_at(cx, call.span(), "upright without content")),
        },
        "italic" => match args.as_slice() {
            // Italics are the math default.
            [body] => convert_math_expr(cx, *body, face, display),
            _ => Err(unsupported_at(cx, call.span(), "italic without content")),
        },
        "hat" | "tilde" | "dot" | "ddot" | "bar" | "arrow" => match args.as_slice() {
            [body] => {
                let body = math_arg(cx, *body, face, display)?;
                let chr = match name.as_str() {
                    // Combining circumflex, tilde, dot, diaeresis, macron.
                    "hat" => "\u{302}",
                    "tilde" => "\u{303}",
                    "dot" => "\u{307}",
                    "ddot" => "\u{308}",
                    "bar" => "\u{304}",
                    _ => "\u{20d7}",
                };
                Ok(format!(
                    "<m:acc><m:accPr><m:chr m:val=\"{chr}\"/></m:accPr><m:e>{body}</m:e></m:acc>"
                ))
            }
            _ => Err(unsupported_at(cx, call.span(), "an accent without a base")),
        },
        _ => Err(unsupported_at(
            cx,
            call.span(),
            &format!("`{name}` in math"),
        )),
    }
}

/// Markup content as math, for grouped call arguments.
/// A grouped argument as math: plain text re-parsed as math, so
/// `frac([x + y], [2])` numbers like `frac(x + y, 2)`. Anything else
/// keeps its own error.
fn convert_math_group(
    cx: &Cx,
    block: ast::ContentBlock,
    face: Face,
    display: bool,
) -> Result<String, Error> {
    let mut text = String::new();
    for expr in block.body().exprs() {
        match expr {
            ast::Expr::Text(t) => text.push_str(t.get()),
            ast::Expr::Space(_) => text.push(' '),
            _ => {
                return Err(unsupported_at(
                    cx,
                    expr.span(),
                    &format!("{} in math", describeish_math(&expr)),
                ));
            }
        }
    }
    let root = typst_syntax::parse_math(&text);
    let (errors, warnings) = root.errors_and_warnings();
    if let Some(first) = errors.first().or(warnings.first()) {
        // Lines count from the group's own line.
        let offset = match first.span.get() {
            typst_syntax::DiagSpanKind::Range { range, .. } => {
                text.get(..range.start).unwrap_or("").matches('\n').count()
            }
            _ => 0,
        };
        return Err(Error::Syntax {
            line: line_of(cx, block.span()) + offset,
            message: first.message.to_string(),
        });
    }
    let math = root
        .cast::<ast::Math>()
        .expect("a parsed fragment roots in math");
    // The fragment's spans are detached: they cannot point back into
    // the source, so an unsupported construct reports the group's
    // line instead of a bogus line 1.
    match convert_math(cx, math, face, display) {
        Err(Error::Unsupported { what, .. }) => Err(unsupported_at(cx, block.span(), &what)),
        result => result,
    }
}

/// A `mat` call and its delimited variants: positional cells with
/// `;` starting a new row. Cell content never takes display limits.
fn convert_matrix(cx: &Cx, call: ast::MathCall, name: &str, face: Face) -> Result<String, Error> {
    let mut rows: Vec<Vec<String>> = vec![Vec::new()];
    for item in call.args().arg_items() {
        match item.arg {
            ast::Arg::Pos(expr) => {
                let cell = math_arg(cx, expr, face, false)?;
                rows.last_mut().expect("a current row").push(cell);
            }
            ast::Arg::Named(named) => {
                return Err(unsupported_at(
                    cx,
                    named.span(),
                    &format!("a `{}` matrix argument", named.name().as_str()),
                ));
            }
            _ => {
                return Err(unsupported_at(cx, call.span(), "spread matrix arguments"));
            }
        }
        if item.ends_in_semicolon {
            rows.push(Vec::new());
        }
    }
    while rows.len() > 1 && rows.last().is_some_and(Vec::is_empty) {
        rows.pop();
    }
    if rows.iter().all(|row| row.is_empty()) {
        return Err(unsupported_at(cx, call.span(), "an empty matrix"));
    }
    let width = rows[0].len();
    if width == 0 || rows.iter().any(|row| row.len() != width) {
        return Err(unsupported_at(cx, call.span(), "a ragged matrix"));
    }
    let mut out = String::from("<m:m>");
    for row in &rows {
        out.push_str("<m:mr>");
        for cell in row {
            out.push_str(&format!("<m:e>{cell}</m:e>"));
        }
        out.push_str("</m:mr>");
    }
    out.push_str("</m:m>");
    // Bare matrices default to parentheses, like `pmat`.
    let (open, close) = match name {
        "mat" | "pmat" => ("(", ")"),
        "bmat" => ("[", "]"),
        "Bmat" => ("{", "}"),
        "vmat" => ("|", "|"),
        _ => ("‖", "‖"),
    };
    Ok(format!(
        "<m:d><m:dPr><m:begChr m:val=\"{open}\"/><m:endChr m:val=\"{close}\"/></m:dPr><m:e>{out}</m:e></m:d>"
    ))
}

fn describeish_math(expr: &ast::Expr) -> String {
    match expr {
        ast::Expr::Strong(_) => "bold text".to_string(),
        ast::Expr::Emph(_) => "italic text".to_string(),
        ast::Expr::Link(_) => "a link".to_string(),
        ast::Expr::ContentBlock(_) => "a nested group".to_string(),
        ast::Expr::Raw(raw) if raw.block() => "a code block".to_string(),
        _ => crate::describe(*expr),
    }
}

/// A call argument as math: grouped content or a bare expression.
fn math_arg(cx: &Cx, expr: ast::Expr, face: Face, display: bool) -> Result<String, Error> {
    match expr {
        ast::Expr::ContentBlock(block) => convert_math_group(cx, block, face, display),
        _ => convert_math_expr(cx, expr, face, display),
    }
}

/// A delimiter character, escaped for an attribute value.
fn delim_char(cx: &Cx, expr: ast::Expr) -> Result<String, Error> {
    let text = match expr {
        ast::Expr::MathText(t) => math_text(t),
        ast::Expr::MathShorthand(s) => s.get().to_string(),
        _ => return Err(unsupported(cx, expr, "an undelimited group")),
    };
    Ok(escape(&text).replace('"', "&quot;"))
}

/// Math text with Typst's hyphen standing in for a minus.
fn math_text(t: ast::MathText) -> String {
    let s = match t.get() {
        MathTextKind::Grapheme(s) | MathTextKind::Number(s) => s.to_string(),
    };
    s.replace('-', "\u{2212}")
}

/// A run of math text as Word writes it: bold is a style, so bold
/// italics take `bi` while bold upright text keeps `nor` and borrows
/// Word's own bold in a sibling `w:rPr`.
fn run(text: &str, face: Face) -> String {
    let props = match (face.bold, face.upright) {
        (true, false) => "<m:sty m:val=\"bi\"/>".to_string(),
        (_, true) => "<m:nor/>".to_string(),
        (false, false) => String::new(),
    };
    let props = if props.is_empty() {
        String::new()
    } else {
        format!("<m:rPr>{props}</m:rPr>")
    };
    let wprops = if face.bold && face.upright {
        "<w:rPr><w:b/></w:rPr>"
    } else {
        ""
    };
    format!(
        "<m:r>{props}{wprops}<m:t xml:space=\"preserve\">{}</m:t></m:r>",
        escape(text)
    )
}

/// XML-escapes run text.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

/// A dotted symbol access as one name.
fn dotted_name(access: ast::MathFieldAccess) -> String {
    let mut parts = vec![access.field().as_str().to_string()];
    let mut target = access.target();
    loop {
        match target {
            ast::MathAccess::MathIdent(id) => {
                parts.push(id.as_str().to_string());
                break;
            }
            ast::MathAccess::MathFieldAccess(nested) => {
                parts.push(nested.field().as_str().to_string());
                target = nested.target();
            }
        }
    }
    parts.reverse();
    parts.join(".")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Error, parse};
    use nebu_docx::content::{Block, Inline};

    fn math_of(source: &str) -> Math {
        let content = parse(source).unwrap();
        assert_eq!(content.blocks.len(), 1, "{source:?}");
        match &content.blocks[0] {
            Block::Paragraph { content, .. } => {
                assert_eq!(content.len(), 1, "{source:?}: {content:?}");
                match &content[0] {
                    Inline::Math { math } => math.clone(),
                    other => panic!("inline math: {other:?}"),
                }
            }
            Block::Equation { math, .. } => math.clone(),
            other => panic!("a math block: {other:?}"),
        }
    }

    #[test]
    fn converts_atoms() {
        // Identifiers, numbers and operators become runs; a lone Greek
        // letter keeps the math italics.
        assert_eq!(
            math_of("$x$\n").omml,
            "<m:oMath><m:r><m:t xml:space=\"preserve\">x</m:t></m:r></m:oMath>"
        );
        let m = math_of("$alpha + 2$\n").omml;
        assert!(m.contains("<m:t xml:space=\"preserve\">α</m:t>"), "{m}");
        assert!(m.contains("<m:t xml:space=\"preserve\">2</m:t>"), "{m}");
        assert!(m.contains("<m:t xml:space=\"preserve\">+</m:t>"), "{m}");
    }

    #[test]
    fn sets_multi_letter_identifiers_upright() {
        let m = math_of("$sin x$\n").omml;
        assert!(
            m.contains("<m:r><m:rPr><m:nor/></m:rPr><m:t xml:space=\"preserve\">sin</m:t></m:r>"),
            "{m}"
        );
    }

    #[test]
    fn converts_sub_and_superscripts() {
        assert_eq!(
            math_of("$x_1$\n").omml,
            concat!(
                "<m:oMath>",
                "<m:sSub><m:e><m:r><m:t xml:space=\"preserve\">x</m:t></m:r></m:e>",
                "<m:sub><m:r><m:t xml:space=\"preserve\">1</m:t></m:r></m:sub></m:sSub>",
                "</m:oMath>",
            )
        );
        // Both scripts share one object, so the superscript sits above
        // the subscript instead of after it.
        assert_eq!(
            math_of("$x_1^2$\n").omml,
            concat!(
                "<m:oMath>",
                "<m:sSubSup><m:e><m:r><m:t xml:space=\"preserve\">x</m:t></m:r></m:e>",
                "<m:sub><m:r><m:t xml:space=\"preserve\">1</m:t></m:r></m:sub>",
                "<m:sup><m:r><m:t xml:space=\"preserve\">2</m:t></m:r></m:sup></m:sSubSup>",
                "</m:oMath>",
            )
        );
    }

    #[test]
    fn puts_primes_before_superscripts() {
        // A prime shares the superscript slot: before any explicit
        // superscript, or as the superscript when there is none.
        assert_eq!(
            math_of("$x'_1$\n").omml,
            concat!(
                "<m:oMath>",
                "<m:sSubSup><m:e><m:r><m:t xml:space=\"preserve\">x</m:t></m:r></m:e>",
                "<m:sub><m:r><m:t xml:space=\"preserve\">1</m:t></m:r></m:sub>",
                "<m:sup><m:r><m:t xml:space=\"preserve\">′</m:t></m:r></m:sup></m:sSubSup>",
                "</m:oMath>",
            )
        );
        assert_eq!(
            math_of("$x'_1^2$\n").omml,
            concat!(
                "<m:oMath>",
                "<m:sSubSup><m:e><m:r><m:t xml:space=\"preserve\">x</m:t></m:r></m:e>",
                "<m:sub><m:r><m:t xml:space=\"preserve\">1</m:t></m:r></m:sub>",
                "<m:sup><m:r><m:t xml:space=\"preserve\">′</m:t></m:r>",
                "<m:r><m:t xml:space=\"preserve\">2</m:t></m:r></m:sup></m:sSubSup>",
                "</m:oMath>",
            )
        );
    }

    #[test]
    fn converts_matrices_and_accents() {
        // A 2x2 matrix with a delimited variant.
        let content = parse("$mat(a, b; c, d) + pmat(x; y)$\n").unwrap();
        assert_eq!(content.blocks.len(), 1, "{content:?}");
        let Block::Paragraph { content, .. } = &content.blocks[0] else {
            panic!("a paragraph: {content:?}");
        };
        let Some(Inline::Math { math }) = content.iter().find_map(|i| match i {
            m @ Inline::Math { .. } => Some(m),
            _ => None,
        }) else {
            panic!("inline math: {content:?}");
        };
        assert!(math.omml.contains("<m:m><m:mr>"), "{:?}", math.omml,);
        assert_eq!(
            math.omml.matches("<m:begChr").count(),
            2,
            "bare mat and pmat both parenthesize: {math:?}"
        );
        // An accented base carries the combining mark.
        let content = parse("$hat(x) + tilde(y)$\n").unwrap();
        let Block::Paragraph { content, .. } = &content.blocks[0] else {
            panic!("a paragraph: {content:?}");
        };
        let Some(Inline::Math { math }) = content.iter().find_map(|i| match i {
            m @ Inline::Math { .. } => Some(m),
            _ => None,
        }) else {
            panic!("inline math: {content:?}");
        };
        assert!(math.omml.contains("<m:acc>"), "{}", math.omml);
        assert!(math.omml.contains("m:val=\"\u{302}\""), "{:?}", math.omml);
        assert!(math.omml.contains("m:val=\"\u{303}\""), "{:?}", math.omml);
        // `arrow` is the combining arrow; `vec` stays unimplemented.
        let content = parse("$arrow(x)$\n").unwrap();
        let Block::Paragraph { content, .. } = &content.blocks[0] else {
            panic!("a paragraph: {content:?}");
        };
        let Some(Inline::Math { math }) = content.iter().find_map(|i| match i {
            m @ Inline::Math { .. } => Some(m),
            _ => None,
        }) else {
            panic!("inline math: {content:?}");
        };
        assert!(math.omml.contains("m:val=\"\u{20d7}\""), "{:?}", math.omml);
        let err = parse("$vec(x)$\n").unwrap_err();
        assert_eq!(
            err,
            Error::Unsupported {
                line: 1,
                what: "`vec` in math".to_string()
            },
            "{err:?}"
        );
    }

    #[test]
    fn converts_dotted_n_ary_operators() {
        // Dotted Typst names resolve through the symbol table before
        // the n-ary check: a coproduct numbers like a sum.
        let content = parse("$ product.co_(i = 1)^n x_i $\n").unwrap();
        assert_eq!(content.blocks.len(), 1, "{content:?}");
        let Block::Equation { math, .. } = &content.blocks[0] else {
            panic!("a display equation: {content:?}");
        };
        assert!(math.omml.contains("<m:nary>"), "{}", math.omml);
        assert!(math.omml.contains("<m:chr m:val=\"∐\"/>"), "{}", math.omml);
        assert_eq!(math.omml.matches('∐').count(), 1, "{}", math.omml);
    }

    #[test]
    fn rejects_ragged_and_empty_matrices() {
        for (source, what) in [
            ("$mat(a, b; c)$\n", "a ragged matrix"),
            ("$mat()$\n", "an empty matrix"),
        ] {
            let err = parse(source).unwrap_err();
            assert_eq!(
                err,
                Error::Unsupported {
                    line: 1,
                    what: what.to_string()
                },
                "{source:?}"
            );
        }
    }

    #[test]
    fn hides_the_missing_side_of_display_n_ary() {
        // A display sum with only a subscript still numbers as one
        // operator: the missing superscript hides behind an empty tag.
        let content = parse("$ sum_x $\n").unwrap();
        let Block::Equation { math, .. } = &content.blocks[0] else {
            panic!("a display equation: {content:?}");
        };
        assert!(math.omml.contains("<m:nary>"), "{:?}", math.omml);
        assert!(
            math.omml.contains("<m:supHide m:val=\"1\"/>"),
            "{:?}",
            math.omml
        );
        assert!(math.omml.contains("<m:sup/>"), "{:?}", math.omml);
    }

    #[test]
    fn converts_display_sums_with_limits_below() {
        // A display sum puts its limits below and above the operator.
        let content = parse("$ sum_(i = 1)^n x_i $\n").unwrap();
        assert_eq!(content.blocks.len(), 1, "{content:?}");
        let Block::Equation { math, .. } = &content.blocks[0] else {
            panic!("a display equation: {content:?}");
        };
        assert!(math.omml.contains("<m:nary>"), "{}", math.omml);
        assert!(
            math.omml.contains("<m:limLoc m:val=\"undOvr\"/>"),
            "{}",
            math.omml
        );
        assert!(!math.omml.contains("<m:sSubSup>"), "{}", math.omml);
        // The operand slot stays empty: the glyph lives only in m:chr
        // and the operand follows as a sibling.
        assert!(math.omml.contains("<m:e/>"), "{}", math.omml);
        assert_eq!(math.omml.matches('∑').count(), 1, "{}", math.omml);
        // The same sum inline keeps beside-scripts.
        let content = parse("$sum_(i = 1)^n x_i$\n").unwrap();
        assert_eq!(content.blocks.len(), 1, "{content:?}");
        let Block::Paragraph { content, .. } = &content.blocks[0] else {
            panic!("a paragraph: {content:?}");
        };
        let Inline::Math { math } = &content[0] else {
            panic!("inline math: {content:?}");
        };
        assert!(math.omml.contains("<m:sSubSup>"), "{}", math.omml);
        assert!(!math.omml.contains("<m:nary>"), "{}", math.omml);
    }

    #[test]
    fn converts_fractions() {
        // The numerator stays on top: exact bytes, not just tags.
        assert_eq!(
            math_of("$a/b$\n").omml,
            concat!(
                "<m:oMath>",
                "<m:f><m:num><m:r><m:t xml:space=\"preserve\">a</m:t></m:r></m:num>",
                "<m:den><m:r><m:t xml:space=\"preserve\">b</m:t></m:r></m:den></m:f>",
                "</m:oMath>",
            )
        );
        for source in ["$frac(a, b)$\n", "$frac([a + b], [2])$\n"] {
            let m = math_of(source).omml;
            assert!(m.contains("<m:f><m:num>"), "{source}: {m}");
            assert!(m.contains("</m:num><m:den>"), "{source}: {m}");
        }
    }

    #[test]
    fn converts_roots_and_delimiters() {
        // The √ symbol takes the same path as the call.
        let m = math_of("$\u{221a}x$\n").omml;
        assert!(
            m.contains("<m:rad><m:radPr><m:degHide m:val=\"1\"/></m:radPr><m:deg/>"),
            "{m}"
        );
        // A bare root keeps its hidden empty degree; a delimiter keeps
        // its properties before its content.
        assert_eq!(
            math_of("$sqrt(x)$\n").omml,
            concat!(
                "<m:oMath>",
                "<m:rad><m:radPr><m:degHide m:val=\"1\"/></m:radPr><m:deg/>",
                "<m:e><m:r><m:t xml:space=\"preserve\">x</m:t></m:r></m:e></m:rad>",
                "</m:oMath>",
            )
        );
        let m = math_of("$root(3, x)$\n").omml;
        assert!(
            m.contains("<m:deg><m:r><m:t xml:space=\"preserve\">3</m:t></m:r></m:deg>"),
            "{m}"
        );
        assert_eq!(
            math_of("$(a)$\n").omml,
            concat!(
                "<m:oMath>",
                "<m:d><m:dPr><m:begChr m:val=\"(\"/><m:endChr m:val=\")\"/></m:dPr>",
                "<m:e><m:r><m:t xml:space=\"preserve\">a</m:t></m:r></m:e></m:d>",
                "</m:oMath>",
            )
        );
    }

    #[test]
    fn bolds_with_style_not_toggles() {
        // Bold italics take the `bi` style; bold upright text keeps
        // `nor` and borrows Word's bold.
        let m = math_of("$bold(x)$\n").omml;
        assert!(m.contains("<m:rPr><m:sty m:val=\"bi\"/></m:rPr>"), "{m}");
        assert!(!m.contains("<m:b/>"), "{m}");
        let m = math_of("$bold(sin)$\n").omml;
        assert!(
            m.contains("<m:rPr><m:nor/></m:rPr><w:rPr><w:b/></w:rPr>"),
            "{m}"
        );
    }

    #[test]
    fn converts_a_labelled_display_equation() {
        let content = parse("$ E = m c^2 $\n<eq-mass>\n").unwrap();
        assert_eq!(content.blocks.len(), 1, "{content:?}");
        match &content.blocks[0] {
            Block::Equation { math, label } => {
                assert_eq!(label.as_deref(), Some("eq-mass"));
                assert!(math.omml.contains('E'), "{}", math.omml);
            }
            other => panic!("a display equation: {other:?}"),
        }
        assert_eq!(parse("$ E = m c^2 $\n\n").unwrap().blocks.len(), 1);
    }

    #[test]
    fn rejects_what_math_cannot_hold() {
        for (source, line, what) in [
            ("$mat(a, b; c)$\n", 1, "a ragged matrix"),
            (
                "$table(columns: 1, [a])$\n",
                1,
                "`table` with named arguments",
            ),
            ("$a & b$\n", 1, "an aligned equation"),
            ("$lr(x)$\n", 1, "a bare sided delimiter"),
        ] {
            let err = parse(source).unwrap_err();
            assert_eq!(
                err,
                Error::Unsupported {
                    line,
                    what: what.to_string()
                },
                "{source:?}"
            );
        }
        let err = parse("$x$\n$y\n").unwrap_err();
        assert!(matches!(err, Error::Syntax { line: 2, .. }), "{err:?}");
    }

    #[test]
    fn reports_grouped_unsupported_at_the_groups_line() {
        // An explicit content group is re-parsed as math, so its
        // inner spans are detached: an unsupported construct inside
        // it reports the group's line, not line 1.
        for source in [
            "Intro\n\n$frac([mat(a, b; c)], [2])$\n",
            "Intro\n\n$frac(#[mat(a, b; c)], [2])$\n",
        ] {
            let err = parse(source).unwrap_err();
            assert_eq!(
                err,
                Error::Unsupported {
                    line: 3,
                    what: "a ragged matrix".to_string()
                },
                "{source:?}: {err:?}"
            );
        }
    }
}
