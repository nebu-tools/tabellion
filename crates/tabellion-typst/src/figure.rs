//! `#image` calls and `#figure` wrappers with image bodies.
//!
//! An image names its file, resolved when the content is written, with
//! an alt text and absolute dimensions in points. Anything computed,
//! relative or unknown is an error naming the argument and the line.

use nebu_docx::content::{Block, Image, Inline, Source};
use typst_syntax::ast::{self, AstNode, Unit};

use super::table::{convert_table, describeish};
use super::{Cx, Error, inline_markup, unsupported, unsupported_at};

/// Whether a call is a bare `#image`.
pub(crate) fn is_image_call(call: ast::FuncCall) -> bool {
    matches!(call.callee(), ast::Expr::Ident(id) if id.as_str() == "image")
}

/// An `#image` call as a content image.
pub(crate) fn convert_image(cx: &Cx, call: ast::FuncCall) -> Result<Image, Error> {
    let mut path: Option<String> = None;
    let mut alt = None;
    let mut width = None;
    let mut height = None;
    for arg in call.args().items() {
        match arg {
            ast::Arg::Pos(ast::Expr::Str(s)) if path.is_none() => {
                path = Some(s.get().to_string());
            }
            ast::Arg::Pos(expr) => {
                if path.is_some() {
                    return Err(unsupported(cx, expr, "an image with two sources"));
                }
                return Err(unsupported(cx, expr, "an image without a file"));
            }
            ast::Arg::Named(named) if named.name().as_str() == "alt" => match named.expr() {
                ast::Expr::Str(s) => alt = Some(s.get().to_string()),
                _ => {
                    return Err(unsupported_at(
                        cx,
                        named.span(),
                        "an alt text that is not text",
                    ));
                }
            },
            ast::Arg::Named(named) if matches!(named.name().as_str(), "width" | "height") => {
                let points = match named.expr() {
                    ast::Expr::Auto(_) => None,
                    other => Some(length_points(cx, other)?),
                };
                if named.name().as_str() == "width" {
                    width = points;
                } else {
                    height = points;
                }
            }
            ast::Arg::Named(named) => {
                return Err(unsupported_at(
                    cx,
                    named.span(),
                    &format!("a `{}` image argument", named.name().as_str()),
                ));
            }
            ast::Arg::Spread(_) => {
                return Err(unsupported(
                    cx,
                    ast::Expr::FuncCall(call),
                    "spread image arguments",
                ));
            }
        }
    }
    let Some(path) = path else {
        return Err(unsupported(
            cx,
            ast::Expr::FuncCall(call),
            "an image without a file",
        ));
    };
    Ok(Image {
        source: Source::File(path),
        fallback: None,
        width,
        height,
        alt,
        title: None,
    })
}

/// An absolute length in points.
fn length_points(cx: &Cx, expr: ast::Expr) -> Result<f64, Error> {
    match expr {
        ast::Expr::Numeric(n) => match n.get() {
            (v, Unit::Pt) => Ok(v),
            (v, Unit::Mm) => Ok(v * 72.0 / 25.4),
            (v, Unit::Cm) => Ok(v * 72.0 / 2.54),
            (v, Unit::In) => Ok(v * 72.0),
            _ => Err(unsupported(cx, expr, "a relative image size")),
        },
        _ => Err(unsupported(cx, expr, "a complex image size")),
    }
}

/// A `#figure` around a table or an image. An omitted caption is
/// not an empty one: without a caption the Word form of an image
/// figure is a bare image, and a label has nowhere to attach.
pub(crate) fn convert_figure(
    cx: &Cx,
    call: ast::FuncCall,
    sibling: Option<(String, typst_syntax::Span)>,
) -> Result<Block, Error> {
    let mut body: Option<ast::Expr> = None;
    let mut caption: Option<Vec<Inline>> = None;
    let mut label: Option<(String, typst_syntax::Span)> = None;
    for arg in call.args().items() {
        match arg {
            ast::Arg::Pos(expr) => {
                if body.is_some() {
                    return Err(unsupported(cx, expr, "a figure with two bodies"));
                }
                body = Some(expr);
            }
            ast::Arg::Named(named) if named.name().as_str() == "caption" => match named.expr() {
                ast::Expr::ContentBlock(block) => {
                    let mut content = Vec::new();
                    inline_markup(
                        cx,
                        block.body(),
                        &mut content,
                        Default::default(),
                        "a caption",
                    )?;
                    caption = Some(content);
                }
                ast::Expr::Str(s) => {
                    caption = Some(vec![Inline::text(&s.get())]);
                }
                _ => {
                    return Err(unsupported(cx, named.expr(), "a computed caption"));
                }
            },
            ast::Arg::Named(named) if named.name().as_str() == "label" => {
                if let ast::Expr::Label(l) = named.expr() {
                    label = Some((l.get().to_string(), l.span()));
                } else {
                    return Err(unsupported(cx, named.expr(), "a computed label"));
                }
            }
            ast::Arg::Named(named) => {
                return Err(unsupported_at(
                    cx,
                    named.span(),
                    &format!("a `{}` figure argument", named.name().as_str()),
                ));
            }
            ast::Arg::Spread(_) => {
                return Err(unsupported(
                    cx,
                    ast::Expr::FuncCall(call),
                    "spread figure arguments",
                ));
            }
        }
    }
    if let Some((name, at)) = sibling {
        if label.is_some() {
            return Err(unsupported_at(cx, at, "two labels for a figure"));
        }
        label = Some((name, at));
    }
    let Some(body) = body else {
        return Err(unsupported(
            cx,
            ast::Expr::FuncCall(call),
            "an empty figure",
        ));
    };
    match body {
        ast::Expr::FuncCall(inner) if matches!(inner.callee(), ast::Expr::Ident(id) if id.as_str() == "table") =>
        {
            let mut table = convert_table(cx, inner)?;
            table.caption = caption;
            if let Some((name, at)) = label {
                if table.caption.is_none() {
                    return Err(unsupported_at(cx, at, "a captionless labeled figure"));
                }
                table.label = Some(name);
            }
            Ok(Block::Table(table))
        }
        ast::Expr::FuncCall(inner) if matches!(inner.callee(), ast::Expr::Ident(id) if id.as_str() == "image") =>
        {
            let image = convert_image(cx, inner)?;
            match (caption, label) {
                (None, None) => Ok(Block::Image { image }),
                (None, Some((_, at))) => {
                    Err(unsupported_at(cx, at, "a captionless labeled figure"))
                }
                (Some(caption), label) => Ok(Block::Figure {
                    image,
                    caption,
                    label: label.map(|(name, _)| name),
                }),
            }
        }
        ast::Expr::FuncCall(_) => Err(unsupported(
            cx,
            body,
            &format!("a figure around {}", describeish(body)),
        )),
        _ => Err(unsupported(cx, body, "a figure around content")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn figure_block(source: &str) -> Block {
        let content = parse(source).unwrap();
        assert_eq!(content.blocks.len(), 1, "{source:?}");
        content.blocks.into_iter().next().unwrap()
    }

    #[test]
    fn keeps_a_mid_paragraph_image_inline() {
        // Text on both sides shares the line: one paragraph with an
        // inline image, not three blocks.
        let content = parse("Before #image(\"wren.png\") after\n").unwrap();
        assert_eq!(content.blocks.len(), 1);
        let Block::Paragraph { content, .. } = &content.blocks[0] else {
            panic!("a paragraph: {:?}", content.blocks[0]);
        };
        assert_eq!(content.len(), 3, "{content:?}");
        assert!(matches!(
            &content[1],
            Inline::Image { image } if matches!(&image.source, Source::File(path) if path == "wren.png")
        ));
    }

    #[test]
    fn reads_a_standalone_image_as_a_block() {
        let content = parse("#image(\"wren.png\")\n").unwrap();
        assert_eq!(content.blocks.len(), 1);
        let Block::Image { image } = &content.blocks[0] else {
            panic!("an image block: {:?}", content.blocks[0]);
        };
        assert!(matches!(
            &image.source,
            Source::File(path) if path == "wren.png"
        ));
    }

    #[test]
    fn reads_an_image_figure() {
        let Block::Figure {
            image,
            caption,
            label,
            ..
        } = figure_block(
            "#figure(image(\"wren.png\", alt: \"A wren\", width: 12pt), caption: [A wren]) <fig-wren>\n",
        )
        else {
            panic!("a figure");
        };
        assert_eq!(label.as_deref(), Some("fig-wren"));
        assert!(matches!(
            image.source,
            Source::File(ref path) if path == "wren.png"
        ));
        assert_eq!(image.alt.as_deref(), Some("A wren"));
        assert_eq!(image.width, Some(12.0));
        assert_eq!(caption.len(), 1);
    }

    #[test]
    fn converts_lengths_to_points() {
        let Block::Image { image } =
            figure_block("#figure(image(\"w.png\", width: 2.54cm, height: 1in))\n")
        else {
            panic!("an image");
        };
        assert!((image.width.unwrap() - 72.0).abs() < 1e-6);
        assert!((image.height.unwrap() - 72.0).abs() < 1e-6);
        // `auto` means the natural size: no width is set.
        let Block::Image { image } = figure_block("#figure(image(\"w.png\", width: auto))\n")
        else {
            panic!("an image");
        };
        assert_eq!(image.width, None);
    }

    #[test]
    fn maps_captionless_figures_without_labels_to_images() {
        // No caption means no numbered figure: a plain image block.
        let content = parse("#figure(image(\"w.png\"))\n").unwrap();
        assert_eq!(content.blocks.len(), 1);
        let Block::Image { image } = &content.blocks[0] else {
            panic!("an image block: {:?}", content.blocks[0]);
        };
        assert!(matches!(
            &image.source,
            Source::File(path) if path == "w.png"
        ));
    }

    #[test]
    fn rejects_captionless_labeled_figures() {
        // A label has nowhere to attach without a visible caption.
        for source in [
            "#figure(image(\"w.png\")) <fig-w>\n",
            "#figure(image(\"w.png\"), label: <fig-w>)\n",
            "#figure(table(columns: 1, [a])) <tab-x>\n",
        ] {
            let err = parse(source).unwrap_err();
            assert_eq!(
                err,
                Error::Unsupported {
                    line: 1,
                    what: "a captionless labeled figure".to_string()
                },
                "{source:?}"
            );
        }
    }

    #[test]
    fn rejects_what_images_cannot_hold() {
        for (source, line, what) in [
            ("#image()\n", 1, "an image without a file"),
            ("#image(1)\n", 1, "an image without a file"),
            (
                "#image(\"p.png\", fit: cover)\n",
                1,
                "a `fit` image argument",
            ),
            (
                "#image(\"p.png\", width: 50%)\n",
                1,
                "a relative image size",
            ),
            (
                "#image(\"p.png\", width: 2em)\n",
                1,
                "a relative image size",
            ),
            (
                "#image(\"p.png\", alt: 1)\n",
                1,
                "an alt text that is not text",
            ),
            (
                "#figure(table(columns: 1, [a]), table(columns: 1, [b]))\n",
                1,
                "a figure with two bodies",
            ),
            (
                "#figure(table(columns: 1, [a]), numbering: \"1\")\n",
                1,
                "a `numbering` figure argument",
            ),
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
    }
}
