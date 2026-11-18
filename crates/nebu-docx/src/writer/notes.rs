//! Footnotes: the part, created with its separators when the document has
//! none, and each note with its reference.

use crate::content::Block;
use crate::view::StoryKind;
use crate::xml::Tree;
use crate::{Document, Error, Target, ns};

use super::roles::Role;
use super::session::{Ctx, Session};
use super::xml::{SETTINGS_ORDER, attr};

const W: &str = ns::W;
const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
const FOOTNOTES_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes";
const FOOTNOTES_CONTENT: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml";

/// Inserts a child into an element where the schema's order puts it.
pub(crate) fn insert_ordered(
    t: &mut Tree,
    parent: crate::xml::NodeId,
    node: crate::xml::NodeId,
    local: &str,
    order: &[&str],
) -> Result<(), Error> {
    let rank = |l: &str| order.iter().position(|o| *o == l).unwrap_or(usize::MAX);
    let me = rank(local);
    let mut before = None;
    for c in t.elements(parent)? {
        let Some(name) = t.name(c)? else { continue };
        if name.namespace == Some(W) && rank(name.local) > me {
            before = Some(c);
            break;
        }
    }
    match before {
        Some(b) => t.insert_before(b, node),
        None => t.append(parent, node),
    }
}

impl Session<'_> {
    /// The footnotes part, created on first use.
    fn notes_part(&mut self, doc: &mut Document) -> Result<String, Error> {
        if let Some(p) = self.notes_part.clone() {
            if self.next_note == 0 {
                let t = doc.xml(&p)?;
                self.next_note = first_free(t)?;
                let w14 = super::session::ignorable_w14(t)?;
                self.para_parts.insert(p.clone(), w14);
            }
            return Ok(p);
        }
        let main = self.main.clone();
        let with_w14 = self.para_parts.get(&main).copied().unwrap_or(false);
        let decl = if with_w14 {
            format!(" xmlns:mc=\"{MC}\" xmlns:w14=\"{W14}\" mc:Ignorable=\"w14\"")
        } else {
            String::new()
        };
        let sep = |kind: &str, id: i32, el: &str| {
            format!(
                "<w:footnote w:type=\"{kind}\" w:id=\"{id}\"><w:p><w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr><w:r><w:{el}/></w:r></w:p></w:footnote>"
            )
        };
        let xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n<w:footnotes xmlns:w=\"{W}\"{decl}>{}{}</w:footnotes>",
            sep("separator", -1, "separator"),
            sep("continuationSeparator", 0, "continuationSeparator")
        );
        let name = super::ids::free_part(doc, "word/footnotes", "xml");
        doc.add_part(&name, FOOTNOTES_CONTENT, xml.into_bytes())?;
        self.rel(doc, &main, FOOTNOTES_TYPE, Target::Part(&name))?;
        doc.xml(&name)?;
        self.para_parts.insert(name.clone(), with_w14);
        // Word lists the separators in the settings.
        let settings = doc
            .relationships(&main)?
            .into_iter()
            .find(|r| r.kind.ends_with("/settings"))
            .and_then(|r| r.part);
        if let Some(s) = settings {
            let t = doc.xml(&s)?;
            let root = t.root_element();
            if t.child(root, Some(W), "footnotePr")?.is_none() {
                let n = t.parse_fragment(
                    "<w:footnotePr><w:footnote w:id=\"-1\"/><w:footnote w:id=\"0\"/></w:footnotePr>",
                    &[("w", W)],
                )?;
                insert_ordered(t, root, n[0], "footnotePr", SETTINGS_ORDER)?;
            }
        }
        self.notes_part = Some(name.clone());
        self.next_note = 1;
        Ok(name)
    }

    /// Writes a footnote and returns the run that calls it.
    pub(crate) fn footnote(
        &mut self,
        doc: &mut Document,
        content: &[Block],
        ctx: &Ctx,
        path: &str,
    ) -> Result<String, Error> {
        let part = self.notes_part(doc)?;
        let id = self.next_note;
        self.next_note += 1;
        let para_ids = self.para_parts.get(&part).copied().unwrap_or(false);
        let note_ctx = Ctx {
            part: part.clone(),
            story: StoryKind::Footnote,
            in_cell: false,
            in_box: false,
            in_note: true,
            width: ctx.width,
            para_ids,
            text: Role::FootnoteText,
            align: None,
            place: "a footnote",
        };
        let reference = self.style_ref(doc, Role::FootnoteReference)?;
        let rstyle = reference
            .as_deref()
            .map(|s| format!("<w:rPr><w:rStyle w:val=\"{}\"/></w:rPr>", attr(s)))
            .unwrap_or_else(|| "<w:rPr><w:vertAlign w:val=\"superscript\"/></w:rPr>".into());
        // The note's first paragraph starts with its mark.
        let mut blocks: Vec<Block> = content.to_vec();
        if !matches!(blocks.first(), Some(Block::Paragraph { .. })) {
            blocks.insert(
                0,
                Block::Paragraph {
                    style: None,
                    align: None,
                    content: vec![],
                },
            );
        }
        self.lead = Some(format!(
            "<w:r>{rstyle}<w:footnoteRef/></w:r><w:r><w:t xml:space=\"preserve\"> </w:t></w:r>"
        ));
        let mut body = String::new();
        let offset = usize::from(!matches!(content.first(), Some(Block::Paragraph { .. })));
        for (i, b) in blocks.iter().enumerate() {
            let p = if i < offset {
                path.to_string()
            } else {
                format!("{path}[{}]", i - offset)
            };
            body.push_str(&self.block(doc, b, &note_ctx, &p)?);
        }
        self.lead = None;
        let t = doc.xml(&part)?;
        let root = t.root_element();
        let nodes = t.parse_fragment(
            &format!("<w:footnote w:id=\"{id}\">{body}</w:footnote>"),
            crate::content::RAW_PREFIXES,
        )?;
        for n in nodes {
            t.append(root, n)?;
        }
        Ok(format!(
            "<w:r>{rstyle}<w:footnoteReference w:id=\"{id}\"/></w:r>"
        ))
    }
}

/// The first footnote id above those the part has.
fn first_free(t: &Tree) -> Result<i64, Error> {
    let mut max = 0i64;
    for n in t.elements(t.root_element())? {
        if t.is(n, Some(W), "footnote")?
            && let Some(v) = t.attribute(n, Some(W), "id")?
            && let Ok(v) = v.parse::<i64>()
        {
            max = max.max(v);
        }
    }
    Ok(max + 1)
}
