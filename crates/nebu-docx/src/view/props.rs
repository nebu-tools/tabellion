//! Reading properties of paragraphs, runs, content controls and drawings.

use crate::Error;
use crate::xml::{NodeId, Tree};

use super::walk::{R, W, W14};
use super::{ContentControlKind, DrawingKind, Format, Placement};

/// The value of `w:val` on a child of `parent`.
pub(crate) fn child_val(tree: &Tree, parent: NodeId, local: &str) -> Result<Option<String>, Error> {
    let Some(c) = tree.child(parent, Some(W), local)? else {
        return Ok(None);
    };
    Ok(tree.attribute(c, Some(W), "val")?.map(|v| v.into_owned()))
}

/// An on/off property: present means on unless `w:val` says false.
pub(crate) fn on_off(tree: &Tree, el: NodeId) -> Result<bool, Error> {
    Ok(!matches!(
        tree.attribute(el, Some(W), "val")?.as_deref(),
        Some("0" | "false" | "off" | "none")
    ))
}

fn flag(tree: &Tree, parent: NodeId, local: &str) -> Result<Option<bool>, Error> {
    match tree.child(parent, Some(W), local)? {
        Some(c) => Ok(Some(on_off(tree, c)?)),
        None => Ok(None),
    }
}

/// The paragraph's style id, from `w:pPr/w:pStyle`.
pub(crate) fn paragraph_style(tree: &Tree, p: NodeId) -> Result<Option<String>, Error> {
    match tree.child(p, Some(W), "pPr")? {
        Some(ppr) => child_val(tree, ppr, "pStyle"),
        None => Ok(None),
    }
}

/// The `w14:paraId` of a paragraph, read as Word reads it: eight
/// hexadecimal digits at most, in either case, below `0x80000000`. Word
/// replaces any other value when it saves, so such a value names nothing
/// that lasts.
pub(crate) fn para_id(tree: &Tree, p: NodeId) -> Result<Option<u32>, Error> {
    Ok(tree
        .attribute(p, Some(W14), "paraId")?
        .and_then(|v| parse_para_id(&v)))
}

/// A paragraph id from its hexadecimal digits.
pub(crate) fn parse_para_id(v: &str) -> Option<u32> {
    if v.is_empty() || v.len() > 8 || !v.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(v, 16).ok().filter(|&n| n < 0x8000_0000)
}

/// Direct paragraph properties that the views report.
#[derive(Debug, Default, Clone)]
pub(crate) struct ParagraphProps {
    pub style: Option<String>,
    /// `w:outlineLvl`, 0 to 8.
    pub outline: Option<u8>,
    /// `w:numPr`: numbering id and level.
    pub numbering: Option<(String, u8)>,
}

pub(crate) fn paragraph_props(tree: &Tree, p: NodeId) -> Result<ParagraphProps, Error> {
    let mut out = ParagraphProps::default();
    let Some(ppr) = tree.child(p, Some(W), "pPr")? else {
        return Ok(out);
    };
    out.style = child_val(tree, ppr, "pStyle")?;
    out.outline = child_val(tree, ppr, "outlineLvl")?.and_then(|v| v.parse().ok());
    out.numbering = numbering_ref(tree, ppr)?;
    Ok(out)
}

/// `w:numPr` in paragraph or style properties.
pub(crate) fn numbering_ref(tree: &Tree, ppr: NodeId) -> Result<Option<(String, u8)>, Error> {
    let Some(np) = tree.child(ppr, Some(W), "numPr")? else {
        return Ok(None);
    };
    let id = child_val(tree, np, "numId")?;
    let level = child_val(tree, np, "ilvl")?
        .and_then(|v| v.parse::<u8>().ok())
        .unwrap_or(0);
    Ok(id.map(|id| (id, level)))
}

/// The direct formatting of a run, from its `w:rPr`.
pub(crate) fn run_format(tree: &Tree, r: NodeId) -> Result<Format, Error> {
    let mut f = Format::default();
    let Some(rpr) = tree.child(r, Some(W), "rPr")? else {
        return Ok(f);
    };
    f.style = child_val(tree, rpr, "rStyle")?;
    f.bold = flag(tree, rpr, "b")?;
    f.italic = flag(tree, rpr, "i")?;
    f.strike = flag(tree, rpr, "strike")?;
    f.double_strike = flag(tree, rpr, "dstrike")?;
    f.small_caps = flag(tree, rpr, "smallCaps")?;
    f.all_caps = flag(tree, rpr, "caps")?;
    f.hidden = flag(tree, rpr, "vanish")?;
    f.underline = child_val(tree, rpr, "u")?;
    if let Some(v) = child_val(tree, rpr, "vertAlign")? {
        match v.as_str() {
            "superscript" => f.superscript = Some(true),
            "subscript" => f.subscript = Some(true),
            _ => {}
        }
    }
    f.size = child_val(tree, rpr, "sz")?
        .and_then(|v| v.parse::<f64>().ok())
        .map(|half| half / 2.0);
    f.color = child_val(tree, rpr, "color")?;
    f.highlight = child_val(tree, rpr, "highlight")?;
    if let Some(fonts) = tree.child(rpr, Some(W), "rFonts")? {
        f.font = tree
            .attribute(fonts, Some(W), "ascii")?
            .or(tree.attribute(fonts, Some(W), "hAnsi")?)
            .map(|v| v.into_owned());
    }
    Ok(f)
}

/// Author and date of a revision or a comment.
pub(crate) fn author_date(tree: &Tree, n: NodeId) -> Result<(String, Option<String>), Error> {
    let author = tree
        .attribute(n, Some(W), "author")?
        .map(|v| v.into_owned())
        .unwrap_or_default();
    let date = tree.attribute(n, Some(W), "date")?.map(|v| v.into_owned());
    Ok((author, date))
}

/// What a content control says about itself, from `w:sdtPr`.
#[derive(Debug, Clone)]
pub(crate) struct ControlProps {
    pub kind: ContentControlKind,
    pub tag: Option<String>,
    pub title: Option<String>,
    pub placeholder: bool,
}

pub(crate) fn control_props(tree: &Tree, sdt: NodeId) -> Result<ControlProps, Error> {
    let mut out = ControlProps {
        kind: ContentControlKind::RichText,
        tag: None,
        title: None,
        placeholder: false,
    };
    let Some(pr) = tree.child(sdt, Some(W), "sdtPr")? else {
        return Ok(out);
    };
    out.tag = child_val(tree, pr, "tag")?;
    out.title = child_val(tree, pr, "alias")?;
    out.placeholder = tree.child(pr, Some(W), "showingPlcHdr")?.is_some();
    for c in tree.elements(pr)? {
        let Some(name) = tree.name(c)? else { continue };
        let kind = match (name.namespace.unwrap_or(""), name.local) {
            (W, "text") => ContentControlKind::PlainText,
            (W, "richText") => ContentControlKind::RichText,
            (W, "picture") => ContentControlKind::Picture,
            (W, "date") => ContentControlKind::Date,
            (W, "comboBox") => ContentControlKind::ComboBox,
            (W, "dropDownList") => ContentControlKind::DropDownList,
            (W, "docPartObj") | (W, "docPartList") => ContentControlKind::BuildingBlock,
            (W, "group") => ContentControlKind::Group,
            (W, "citation") => ContentControlKind::Citation,
            (W, "bibliography") => ContentControlKind::Bibliography,
            (W, "equation") => ContentControlKind::Equation,
            (W14, "checkbox") => ContentControlKind::CheckBox,
            (ns, "repeatingSection") if ns.contains("/word/2012/") => {
                ContentControlKind::RepeatingSection
            }
            (ns, "repeatingSectionItem") if ns.contains("/word/2012/") => {
                ContentControlKind::RepeatingSectionItem
            }
            _ => continue,
        };
        out.kind = kind;
    }
    Ok(out)
}

const WP: &str = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const VML: &str = "urn:schemas-microsoft-com:vml";

/// What a drawing holds, read from `w:drawing`, `w:pict` or `w:object`.
#[derive(Debug, Clone)]
pub(crate) struct DrawingProps {
    pub kind: DrawingKind,
    pub placement: Placement,
    pub name: Option<String>,
    pub title: Option<String>,
    pub alt: Option<String>,
    /// Width and height in points.
    pub size: Option<(f64, f64)>,
    /// Relationship ids of pictures: embedded, then linked.
    pub embed: Option<String>,
    pub link: Option<String>,
    /// The text boxes inside, outermost only.
    pub text_boxes: Vec<NodeId>,
}

const EMU_PER_POINT: f64 = 12_700.0;

pub(crate) fn drawing_props(tree: &Tree, d: NodeId) -> Result<DrawingProps, Error> {
    let mut out = DrawingProps {
        kind: DrawingKind::Other,
        placement: Placement::Inline,
        name: None,
        title: None,
        alt: None,
        size: None,
        embed: None,
        link: None,
        text_boxes: vec![],
    };
    let legacy = !tree.is(d, Some(W), "drawing")?;
    // Depth first, in document order. The content of a text box is a story
    // of its own, and of mc:AlternateContent only the branch Word takes
    // counts, so neither is entered.
    let mut stack: Vec<NodeId> = tree.elements(d)?.collect();
    stack.reverse();
    while let Some(n) = stack.pop() {
        let Some(name) = tree.name(n)? else { continue };
        let ns = name.namespace.unwrap_or("");
        let enter = match (ns, name.local) {
            (W, "txbxContent") => false,
            (super::walk::MC, "AlternateContent") => {
                if let Some(b) = super::walk::choose(tree, n)? {
                    stack.push(b);
                }
                false
            }
            _ => true,
        };
        if enter {
            let at = stack.len();
            stack.extend(tree.elements(n)?);
            stack[at..].reverse();
        }
        match (ns, name.local) {
            (W, "txbxContent") => {
                out.text_boxes.push(n);
                if matches!(out.kind, DrawingKind::Other | DrawingKind::Shape) {
                    out.kind = DrawingKind::TextBox;
                }
            }
            (WP, "inline") => out.placement = Placement::Inline,
            (WP, "anchor") => out.placement = Placement::Floating,
            (WP, "extent") if out.size.is_none() => {
                let v = |a| -> Result<Option<f64>, Error> {
                    Ok(tree
                        .attribute(n, None, a)?
                        .and_then(|v| v.parse::<f64>().ok()))
                };
                if let (Some(cx), Some(cy)) = (v("cx")?, v("cy")?) {
                    out.size = Some((cx / EMU_PER_POINT, cy / EMU_PER_POINT));
                }
            }
            (WP, "docPr") => {
                out.name = attr(tree, n, "name")?;
                out.title = attr(tree, n, "title")?;
                out.alt = attr(tree, n, "descr")?;
            }
            (A, "graphicData") => {
                let uri = attr(tree, n, "uri")?.unwrap_or_default();
                let kind = if uri.ends_with("/picture") {
                    DrawingKind::Picture
                } else if uri.ends_with("/chart") || uri.contains("chartex") {
                    DrawingKind::Chart
                } else if uri.ends_with("/diagram") {
                    DrawingKind::Diagram
                } else if uri.ends_with("wordprocessingShape") {
                    DrawingKind::Shape
                } else if uri.ends_with("wordprocessingGroup") {
                    DrawingKind::Group
                } else if uri.ends_with("wordprocessingCanvas") {
                    DrawingKind::Canvas
                } else if uri.ends_with("wordprocessingInk") || uri.contains("/ink") {
                    DrawingKind::Ink
                } else {
                    DrawingKind::Other
                };
                if out.kind != DrawingKind::TextBox || kind != DrawingKind::Shape {
                    out.kind = kind;
                }
            }
            (A, "blip") if out.embed.is_none() && out.link.is_none() => {
                out.embed = tree.attribute(n, Some(R), "embed")?.map(|v| v.into_owned());
                out.link = tree.attribute(n, Some(R), "link")?.map(|v| v.into_owned());
            }
            (VML, "imagedata") if out.embed.is_none() => {
                out.embed = tree.attribute(n, Some(R), "id")?.map(|v| v.into_owned());
                if matches!(out.kind, DrawingKind::Other | DrawingKind::Shape) {
                    out.kind = DrawingKind::Picture;
                }
            }
            (VML, "shape" | "rect" | "roundrect" | "oval") if legacy && out.size.is_none() => {
                if let Some(style) = attr(tree, n, "style")? {
                    out.size = vml_size(&style);
                    if style.contains("position:absolute") {
                        out.placement = Placement::Floating;
                    }
                }
                if out.alt.is_none() {
                    out.alt = attr(tree, n, "alt")?;
                }
                if out.kind == DrawingKind::Other {
                    out.kind = DrawingKind::Shape;
                }
            }
            _ => {}
        }
    }
    if out.kind == DrawingKind::Other && tree.is(d, Some(W), "object")? {
        out.kind = DrawingKind::Object;
    }
    Ok(out)
}

fn attr(tree: &Tree, n: NodeId, local: &str) -> Result<Option<String>, Error> {
    Ok(tree
        .attribute(n, None, local)?
        .map(|v| v.into_owned())
        .filter(|v| !v.is_empty()))
}

/// Width and height in points from a VML style such as
/// `width:90pt;height:60pt`.
fn vml_size(style: &str) -> Option<(f64, f64)> {
    let mut w = None;
    let mut h = None;
    for decl in style.split(';') {
        let Some((k, v)) = decl.split_once(':') else {
            continue;
        };
        let v = length_points(v.trim());
        match k.trim() {
            "width" => w = v,
            "height" => h = v,
            _ => {}
        }
    }
    Some((w?, h?))
}

fn length_points(v: &str) -> Option<f64> {
    let units = [
        ("pt", 1.0),
        ("in", 72.0),
        ("cm", 72.0 / 2.54),
        ("mm", 72.0 / 25.4),
        ("px", 0.75),
        ("pc", 12.0),
    ];
    for (u, k) in units {
        if let Some(n) = v.strip_suffix(u) {
            return n.trim().parse::<f64>().ok().map(|n| n * k);
        }
    }
    // Plain numbers are EMU in some writers and points in others; Word
    // writes units.
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vml_sizes() {
        assert_eq!(vml_size("width:90pt;height:60pt"), Some((90.0, 60.0)));
        assert_eq!(
            vml_size("position:absolute;width:1in;height:0.5in"),
            Some((72.0, 36.0))
        );
        assert_eq!(vml_size("width:90pt"), None);
    }
}
