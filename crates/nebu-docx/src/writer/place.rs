//! Where a [`Place`] is: the part, the story, the element that holds the
//! new blocks, and what the place allows.

use crate::view::StoryKind;
use crate::xml::{NodeId, Tree};
use crate::{Document, Error, ns};

use super::Place;

const W: &str = ns::W;

/// Where new blocks go.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Anchor {
    Before(NodeId),
    After(NodeId),
    Replace(NodeId),
    /// As the last child of the container, which is empty.
    Append,
}

/// A place resolved against the document.
#[derive(Debug, Clone)]
pub(crate) struct Target {
    pub part: String,
    pub story: StoryKind,
    /// The element that holds the new blocks.
    pub container: NodeId,
    pub anchor: Anchor,
    pub in_cell: bool,
    pub in_box: bool,
    /// The top-level block of the story that holds the place.
    pub top: Option<NodeId>,
    /// The width of the cell that holds the place, in points, if it says.
    pub cell_width: Option<f64>,
}

impl Target {
    /// What a place does not allow, for the error that names it.
    pub fn name(&self) -> &'static str {
        if self.in_box {
            return "a text box";
        }
        match self.story {
            StoryKind::Main if self.in_cell => "a table cell",
            StoryKind::Main => "the body",
            StoryKind::Header => "a header",
            StoryKind::Footer => "a footer",
            StoryKind::Footnote => "a footnote",
            StoryKind::Endnote => "an endnote",
            StoryKind::Comment => "a comment",
            StoryKind::TextBox => "a text box",
        }
    }
}

/// The story parts of the document and their kinds.
pub(crate) fn stories(doc: &mut Document) -> Result<Vec<(String, StoryKind)>, Error> {
    let main = doc.main_part().to_string();
    let mut out = vec![(main.clone(), StoryKind::Main)];
    for r in doc.relationships(&main)? {
        let Some(p) = r.part else { continue };
        let kind = match r.kind.rsplit('/').next().unwrap_or_default() {
            "header" => StoryKind::Header,
            "footer" => StoryKind::Footer,
            "footnotes" => StoryKind::Footnote,
            "endnotes" => StoryKind::Endnote,
            "comments" => StoryKind::Comment,
            _ => continue,
        };
        if !out.iter().any(|(q, _)| q.eq_ignore_ascii_case(&p)) {
            out.push((p, kind));
        }
    }
    for (p, _) in &out {
        doc.xml(p)?;
    }
    Ok(out)
}

fn is_w(t: &Tree, n: NodeId, local: &str) -> Result<bool, Error> {
    t.is(n, Some(W), local)
}

/// Resolves a place.
pub(crate) fn resolve(
    doc: &mut Document,
    stories: &[(String, StoryKind)],
    place: Place,
) -> Result<Target, Error> {
    let main = doc.main_part().to_string();
    let node = match place {
        Place::End => {
            let t = doc.xml(&main)?;
            let Some(body) = t.child(t.root_element(), Some(W), "body")? else {
                return Err(Error::Invalid("the document has no body".into()));
            };
            let last = t.elements(body)?.last();
            let anchor = match last {
                Some(l) if is_w(t, l, "sectPr")? => Anchor::Before(l),
                Some(l) => Anchor::After(l),
                None => {
                    // An empty body: add after nothing by appending.
                    return Ok(Target {
                        part: main,
                        story: StoryKind::Main,
                        container: body,
                        anchor: Anchor::Append,
                        in_cell: false,
                        in_box: false,
                        top: None,
                        cell_width: None,
                    });
                }
            };
            let top = match anchor {
                Anchor::After(n) | Anchor::Before(n) | Anchor::Replace(n) => Some(n),
                Anchor::Append => None,
            };
            return Ok(Target {
                part: main,
                story: StoryKind::Main,
                container: body,
                anchor,
                in_cell: false,
                in_box: false,
                top,
                cell_width: None,
            });
        }
        Place::Before(n) | Place::After(n) | Place::Replace(n) => n,
    };
    let Some((part, story)) = stories
        .iter()
        .find(|(p, _)| doc.parsed(p).is_some_and(|t| t.kind(node).is_ok()))
        .cloned()
    else {
        return Err(Error::Invalid(
            "the place is a node of no story of this document; take it from this document's views"
                .into(),
        ));
    };
    let t = doc.xml(&part)?;
    let Some(name) = t.name(node)? else {
        return Err(Error::Invalid("the place is not an element".into()));
    };
    let local = name.local.to_string();
    if name.namespace != Some(W) {
        return Err(Error::Invalid(format!(
            "the place is a {local} element, not a block"
        )));
    }
    if story == StoryKind::Comment {
        return Err(Error::Invalid(
            "content cannot be inserted into a comment".into(),
        ));
    }
    let Some(parent) = t.parent(node)? else {
        return Err(Error::Invalid(
            "the place is not attached to its story".into(),
        ));
    };
    // The container must be one that holds blocks.
    let container_ok = is_w(t, parent, "body")?
        || is_w(t, parent, "tc")?
        || is_w(t, parent, "hdr")?
        || is_w(t, parent, "ftr")?
        || is_w(t, parent, "footnote")?
        || is_w(t, parent, "endnote")?
        || is_w(t, parent, "txbxContent")?
        || is_w(t, parent, "customXml")?
        || (is_w(t, parent, "sdtContent")?
            && t.parent(parent)?.is_some_and(|sdt| {
                t.parent(sdt)
                    .ok()
                    .flatten()
                    .is_some_and(|g| block_container(t, g).unwrap_or(false))
            }));
    if !container_ok {
        return Err(Error::Invalid(format!(
            "the place is a w:{local} inside a paragraph or row; name a block: a paragraph, a table or a content control around paragraphs"
        )));
    }
    match local.as_str() {
        "p" | "tbl" | "sdt" | "customXml" => {}
        "sectPr" => {
            if !matches!(place, Place::Before(_)) {
                return Err(Error::Invalid(
                    "the final section properties can only have content before them".into(),
                ));
            }
        }
        other => {
            if matches!(place, Place::Replace(_)) {
                return Err(Error::Invalid(format!(
                    "a w:{other} between blocks cannot be replaced"
                )));
            }
        }
    }
    if matches!(place, Place::Replace(_))
        && local == "p"
        && let Some(ppr) = t.child(node, Some(W), "pPr")?
        && t.child(ppr, Some(W), "sectPr")?.is_some()
    {
        return Err(Error::Invalid(
            "the paragraph ends a section; replacing it would remove the section break".into(),
        ));
    }
    let mut in_cell = false;
    let mut in_box = false;
    let mut cell_width = None;
    let mut top = None;
    let mut cur = Some(node);
    while let Some(c) = cur {
        let p = t.parent(c)?;
        if is_w(t, c, "tc")? {
            if !in_cell
                && let Some(pr) = t.child(c, Some(W), "tcPr")?
                && let Some(w) = t.child(pr, Some(W), "tcW")?
                && t.attribute(w, Some(W), "type")?.as_deref() == Some("dxa")
                && let Some(v) = t.attribute(w, Some(W), "w")?
                && let Ok(v) = v.parse::<f64>()
            {
                cell_width = Some(v / 20.0);
            }
            in_cell = true;
        }
        if is_w(t, c, "txbxContent")? {
            in_box = true;
        }
        if let Some(p) = p
            && (is_w(t, p, "body")? || is_w(t, p, "hdr")? || is_w(t, p, "ftr")?)
        {
            top = Some(c);
        }
        cur = p;
    }
    let anchor = match place {
        Place::Before(n) => Anchor::Before(n),
        Place::After(n) => Anchor::After(n),
        Place::Replace(n) => Anchor::Replace(n),
        Place::End => unreachable!("handled above"),
    };
    Ok(Target {
        part,
        story,
        container: parent,
        anchor,
        in_cell,
        in_box,
        top,
        cell_width,
    })
}

fn block_container(t: &Tree, n: NodeId) -> Result<bool, Error> {
    for l in [
        "body",
        "tc",
        "hdr",
        "ftr",
        "footnote",
        "endnote",
        "txbxContent",
        "sdtContent",
        "customXml",
    ] {
        if is_w(t, n, l)? {
            return Ok(true);
        }
    }
    Ok(false)
}
