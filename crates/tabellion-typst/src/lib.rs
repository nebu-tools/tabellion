//! Typst source as [`Content`]: headings, paragraphs, lists and page
//! breaks that [`Document::insert`] writes into a document.
//!
//! [`Content`]: nebu_docx::Content
//! [`Document::insert`]: nebu_docx::Document::insert
//!
//! Only the Typst constructs with a content-model form are read: text
//! with strong, emphasized, monospaced, sub/superscript, struck and
//! underlined marks, links and cross-references, headings (levels 1
//! to 9, with one label), bulleted and numbered lists with nesting,
//! and page breaks. Anything else is an error naming the construct
//! and the line where it stands.

use nebu_docx::content::{Block, Content, Inline, LinkTarget, List, ListItem, Marks};
use thiserror::Error;
use typst_syntax::{DiagSpanKind, Source, ast, ast::AstNode};

mod table;
use table::{convert_tableish, is_tableish};

/// What is wrong with Typst source.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Error {
    /// The source does not parse, with the first message.
    #[error("line {line}: invalid Typst: {message}")]
    Syntax {
        /// 1-based line number.
        line: usize,
        /// What the parser reports.
        message: String,
    },
    /// A construct with no content-model form.
    #[error("line {line}: {what} has no Word form")]
    Unsupported {
        /// 1-based line number of the construct.
        line: usize,
        /// What the construct is, such as `` `#table` ``.
        what: String,
    },
}

/// Reads Typst source as content blocks.
pub fn parse(source: &str) -> Result<Content, Error> {
    let src = Source::detached(source.to_string());
    let root = src.root();
    // Errors and warnings both stop the conversion: a warning like
    // "no text within stars" marks content the tree drops.
    let (errors, warnings) = root.errors_and_warnings();
    if let Some(first) = errors.first().or(warnings.first()) {
        return Err(syntax_error(&src, first));
    }
    let markup = root
        .cast::<ast::Markup>()
        .expect("a parsed source roots in markup");
    let cx = Cx { source: &src };
    let exprs: Vec<ast::Expr> = markup.exprs().collect();
    let (blocks, _) = convert_flow(&cx, &exprs, true)?;
    Ok(Content {
        blocks,
        table_styles: Vec::new(),
    })
}

/// What the converter knows: the source, for line numbers.
pub(crate) struct Cx<'a> {
    pub(crate) source: &'a Source,
}

/// A syntax error with its line.
fn syntax_error(source: &Source, e: &typst_syntax::SyntaxDiagnostic) -> Error {
    let range = match e.span.get() {
        DiagSpanKind::Number { id, num, sub_range } if id == source.id() => {
            source.range(num, sub_range)
        }
        DiagSpanKind::Range { id, range } if id == source.id() => Some(range),
        _ => None,
    };
    let line = range
        .and_then(|r| source.lines().byte_to_line_column(r.start))
        .map(|(l, _)| l + 1)
        .unwrap_or(1);
    Error::Syntax {
        line,
        message: e.message.to_string(),
    }
}

/// The line where a node stands, 1-based.
fn line_of(cx: &Cx, span: typst_syntax::Span) -> usize {
    cx.source
        .find(span)
        .map(|n| n.range().start)
        .and_then(|at| cx.source.lines().byte_to_line_column(at))
        .map(|(l, _)| l + 1)
        .unwrap_or(1)
}

/// An unsupported construct at this expression.
pub(crate) fn unsupported(cx: &Cx, at: ast::Expr, what: &str) -> Error {
    unsupported_at(cx, at.span(), what)
}

/// An unsupported construct at a span.
pub(crate) fn unsupported_at(cx: &Cx, span: typst_syntax::Span, what: &str) -> Error {
    Error::Unsupported {
        line: line_of(cx, span),
        what: what.to_string(),
    }
}

/// Pushes the paragraph if it holds anything. A trailing space from
/// the source newline is not content.
fn flush(para: &mut Vec<Inline>, blocks: &mut Vec<Block>, trim_end: bool) {
    if trim_end {
        trim_trailing_space(para);
    }
    if !para.is_empty() {
        blocks.push(Block::Paragraph {
            style: None,
            align: None,
            content: std::mem::take(para),
        });
    }
}

/// Drops a trailing space run: trailing source whitespace is not
/// content, in a paragraph, heading or list item alike.
fn trim_trailing_space(inlines: &mut Vec<Inline>) {
    if let Some(Inline::Text { text, .. }) = inlines.last_mut() {
        let trimmed = text.trim_end_matches([' ', '\t']).to_string();
        if trimmed.is_empty() {
            inlines.pop();
        } else {
            *text = trimmed;
        }
    }
}

/// Block positions: headings, lists, page breaks, tables and
/// paragraphs of inlines. Trailing source whitespace is trimmed only
/// at the top level, where a paragraph, heading or list item ends: an
/// inline group keeps its boundary spaces for the enclosing line.
/// Also reports whether source whitespace leads: a group starting in
/// whitespace passes one collapsed space to the enclosing line, so a
/// paragraph never starts padded and duplicate spaces collapse.
pub(crate) fn convert_flow(
    cx: &Cx,
    exprs: &[ast::Expr],
    trim_end: bool,
) -> Result<(Vec<Block>, bool), Error> {
    let mut blocks = Vec::new();
    let mut para: Vec<Inline> = Vec::new();
    let mut leading_space = false;
    let mut i = 0;
    while i < exprs.len() {
        match exprs[i] {
            ast::Expr::Parbreak(_) => {
                flush(&mut para, &mut blocks, trim_end);
            }
            ast::Expr::Space(_) => {
                if para.is_empty() && blocks.is_empty() {
                    leading_space = true;
                } else {
                    push_space(&mut para, &Marks::default());
                }
            }
            ast::Expr::Heading(h) => {
                flush(&mut para, &mut blocks, trim_end);
                let depth = h.depth().get();
                if depth > 9 {
                    return Err(unsupported(cx, exprs[i], "a deeper heading"));
                }
                let mut content = Vec::new();
                inline_markup(cx, h.body(), &mut content, Marks::default(), "a heading")?;
                trim_trailing_space(&mut content);
                // A label follows the heading, past any spaces. Spaces
                // without a label after them are no-ops on an empty
                // paragraph, so leaving them is harmless.
                let mut j = i + 1;
                while matches!(exprs.get(j), Some(ast::Expr::Space(_))) {
                    j += 1;
                }
                let mut label = None;
                if let Some(ast::Expr::Label(l)) = exprs.get(j) {
                    label = Some(l.get().to_string());
                    i = j;
                }
                blocks.push(Block::Heading {
                    level: depth as u8,
                    content,
                    label,
                });
            }
            ast::Expr::ListItem(_) | ast::Expr::EnumItem(_) => {
                flush(&mut para, &mut blocks, trim_end);
                let (list, n) = convert_list(cx, &exprs[i..])?;
                blocks.push(Block::List(list));
                i += n;
                continue;
            }
            ast::Expr::FuncCall(call) if is_pagebreak(call) => {
                flush(&mut para, &mut blocks, trim_end);
                blocks.push(Block::PageBreak);
            }
            ast::Expr::FuncCall(call) if is_tableish(call) => {
                flush(&mut para, &mut blocks, trim_end);
                let mut block = convert_tableish(cx, call)?;
                // A label follows the table, past any spaces.
                let mut j = i + 1;
                while matches!(exprs.get(j), Some(ast::Expr::Space(_))) {
                    j += 1;
                }
                if let (Some(ast::Expr::Label(l)), Block::Table(table)) = (exprs.get(j), &mut block)
                {
                    if table.label.is_some() {
                        return Err(unsupported(cx, exprs[j], "two labels for a table"));
                    }
                    table.label = Some(l.get().to_string());
                    i = j;
                }
                blocks.push(block);
            }
            ast::Expr::Raw(raw) if raw.block() => {
                flush(&mut para, &mut blocks, trim_end);
                blocks.push(Block::Paragraph {
                    style: None,
                    align: None,
                    content: vec![Inline::Text {
                        text: raw_lines(raw),
                        marks: Marks {
                            code: true,
                            ..Marks::default()
                        },
                    }],
                });
            }
            ast::Expr::Label(_) => {
                return Err(unsupported(cx, exprs[i], "a label outside a heading"));
            }
            ast::Expr::Ref(_)
            | ast::Expr::Link(_)
            | ast::Expr::Text(_)
            | ast::Expr::Escape(_)
            | ast::Expr::Shorthand(_)
            | ast::Expr::SmartQuote(_)
            | ast::Expr::Linebreak(_)
            | ast::Expr::Strong(_)
            | ast::Expr::Emph(_)
            | ast::Expr::Raw(_)
            | ast::Expr::FuncCall(_)
            | ast::Expr::ContentBlock(_)
            | ast::Expr::Parenthesized(_)
            | ast::Expr::Int(_)
            | ast::Expr::Float(_)
            | ast::Expr::Str(_)
            | ast::Expr::Bool(_)
            | ast::Expr::None(_) => {
                // A pending leading space belongs to the line start:
                // adopt it when nothing precedes it, drop it (as the
                // push above does) otherwise.
                let fresh = para.is_empty() && blocks.is_empty();
                if inline_into(cx, exprs[i], &mut para, Marks::default(), "the document")? && fresh
                {
                    leading_space = true;
                }
            }
            _ => {
                return Err(unsupported(cx, exprs[i], &describe(exprs[i])));
            }
        }
        i += 1;
    }
    flush(&mut para, &mut blocks, trim_end);
    Ok((blocks, leading_space))
}

/// Consecutive items of one kind: a bulleted or numbered list. The
/// writer numbers from 1, so an explicit number must be the next one
/// or the list is unrepresentable.
fn convert_list(cx: &Cx, exprs: &[ast::Expr]) -> Result<(List, usize), Error> {
    let ordered = matches!(exprs[0], ast::Expr::EnumItem(_));
    let mut items = Vec::new();
    let mut next = 1u64;
    let mut n = 0;
    loop {
        // Newlines separate items; a blank line ends the list.
        while matches!(exprs.get(n), Some(ast::Expr::Space(_))) {
            n += 1;
        }
        let body = match exprs.get(n) {
            Some(ast::Expr::ListItem(item)) if !ordered => item.body(),
            Some(ast::Expr::EnumItem(item)) if ordered => {
                match item.number() {
                    Some(k) if k != next => {
                        return Err(unsupported(
                            cx,
                            exprs[n],
                            &format!("list number {k} out of order"),
                        ));
                    }
                    Some(k) => next = k.saturating_add(1),
                    // `+` continues the count.
                    None => next = next.saturating_add(1),
                }
                item.body()
            }
            _ => break,
        };
        items.push(convert_item(cx, body)?);
        n += 1;
    }
    Ok((List { ordered, items }, n))
}

/// Whether an item-body expression after the nested list holds text
/// that the model would move before that list. Whitespace holds no
/// content, and block constructs keep their own errors.
fn emits_after_list(expr: &ast::Expr) -> bool {
    match expr {
        ast::Expr::Space(_)
        | ast::Expr::Label(_)
        | ast::Expr::Parbreak(_)
        | ast::Expr::Heading(_)
        | ast::Expr::TermItem(_)
        | ast::Expr::Equation(_)
        | ast::Expr::Math(_) => false,
        ast::Expr::Text(t) => !t.get().is_empty(),
        ast::Expr::None(_) => false,
        ast::Expr::Raw(raw) => !raw.block(),
        ast::Expr::FuncCall(call) => !is_pagebreak(*call),
        _ => true,
    }
}

/// One item: its text, and the one nested list the model holds.
fn convert_item(cx: &Cx, body: ast::Markup) -> Result<ListItem, Error> {
    let exprs: Vec<ast::Expr> = body.exprs().collect();
    let mut content = Vec::new();
    let mut list = None;
    let mut i = 0;
    while i < exprs.len() {
        match exprs[i] {
            ast::Expr::ListItem(_) | ast::Expr::EnumItem(_) => {
                if list.is_some() {
                    return Err(unsupported(cx, exprs[i], "a list item with two lists"));
                }
                // A newline before the nested list leaves a trailing
                // space that means nothing.
                trim_trailing_space(&mut content);
                let (sub, n) = convert_list(cx, &exprs[i..])?;
                list = Some(Box::new(sub));
                i += n;
                continue;
            }
            ast::Expr::Parbreak(_) => {
                return Err(unsupported(cx, exprs[i], "a list item with two paragraphs"));
            }
            _ => {
                if list.is_some() && emits_after_list(&exprs[i]) {
                    // The model writes the item's text before its nested
                    // list, so text after the list would move.
                    return Err(unsupported(cx, exprs[i], "text after a nested list"));
                }
                inline_into(cx, exprs[i], &mut content, Marks::default(), "a list item")?;
            }
        }
        i += 1;
    }
    trim_trailing_space(&mut content);
    Ok(ListItem { content, list })
}

/// The inlines of markup with marks on top. True when leading
/// whitespace is still owed to the enclosing line.
pub(crate) fn inline_markup(
    cx: &Cx,
    body: ast::Markup,
    out: &mut Vec<Inline>,
    marks: Marks,
    ctx: &str,
) -> Result<bool, Error> {
    // True when leading whitespace is still owed: an inner group found
    // an empty buffer. Nothing was emitted for it yet.
    let mut pending = false;
    for expr in body.exprs() {
        pending |= inline_into(cx, expr, out, marks.clone(), ctx)?;
    }
    Ok(pending)
}

/// One inline-capable expression. Block expressions are errors naming
/// where they stand: a heading, list or page break cannot hide in a
/// line of text.
///
/// Returns whether leading whitespace is still owed: a nested group
/// found an empty buffer, so the enclosing flow adopts it when it
/// starts its line, and drops it otherwise.
#[allow(clippy::too_many_lines)]
fn inline_into(
    cx: &Cx,
    expr: ast::Expr,
    out: &mut Vec<Inline>,
    marks: Marks,
    ctx: &str,
) -> Result<bool, Error> {
    match expr {
        ast::Expr::Text(t) => {
            push_text(out, t.get(), marks);
            Ok(false)
        }
        ast::Expr::Space(_) => {
            push_space(out, &marks);
            Ok(false)
        }
        ast::Expr::Linebreak(_) => {
            out.push(Inline::LineBreak);
            Ok(false)
        }
        ast::Expr::Escape(e) => {
            push_text(out, &e.get().to_string(), marks);
            Ok(false)
        }
        ast::Expr::Shorthand(s) => {
            push_text(out, &s.get().to_string(), marks);
            Ok(false)
        }
        ast::Expr::SmartQuote(q) => {
            push_text(out, if q.double() { "\"" } else { "'" }, marks);
            Ok(false)
        }
        ast::Expr::Strong(s) => {
            let mut inner = marks;
            inner.bold = true;
            Ok(inline_markup(cx, s.body(), out, inner, ctx)?)
        }
        ast::Expr::Emph(e) => {
            let mut inner = marks;
            inner.italic = true;
            Ok(inline_markup(cx, e.body(), out, inner, ctx)?)
        }
        ast::Expr::Raw(raw) if !raw.block() => {
            let mut inner = marks;
            inner.code = true;
            push_text(out, &raw_lines(raw), inner);
            Ok(false)
        }
        ast::Expr::Link(link) => {
            let url = link.get().to_string();
            out.push(Inline::Link {
                target: LinkTarget::Url(url.clone()),
                content: vec![Inline::Text { text: url, marks }],
            });
            Ok(false)
        }
        ast::Expr::Ref(r) => {
            if r.supplement().is_some() {
                return Err(unsupported(cx, expr, "a reference with its own text"));
            }
            out.push(Inline::Reference {
                label: r.target().to_string(),
            });
            Ok(false)
        }
        ast::Expr::Label(_) => Err(unsupported(cx, expr, "a label outside a heading")),
        ast::Expr::FuncCall(call) => inline_call(cx, call, out, marks, ctx),
        ast::Expr::ContentBlock(block) => {
            // A group: it holds if it is one plain paragraph. The
            // text keeps the inherited marks: the group's own runs
            // are unmarked by the check above. A leading group space
            // collapses onto the enclosing line when it holds text,
            // and is owed to it when the buffer is still empty.
            let (inner, leading) =
                convert_flow(cx, &block.body().exprs().collect::<Vec<_>>(), false)?;
            match inner.as_slice() {
                [Block::Paragraph { content, .. }] if content_styled_plain(content) => {
                    let out_was_empty = out.is_empty();
                    if leading && !out_was_empty {
                        push_space(out, &marks);
                    }
                    for i in content {
                        if let Inline::Text { text, .. } = i {
                            push_text(out, text, marks.clone());
                        }
                    }
                    Ok(leading && out_was_empty)
                }
                _ => Err(unsupported(cx, expr, "a block group")),
            }
        }
        ast::Expr::Parenthesized(p) => inline_into(cx, p.expr(), out, marks, ctx),
        ast::Expr::Int(n) => {
            push_text(out, &n.get().to_string(), marks);
            Ok(false)
        }
        ast::Expr::Float(n) => {
            push_text(out, &n.get().to_string(), marks);
            Ok(false)
        }
        ast::Expr::Str(s) => {
            push_text(out, &s.get(), marks);
            Ok(false)
        }
        ast::Expr::Bool(b) => {
            push_text(out, &b.get().to_string(), marks);
            Ok(false)
        }
        ast::Expr::None(_) => Ok(false),
        _ => Err(unsupported(
            cx,
            expr,
            &format!("{} inside {}", describe(expr), ctx),
        )),
    }
}

/// The single trailing content block a styling call takes.
fn styled_body<'a>(
    cx: &Cx,
    call: ast::FuncCall<'a>,
    args: &[ast::Arg<'a>],
    name: &str,
) -> Result<ast::Markup<'a>, Error> {
    match args {
        [ast::Arg::Pos(ast::Expr::ContentBlock(block))] => Ok(block.body()),
        _ => Err(unsupported(
            cx,
            ast::Expr::FuncCall(call),
            &format!("`#{name}`"),
        )),
    }
}

/// A function call in inline position. Returns whether leading
/// whitespace is still owed, as [`inline_into`] does.
fn inline_call(
    cx: &Cx,
    call: ast::FuncCall,
    out: &mut Vec<Inline>,
    marks: Marks,
    ctx: &str,
) -> Result<bool, Error> {
    let name = match call.callee() {
        ast::Expr::Ident(ident) => ident.as_str().to_string(),
        callee => return Err(unsupported(cx, callee, "a computed call")),
    };
    let args: Vec<ast::Arg> = call.args().items().collect();
    match name.as_str() {
        "pagebreak" if args.is_empty() => Err(unsupported(
            cx,
            ast::Expr::FuncCall(call),
            "a page break inside",
        )),
        "link" => match args.as_slice() {
            [
                ast::Arg::Pos(ast::Expr::Str(url)),
                ast::Arg::Pos(ast::Expr::ContentBlock(block)),
            ] => {
                let mut content = Vec::new();
                // Leading whitespace inside the link stays inside it:
                // a fresh line of its own.
                inline_markup(cx, block.body(), &mut content, marks, "a link")?;
                out.push(Inline::Link {
                    target: LinkTarget::Url(url.get().to_string()),
                    content,
                });
                Ok(false)
            }
            _ => Err(unsupported(cx, ast::Expr::FuncCall(call), "`#link`")),
        },
        "ref" => match args.as_slice() {
            [ast::Arg::Pos(ast::Expr::Label(label))] => {
                out.push(Inline::Reference {
                    label: label.get().to_string(),
                });
                Ok(false)
            }
            _ => Err(unsupported(cx, ast::Expr::FuncCall(call), "`#ref`")),
        },
        "sub" => {
            let mut inner = marks;
            inner.subscript = true;
            Ok(inline_markup(
                cx,
                styled_body(cx, call, &args, &name)?,
                out,
                inner,
                ctx,
            )?)
        }
        "super" => {
            let mut inner = marks;
            inner.superscript = true;
            Ok(inline_markup(
                cx,
                styled_body(cx, call, &args, &name)?,
                out,
                inner,
                ctx,
            )?)
        }
        "strike" => {
            let mut inner = marks;
            inner.strike = true;
            Ok(inline_markup(
                cx,
                styled_body(cx, call, &args, &name)?,
                out,
                inner,
                ctx,
            )?)
        }
        "underline" => {
            let mut inner = marks;
            inner.underline = true;
            Ok(inline_markup(
                cx,
                styled_body(cx, call, &args, &name)?,
                out,
                inner,
                ctx,
            )?)
        }
        _ => Err(unsupported(
            cx,
            ast::Expr::FuncCall(call),
            &format!("`#{name}`"),
        )),
    }
}

/// A `#pagebreak()` call with no arguments.
fn is_pagebreak(call: ast::FuncCall) -> bool {
    matches!(call.callee(), ast::Expr::Ident(ident) if ident.as_str() == "pagebreak")
        && call.args().items().next().is_none()
}

/// Inline text, merged with a matching tail.
fn push_text(out: &mut Vec<Inline>, text: &str, marks: Marks) {
    if text.is_empty() {
        return;
    }
    if let Some(Inline::Text {
        text: tail,
        marks: m,
    }) = out.last_mut()
        && *m == marks
    {
        tail.push_str(text);
        return;
    }
    out.push(Inline::Text {
        text: text.to_string(),
        marks,
    });
}

/// A collapsed space between words, in the current marks so a style
/// runs across it.
fn push_space(out: &mut Vec<Inline>, marks: &Marks) {
    let trailing = match out.last() {
        None => true,
        Some(Inline::Text { text, .. }) => text.ends_with(' ') || text.ends_with('\n'),
        Some(_) => false,
    };
    if !trailing {
        push_text(out, " ", marks.clone());
    }
}

/// Raw lines as code text.
fn raw_lines(raw: ast::Raw) -> String {
    raw.lines()
        .map(|t| t.get().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Plain paragraph content: only unmarked text, for groups.
fn content_styled_plain(content: &[Inline]) -> bool {
    content.iter().all(|i| {
        matches!(
            i,
            Inline::Text {
                marks: Marks {
                    bold: false,
                    italic: false,
                    underline: false,
                    strike: false,
                    superscript: false,
                    subscript: false,
                    small_caps: false,
                    code: false,
                    color: None,
                    highlight: None,
                    size: None,
                },
                ..
            }
        )
    })
}

/// A short name for an error message.
fn describe(expr: ast::Expr) -> String {
    match expr {
        ast::Expr::Heading(_) => "a heading".to_string(),
        ast::Expr::ListItem(_) => "a list".to_string(),
        ast::Expr::EnumItem(_) => "a list".to_string(),
        ast::Expr::TermItem(_) => "a term list".to_string(),
        ast::Expr::Equation(_) => "math".to_string(),
        ast::Expr::Math(_) => "math".to_string(),
        ast::Expr::Parbreak(_) => "a paragraph break".to_string(),
        ast::Expr::Raw(raw) if raw.block() => "a code block".to_string(),
        ast::Expr::FuncCall(call) => match call.callee() {
            ast::Expr::Ident(ident) => format!("`#{}`", ident.as_str()),
            _ => "a computed call".to_string(),
        },
        ast::Expr::Conditional(_) => "code".to_string(),
        ast::Expr::WhileLoop(_) => "code".to_string(),
        ast::Expr::ForLoop(_) => "code".to_string(),
        ast::Expr::LoopBreak(_) => "code".to_string(),
        ast::Expr::LoopContinue(_) => "code".to_string(),
        ast::Expr::Closure(_) => "code".to_string(),
        ast::Expr::LetBinding(_) => "code".to_string(),
        ast::Expr::DestructAssignment(_) => "code".to_string(),
        ast::Expr::Numeric(_) => "a measurement".to_string(),
        ast::Expr::FuncReturn(_) => "code".to_string(),
        ast::Expr::SetRule(_) => "a set rule".to_string(),
        ast::Expr::ShowRule(_) => "a show rule".to_string(),
        ast::Expr::Contextual(_) => "context".to_string(),
        ast::Expr::ModuleImport(_) => "an import".to_string(),
        ast::Expr::ModuleInclude(_) => "an include".to_string(),
        ast::Expr::CodeBlock(_) => "code".to_string(),
        ast::Expr::Array(_) => "an array".to_string(),
        ast::Expr::Dict(_) => "a dictionary".to_string(),
        ast::Expr::FieldAccess(_) => "a field".to_string(),
        ast::Expr::Parenthesized(_) => unreachable!("handled above"),
        ast::Expr::ContentBlock(_) => unreachable!("handled above"),
        _ => "this".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(source: &str) -> Vec<Block> {
        parse(source).unwrap().blocks
    }

    #[test]
    fn reads_headings_paragraphs_and_marks() {
        let got = blocks("= Report <rep>\n\nA *bold* and _italic_ `code` line.\n");
        assert_eq!(got.len(), 2);
        let Block::Heading {
            level,
            label,
            content,
        } = &got[0]
        else {
            panic!("a heading: {:?}", got[0])
        };
        assert_eq!(*level, 1);
        assert_eq!(label.as_deref(), Some("rep"));
        assert_eq!(content.len(), 1);
        let Block::Paragraph { content, .. } = &got[1] else {
            panic!("a paragraph: {:?}", got[1])
        };
        let marks: Vec<(String, bool, bool, bool)> = content
            .iter()
            .filter_map(|i| match i {
                Inline::Text { text, marks } => {
                    Some((text.clone(), marks.bold, marks.italic, marks.code))
                }
                _ => None,
            })
            .collect();
        assert!(
            marks.iter().any(|(t, b, _, _)| t.contains("bold") && *b),
            "{marks:?}"
        );
        assert!(
            marks.iter().any(|(t, _, i, _)| t.contains("italic") && *i),
            "{marks:?}"
        );
        assert!(
            marks.iter().any(|(t, _, _, c)| t.contains("code") && *c),
            "{marks:?}"
        );
    }

    #[test]
    fn reads_nested_lists_and_page_breaks() {
        let got = blocks("- one\n  - nested\n- two\n\n#pagebreak()\n\n+ first\n+ second\n");
        assert_eq!(got.len(), 3);
        let Block::List(first) = &got[0] else {
            panic!("a list: {got:?}")
        };
        assert!(!first.ordered);
        assert_eq!(first.items.len(), 2);
        assert!(first.items[0].list.is_some());
        assert!(matches!(got[1], Block::PageBreak));
        let Block::List(second) = &got[2] else {
            panic!("a list: {got:?}")
        };
        assert!(second.ordered);
    }

    #[test]
    fn rejects_what_has_no_word_form() {
        for (source, what) in [
            ("#grid(columns: 2, [a])\n", "`#grid`"),
            ("#figure(image(\"pic.png\"))\n", "a figure around `#image`"),
            ("$x + y$\n", "math"),
            ("#image(\"pic.png\")\n", "`#image`"),
            ("#let x = 1\n", "code"),
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
    fn rejects_deep_headings_and_stray_labels() {
        let err = parse("========== Too deep\n").unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }), "{err:?}");
        let err = parse("<alone>\n").unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }), "{err:?}");
    }

    #[test]
    fn rejects_renumbered_lists() {
        // The writer numbers from 1: an explicit start or jump has no
        // Word form.
        let err = parse("3. Third\n").unwrap_err();
        assert_eq!(
            err,
            Error::Unsupported {
                line: 1,
                what: "list number 3 out of order".to_string()
            }
        );
        let err = parse("1. first\n2. second\n4. fourth\n").unwrap_err();
        assert_eq!(
            err,
            Error::Unsupported {
                line: 3,
                what: "list number 4 out of order".to_string()
            }
        );
        // Continuing with `+` and matching numbers is fine.
        assert!(parse("1. first\n+ second\n3. third\n").is_ok());
    }

    #[test]
    fn rejects_text_after_a_nested_list() {
        // The model writes the item's text before its nested list, so
        // text after the list would move.
        let err = parse("- parent\n    - child\n  after\n").unwrap_err();
        assert_eq!(
            err,
            Error::Unsupported {
                line: 3,
                what: "text after a nested list".to_string()
            }
        );
    }

    fn has_bold(inlines: &[Inline]) -> bool {
        inlines.iter().any(|i| match i {
            Inline::Text { marks, .. } => marks.bold,
            Inline::Link { content, .. } => has_bold(content),
            _ => false,
        })
    }

    #[test]
    fn keeps_marks_through_links_and_groups() {
        for source in [
            "*#[hi]*\n",
            "*#link(\"https://x.example\")[hi]*\n",
            "*see https://x.example now*\n",
        ] {
            let got = blocks(source);
            assert_eq!(got.len(), 1, "{source:?}");
            let Block::Paragraph { content, .. } = &got[0] else {
                panic!("a paragraph: {got:?}");
            };
            assert!(has_bold(content), "{source:?}: {content:?}");
        }
    }

    #[test]
    fn rejects_references_with_their_own_text() {
        // The model always shows the label word and number: an
        // explicit supplement has no form, and may even hide math.
        for source in ["@intro[See]\n", "@intro[$x$]\n"] {
            let err = parse(source).unwrap_err();
            assert_eq!(
                err,
                Error::Unsupported {
                    line: 1,
                    what: "a reference with its own text".to_string()
                },
                "{source:?}"
            );
        }
        // A plain reference reads fine.
        let got = blocks("@intro\n");
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn preserves_space_at_group_boundaries() {
        // An inline group keeps its boundary spaces for the enclosing
        // line: only the enclosing paragraph trims its end. A nested
        // group propagates its leading space the same way.
        for source in [
            "#[hello ]world\n",
            "hello#[ world]\n",
            "hello#[#[ world]]\n",
        ] {
            let got = blocks(source);
            assert_eq!(got.len(), 1, "{source:?}");
            let Block::Paragraph { content, .. } = &got[0] else {
                panic!("a paragraph: {got:?}");
            };
            assert_eq!(
                content.as_slice(),
                &[Inline::Text {
                    text: "hello world".to_string(),
                    marks: Marks::default(),
                }],
                "{source:?}: {content:?}"
            );
        }
    }

    #[test]
    fn keeps_marks_across_the_space() {
        // The collapsed line break carries the marks: the whole phrase,
        // space included, stays underlined in one run.
        let got = blocks("#underline[two\nwords]\n");
        assert_eq!(got.len(), 1);
        let Block::Paragraph { content, .. } = &got[0] else {
            panic!("a paragraph: {got:?}");
        };
        assert_eq!(
            content.as_slice(),
            &[Inline::Text {
                text: "two words".to_string(),
                marks: Marks {
                    underline: true,
                    ..Marks::default()
                },
            }],
            "{content:?}"
        );
    }

    #[test]
    fn rejects_warnings_as_syntax_errors() {
        // `**` holds no text: Typst warns, and the tree drops it.
        let err = parse("ok\n**\n").unwrap_err();
        assert_eq!(
            err,
            Error::Syntax {
                line: 2,
                message: "no text within stars".to_string()
            }
        );
    }
}
