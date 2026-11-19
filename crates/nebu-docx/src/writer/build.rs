//! Blocks and inlines as WordprocessingML.

use crate::content::{Align, Block, Image, Inline, LinkTarget, Marks, Math};
use crate::view::{StoryKind, StyleKind};
use crate::{Document, Error, Target};

use super::roles::Role;
use super::session::{Ctx, LabelKind, Session};
use super::xml::{attr, ppr, run_text, t, text, twips};

const HYPERLINK: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";

/// The `SEQ` identifier of a label word: Word's own captions use the label
/// with spaces as underscores.
pub(crate) fn seq_name(word: &str) -> String {
    word.trim().replace(' ', "_")
}

fn align_val(a: Align) -> &'static str {
    match a {
        Align::Left => "left",
        Align::Center => "center",
        Align::Right => "right",
        Align::Justify => "both",
    }
}

fn invalid(path: &str, what: impl std::fmt::Display) -> Error {
    Error::Invalid(format!("{path}: {what}"))
}

/// Run properties from marks and a character style, in schema order.
fn rpr(marks: &Marks, style: Option<&str>) -> String {
    let mut x = String::new();
    if let Some(s) = style {
        x.push_str(&format!("<w:rStyle w:val=\"{}\"/>", attr(s)));
    }
    if marks.bold {
        x.push_str("<w:b/><w:bCs/>");
    }
    if marks.italic {
        x.push_str("<w:i/><w:iCs/>");
    }
    if marks.small_caps {
        x.push_str("<w:smallCaps/>");
    }
    if marks.strike {
        x.push_str("<w:strike/>");
    }
    if let Some(c) = &marks.color {
        x.push_str(&format!("<w:color w:val=\"{}\"/>", c.hex()));
    }
    if let Some(s) = marks.size {
        let half = (s * 2.0).round() as i64;
        x.push_str(&format!(
            "<w:sz w:val=\"{half}\"/><w:szCs w:val=\"{half}\"/>"
        ));
    }
    if marks.underline {
        x.push_str("<w:u w:val=\"single\"/>");
    }
    if let Some(h) = &marks.highlight {
        x.push_str(&format!(
            "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>",
            h.hex()
        ));
    }
    if marks.superscript {
        x.push_str("<w:vertAlign w:val=\"superscript\"/>");
    } else if marks.subscript {
        x.push_str("<w:vertAlign w:val=\"subscript\"/>");
    }
    if x.is_empty() {
        String::new()
    } else {
        format!("<w:rPr>{x}</w:rPr>")
    }
}

/// A complex field: begin, code, separate, result, end.
pub(crate) fn field(code: &str, result: &str, result_rpr: &str) -> String {
    format!(
        "<w:r><w:fldChar w:fldCharType=\"begin\"/></w:r><w:r><w:instrText xml:space=\"preserve\"> {} </w:instrText></w:r><w:r><w:fldChar w:fldCharType=\"separate\"/></w:r><w:r>{result_rpr}{}</w:r><w:r><w:fldChar w:fldCharType=\"end\"/></w:r>",
        text(code.trim()),
        t(result)
    )
}

impl Session<'_> {
    pub(crate) fn para_id(&mut self, ctx: &Ctx) -> String {
        if ctx.para_ids {
            format!(" w14:paraId=\"{:08X}\"", self.ids.para())
        } else {
            String::new()
        }
    }

    pub(crate) fn style_ref(
        &mut self,
        doc: &mut Document,
        role: Role,
    ) -> Result<Option<String>, Error> {
        let roles = self.opts.roles.clone();
        self.book.role(doc, &roles, role)
    }

    fn p_style(
        &mut self,
        doc: &mut Document,
        role: Role,
    ) -> Result<Vec<(&'static str, String)>, Error> {
        Ok(match self.style_ref(doc, role)? {
            Some(id) => vec![("pStyle", format!("<w:pStyle w:val=\"{}\"/>", attr(&id)))],
            None => vec![],
        })
    }

    /// A paragraph with properties and content.
    pub(crate) fn paragraph_xml(
        &mut self,
        ctx: &Ctx,
        mut props: Vec<(&'static str, String)>,
        content: &str,
    ) -> String {
        let id = self.para_id(ctx);
        let lead = self.lead.take().unwrap_or_default();
        if let Some(track) = self.opts.track.clone() {
            let mark = self.ids.revision();
            let body = self.ids.revision();
            props.push((
                "rPr",
                format!(
                    "<w:rPr><w:ins w:id=\"{mark}\" w:author=\"{}\" w:date=\"{}\"/></w:rPr>",
                    attr(&track.author),
                    attr(&track.date)
                ),
            ));
            let inside = format!(
                "<w:ins w:id=\"{body}\" w:author=\"{}\" w:date=\"{}\">{lead}{content}</w:ins>",
                attr(&track.author),
                attr(&track.date)
            );
            return format!("<w:p{id}>{}{inside}</w:p>", ppr(props));
        }
        format!("<w:p{id}>{}{lead}{content}</w:p>", ppr(props))
    }

    pub(crate) fn block(
        &mut self,
        doc: &mut Document,
        b: &Block,
        ctx: &Ctx,
        path: &str,
    ) -> Result<String, Error> {
        crate::cost::tick(1);
        match b {
            Block::Paragraph {
                style,
                align,
                content,
            } => {
                let mut props = match style {
                    Some(name) => match self.book.find(StyleKind::Paragraph, name) {
                        Some(id) => {
                            vec![("pStyle", format!("<w:pStyle w:val=\"{}\"/>", attr(&id)))]
                        }
                        None => {
                            return Err(invalid(
                                &format!("{path}.style"),
                                format!("the document has no paragraph style {name:?}"),
                            ));
                        }
                    },
                    None => self.p_style(doc, ctx.text)?,
                };
                if let Some(a) = align.or(ctx.align) {
                    props.push(("jc", format!("<w:jc w:val=\"{}\"/>", align_val(a))));
                }
                let inner = self.inlines(doc, content, ctx, &format!("{path}.content"), None)?;
                Ok(self.paragraph_xml(ctx, props, &inner))
            }
            Block::Heading {
                level,
                content,
                label,
            } => {
                let mut props = self.p_style(doc, Role::Heading(*level))?;
                if let Some(a) = ctx.align {
                    props.push(("jc", format!("<w:jc w:val=\"{}\"/>", align_val(a))));
                }
                let mut inner =
                    self.inlines(doc, content, ctx, &format!("{path}.content"), None)?;
                if let Some(l) = label {
                    self.main_only(ctx, &format!("{path}.label"), "a labelled heading")?;
                    let bm = self.bookmark_around(l, &inner);
                    inner = bm;
                }
                Ok(self.paragraph_xml(ctx, props, &inner))
            }
            Block::List(l) => self.list(doc, l, ctx, path),
            Block::Table(t) => self.table(doc, t, ctx, path),
            Block::Figure {
                image,
                caption,
                label,
            } => {
                self.main_only(ctx, &format!("{path}.caption"), "a caption")?;
                let pic = self.picture(doc, image, ctx, &format!("{path}.image"))?;
                let mut props = self.p_style(doc, Role::Figure)?;
                props.push(("keepNext", "<w:keepNext/>".into()));
                if let Some(a) = ctx.align {
                    props.push(("jc", format!("<w:jc w:val=\"{}\"/>", align_val(a))));
                }
                let mut x = self.paragraph_xml(ctx, props, &pic);
                let word = self.opts.captions.figure.clone();
                x.push_str(&self.caption(
                    doc,
                    ctx,
                    &word,
                    caption,
                    label.as_deref(),
                    false,
                    path,
                )?);
                Ok(x)
            }
            Block::Image { image } => {
                let pic = self.picture(doc, image, ctx, &format!("{path}.image"))?;
                let mut props = self.p_style(doc, Role::Figure)?;
                if let Some(a) = ctx.align {
                    props.push(("jc", format!("<w:jc w:val=\"{}\"/>", align_val(a))));
                }
                Ok(self.paragraph_xml(ctx, props, &pic))
            }
            Block::Equation { math, label } => {
                self.equation(doc, math, label.as_deref(), ctx, path)
            }
            Block::PageBreak => {
                if ctx.story != StoryKind::Main || ctx.in_cell || ctx.in_box {
                    return Err(invalid(
                        path,
                        format!("a page break cannot stand in {}", ctx.place),
                    ));
                }
                Ok(self.paragraph_xml(ctx, vec![], "<w:r><w:br w:type=\"page\"/></w:r>"))
            }
            Block::Raw { xml } => {
                if self.opts.track.is_some() {
                    return Err(invalid(
                        path,
                        "raw XML cannot be written as a tracked insertion",
                    ));
                }
                Ok(xml.clone())
            }
        }
    }

    /// Captions and labels number in the body only.
    pub(crate) fn main_only(&self, ctx: &Ctx, path: &str, what: &str) -> Result<(), Error> {
        if ctx.story != StoryKind::Main || ctx.in_box {
            return Err(invalid(
                path,
                format!(
                    "{what} is numbered in the body; it cannot stand in {}",
                    ctx.place
                ),
            ));
        }
        Ok(())
    }

    /// A hidden bookmark `_Ref_<label>` around some content.
    pub(crate) fn bookmark_around(&mut self, label: &str, inner: &str) -> String {
        let id = self.ids.bookmark();
        let name = format!("_Ref_{label}");
        format!(
            "<w:bookmarkStart w:id=\"{id}\" w:name=\"{}\"/>{inner}<w:bookmarkEnd w:id=\"{id}\"/>",
            attr(&name)
        )
    }

    /// A caption as Word writes one: the label word, a `SEQ` field, the
    /// separator and the text, the bookmark of the label around word and
    /// number.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn caption(
        &mut self,
        doc: &mut Document,
        ctx: &Ctx,
        word: &str,
        caption: &[Inline],
        label: Option<&str>,
        above: bool,
        path: &str,
    ) -> Result<String, Error> {
        let mut props = self.p_style(doc, Role::Caption)?;
        if above {
            props.push(("keepNext", "<w:keepNext/>".into()));
        }
        let seq = seq_name(word);
        let number = format!(
            "<w:r><w:t xml:space=\"preserve\">{} </w:t></w:r><w:fldSimple w:instr=\" SEQ {} \\* ARABIC \"><w:r><w:rPr><w:noProof/></w:rPr><w:t>1</w:t></w:r></w:fldSimple>",
            text(word),
            attr(&seq)
        );
        let mut inner = match label {
            Some(l) => self.bookmark_around(l, &number),
            None => number,
        };
        let sep = self.opts.captions.separator.clone();
        if !sep.is_empty() {
            inner.push_str(&format!("<w:r>{}</w:r>", t(&sep)));
        }
        inner.push_str(&self.inlines(doc, caption, ctx, &format!("{path}.caption"), None)?);
        Ok(self.paragraph_xml(ctx, props, &inner))
    }

    fn equation(
        &mut self,
        doc: &mut Document,
        math: &Math,
        label: Option<&str>,
        ctx: &Ctx,
        path: &str,
    ) -> Result<String, Error> {
        let m = math.omml.trim();
        let Some(l) = label else {
            let props = self.p_style(doc, ctx.text)?;
            return Ok(self.paragraph_xml(ctx, props, &format!("<m:oMathPara>{m}</m:oMathPara>")));
        };
        self.main_only(ctx, &format!("{path}.label"), "a numbered equation")?;
        let mut props = self.p_style(doc, ctx.text)?;
        props.push((
            "tabs",
            format!(
                "<w:tabs><w:tab w:val=\"center\" w:pos=\"{}\"/><w:tab w:val=\"right\" w:pos=\"{}\"/></w:tabs>",
                twips(ctx.width / 2.0),
                twips(ctx.width)
            ),
        ));
        let seq = seq_name(&self.opts.captions.equation);
        let number = format!(
            "<w:r><w:t>(</w:t></w:r><w:fldSimple w:instr=\" SEQ {} \\* ARABIC \"><w:r><w:rPr><w:noProof/></w:rPr><w:t>1</w:t></w:r></w:fldSimple><w:r><w:t>)</w:t></w:r>",
            attr(&seq)
        );
        let numbered = self.bookmark_around(l, &number);
        Ok(self.paragraph_xml(
            ctx,
            props,
            &format!("<w:r><w:tab/></w:r>{m}<w:r><w:tab/></w:r>{numbered}"),
        ))
    }

    /// Inlines as runs. `style` is the character style of every run, for
    /// the text of a link.
    pub(crate) fn inlines(
        &mut self,
        doc: &mut Document,
        list: &[Inline],
        ctx: &Ctx,
        path: &str,
        style: Option<&str>,
    ) -> Result<String, Error> {
        let mut out = String::new();
        for (i, x) in list.iter().enumerate() {
            crate::cost::tick(1);
            let p = format!("{path}[{i}]");
            match x {
                Inline::Text { text: s, marks } => {
                    if s.is_empty() {
                        continue;
                    }
                    let code = if marks.code && style.is_none() {
                        self.style_ref(doc, Role::Code)?
                    } else {
                        None
                    };
                    let st = style.map(str::to_string).or(code);
                    out.push_str(&format!(
                        "<w:r>{}{}</w:r>",
                        rpr(marks, st.as_deref()),
                        run_text(s)
                    ));
                }
                Inline::Footnote { content } => {
                    if ctx.story != StoryKind::Main || ctx.in_box {
                        return Err(invalid(
                            &p,
                            format!("a footnote cannot stand in {}", ctx.place),
                        ));
                    }
                    out.push_str(&self.footnote(doc, content, ctx, &format!("{p}.content"))?);
                }
                Inline::Reference { label } => {
                    self.main_only(ctx, &p, "a cross-reference")?;
                    let Some(l) = self.labels.get(&label.to_lowercase()).cloned() else {
                        let mut known: Vec<&str> =
                            self.labels.values().map(|l| l.name.as_str()).collect();
                        known.sort_unstable();
                        return Err(invalid(
                            &format!("{p}.label"),
                            format!(
                                "no label {label:?}; the labels are {}",
                                if known.is_empty() {
                                    "none".to_string()
                                } else {
                                    known.join(", ")
                                }
                            ),
                        ));
                    };
                    if l.kind == LabelKind::Equation {
                        let word = self.opts.captions.equation.clone();
                        out.push_str(&format!("<w:r>{}</w:r>", t(&format!("{word} "))));
                    }
                    out.push_str(&field(&format!("REF {} \\h", l.bookmark), &l.name, ""));
                }
                Inline::Link { target, content } => {
                    let link_style = self.style_ref(doc, Role::Hyperlink)?;
                    let inner = self.inlines(
                        doc,
                        content,
                        ctx,
                        &format!("{p}.content"),
                        link_style.as_deref(),
                    )?;
                    let open = match target {
                        LinkTarget::Url(u) => {
                            let id = self.rel(doc, &ctx.part, HYPERLINK, Target::External(u))?;
                            format!("<w:hyperlink r:id=\"{}\" w:history=\"1\">", attr(&id))
                        }
                        LinkTarget::Bookmark(b) => {
                            if !self.bookmarks.contains(&b.to_lowercase()) {
                                return Err(invalid(
                                    &format!("{p}.target.bookmark"),
                                    format!("the document has no bookmark {b:?}"),
                                ));
                            }
                            format!("<w:hyperlink w:anchor=\"{}\" w:history=\"1\">", attr(b))
                        }
                        LinkTarget::Label(l) => {
                            let Some(found) = self.labels.get(&l.to_lowercase()) else {
                                return Err(invalid(
                                    &format!("{p}.target.label"),
                                    format!("no label {l:?}"),
                                ));
                            };
                            format!(
                                "<w:hyperlink w:anchor=\"{}\" w:history=\"1\">",
                                attr(&found.bookmark)
                            )
                        }
                    };
                    let inner = if inner.is_empty() {
                        let shown = match target {
                            LinkTarget::Url(u) | LinkTarget::Bookmark(u) | LinkTarget::Label(u) => {
                                u.clone()
                            }
                        };
                        format!(
                            "<w:r>{}{}</w:r>",
                            rpr(&Marks::default(), link_style.as_deref()),
                            run_text(&shown)
                        )
                    } else {
                        inner
                    };
                    out.push_str(&format!("{open}{inner}</w:hyperlink>"));
                }
                Inline::Math { math } => out.push_str(math.omml.trim()),
                Inline::LineBreak => out.push_str("<w:r><w:br/></w:r>"),
                Inline::Field { code, result } => {
                    let r = style
                        .map(|s| rpr(&Marks::default(), Some(s)))
                        .unwrap_or_default();
                    out.push_str(&field(code, result, &r));
                }
                Inline::Image { image } => {
                    out.push_str(&self.picture(doc, image, ctx, &format!("{p}.image"))?)
                }
                Inline::Raw { xml } => out.push_str(xml),
            }
        }
        Ok(out)
    }

    /// A relationship from a part: one the part has to the same target,
    /// or a new one. The part's relationships are read once per session.
    pub(crate) fn rel(
        &mut self,
        doc: &mut Document,
        source: &str,
        kind: &str,
        target: Target<'_>,
    ) -> Result<String, Error> {
        let src = source.to_lowercase();
        let key = |kind: &str, external: bool, t: &str| {
            format!(
                "{kind}|{}",
                if external {
                    t.to_string()
                } else {
                    t.to_lowercase()
                }
            )
        };
        if !self.rels.contains_key(&src) {
            let list = doc.relationships(source)?;
            let next = list
                .iter()
                .filter_map(|r| r.id.strip_prefix("rId")?.parse::<u64>().ok())
                .max()
                .unwrap_or(0)
                + 1;
            let mut known = std::collections::HashMap::new();
            for r in list {
                let t = match (&r.part, r.external) {
                    (_, true) => key(&r.kind, true, &r.target),
                    (Some(p), false) => key(&r.kind, false, p),
                    (None, false) => continue,
                };
                known.entry(t).or_insert(r.id);
            }
            self.rels.insert(src.clone(), (next, known));
        }
        let k = match target {
            Target::Part(p) => key(kind, false, doc.part_name(p).unwrap_or(p)),
            Target::External(u) => key(kind, true, u),
        };
        let Some((next, known)) = self.rels.get_mut(&src) else {
            return Err(Error::Invalid("no relationships read".into()));
        };
        if let Some(id) = known.get(&k) {
            return Ok(id.clone());
        }
        let id = format!("rId{next}");
        *next += 1;
        known.insert(k, id.clone());
        doc.add_relationship_as(source, &id, kind, target)?;
        Ok(id)
    }

    /// A picture as a run.
    pub(crate) fn picture(
        &mut self,
        doc: &mut Document,
        image: &Image,
        ctx: &Ctx,
        path: &str,
    ) -> Result<String, Error> {
        super::media::picture(self, doc, image, ctx, path)
    }
}
