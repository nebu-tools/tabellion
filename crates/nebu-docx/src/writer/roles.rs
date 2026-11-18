//! Which style plays which part, and the styles the writer adds.
//!
//! [`Roles`] names a style for each [`Role`]. A role the caller does not
//! name goes to Word's built-in style for it, found by the name Word
//! stores (`heading 1`, `caption`). A role whose style the document lacks
//! falls back in a fixed order, given by [`Role::fallback`]: the body to
//! the default paragraph style, a list to `List Paragraph`, table text to
//! the default paragraph style, a figure to the body. A role with no
//! fallback, and a fallback the document lacks, gets Word's own definition
//! of the built-in style, as Word writes it (`builtin.xml`, made by
//! `tools/word/builtin-styles.sh`). [`Inserted::roles`] says which of
//! these each role got.
//!
//! [`Inserted::roles`]: super::Inserted::roles

use std::collections::{BTreeMap, HashMap};

use crate::content::{Border, LineStyle, RowLook, TableStyleDef};
use crate::view::{StyleKind, Styles};
use crate::{Document, Error, ns};

use super::TableStyle;
use super::xml::{attr, eighths, twips};

const W: &str = ns::W;
const STYLES_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";
const STYLES_CONTENT: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml";

/// Word's definitions of the built-in styles the writer may add.
const BUILTIN: &str = include_str!("builtin.xml");

/// A part that content plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Role {
    /// Paragraphs of body text.
    Body,
    /// Headings of a level, 1 to 9.
    Heading(u8),
    /// Captions of tables, figures and equations.
    Caption,
    /// Items of bulleted lists.
    ListBullet,
    /// Items of numbered lists.
    ListNumber,
    /// The text of footnotes.
    FootnoteText,
    /// The mark that calls a footnote.
    FootnoteReference,
    /// The paragraph that holds a figure's picture.
    Figure,
    /// Paragraphs in table cells.
    TableText,
    /// Tables that name no style.
    Table,
    /// The text of hyperlinks.
    Hyperlink,
    /// Code in the line of text.
    Code,
}

impl Role {
    /// The name Word stores for its built-in style for this role, if it has
    /// one.
    pub fn built_in(self) -> Option<String> {
        Some(
            match self {
                Role::Body => "Body Text",
                Role::Heading(n) => return Some(format!("heading {n}")),
                Role::Caption => "caption",
                Role::ListBullet => "List Bullet",
                Role::ListNumber => "List Number",
                Role::FootnoteText => "footnote text",
                Role::FootnoteReference => "footnote reference",
                Role::Figure => return None,
                Role::TableText => return None,
                Role::Table => "Table Grid",
                Role::Hyperlink => "Hyperlink",
                Role::Code => "HTML Code",
            }
            .to_string(),
        )
    }

    /// Where the role goes when the document lacks its style: another
    /// role, the default paragraph style, or Word's definition added to
    /// the document.
    pub fn fallback(self) -> Fallback {
        match self {
            Role::Body | Role::TableText => Fallback::DefaultParagraph,
            Role::Figure => Fallback::Role(Role::Body),
            Role::ListBullet | Role::ListNumber => Fallback::Add("List Paragraph"),
            _ => Fallback::AddOwn,
        }
    }

    fn kind(self) -> StyleKind {
        match self {
            Role::Table => StyleKind::Table,
            Role::FootnoteReference | Role::Hyperlink | Role::Code => StyleKind::Character,
            _ => StyleKind::Paragraph,
        }
    }

    fn describe(self) -> String {
        match self {
            Role::Body => "body text".into(),
            Role::Heading(n) => format!("headings of level {n}"),
            Role::Caption => "captions".into(),
            Role::ListBullet => "bulleted lists".into(),
            Role::ListNumber => "numbered lists".into(),
            Role::FootnoteText => "footnote text".into(),
            Role::FootnoteReference => "footnote references".into(),
            Role::Figure => "figures".into(),
            Role::TableText => "table text".into(),
            Role::Table => "tables".into(),
            Role::Hyperlink => "hyperlinks".into(),
            Role::Code => "code".into(),
        }
    }
}

/// What a role falls back to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fallback {
    /// The document's default paragraph style.
    DefaultParagraph,
    /// The style of another role.
    Role(Role),
    /// This built-in style, found or added.
    Add(&'static str),
    /// The role's own built-in style, added.
    AddOwn,
}

/// Where a role's style came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoleSource {
    /// The style the caller named.
    Chosen(String),
    /// Word's built-in style for the role, which the document has.
    BuiltIn(String),
    /// The document lacks `wanted`; `used` stands in for it.
    Fallback {
        /// The style the role would have had.
        wanted: String,
        /// The style it has.
        used: String,
    },
    /// Word's definition of the style, added to the document.
    Added(String),
    /// No style: the document has no default paragraph style either.
    Unstyled,
}

/// The styles that play the roles. The default detects them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Roles {
    chosen: BTreeMap<Role, String>,
}

impl Roles {
    /// The roles Word's built-in styles play in a document with these
    /// styles: each role whose built-in style the document has, by its
    /// stored name. Roles it lacks are left to their fallbacks.
    pub fn detect(styles: &Styles) -> Roles {
        let mut chosen = BTreeMap::new();
        let mut roles = vec![
            Role::Body,
            Role::Caption,
            Role::ListBullet,
            Role::ListNumber,
            Role::FootnoteText,
            Role::FootnoteReference,
            Role::Table,
            Role::Hyperlink,
            Role::Code,
        ];
        roles.extend((1..=9).map(Role::Heading));
        for r in roles {
            if let Some(name) = r.built_in()
                && let Some(s) = styles.find_of(r.kind(), &name)
            {
                chosen.insert(r, s.name.clone());
            }
        }
        Roles { chosen }
    }

    /// Names the style for a role, by id, stored name or the name Word
    /// shows.
    pub fn set(&mut self, role: Role, style: &str) -> &mut Roles {
        self.chosen.insert(role, style.to_string());
        self
    }

    /// The style named for a role.
    pub fn get(&self, role: Role) -> Option<&str> {
        self.chosen.get(&role).map(String::as_str)
    }
}

/// The styles of the document as the writer adds to them.
pub(crate) struct StyleBook {
    styles: Styles,
    /// Styles added in this session: lower-case name and id to id, kind.
    added: HashMap<String, (String, StyleKind)>,
    ids: std::collections::HashSet<String>,
    part: Option<String>,
    pub added_names: Vec<String>,
    resolved: HashMap<Role, (Option<String>, RoleSource)>,
}

/// The `w:style` elements of Word's definitions, by stored name in lower
/// case.
fn builtin(name: &str) -> Option<&'static str> {
    let key = format!("<w:name w:val=\"{name}\"/>");
    let at = BUILTIN.find(&key)?;
    let start = BUILTIN[..at].rfind("<w:style ")?;
    let end = at + BUILTIN[at..].find("</w:style>")? + "</w:style>".len();
    Some(&BUILTIN[start..end])
}

impl StyleBook {
    pub fn new(styles: Styles, part: Option<String>) -> StyleBook {
        let ids = styles.iter().map(|s| s.id.to_lowercase()).collect();
        StyleBook {
            styles,
            added: HashMap::new(),
            ids,
            part,
            added_names: vec![],
            resolved: HashMap::new(),
        }
    }

    pub fn styles(&self) -> &Styles {
        &self.styles
    }

    /// The id of a style of a kind, by id or name, among the document's
    /// styles and those added.
    pub fn find(&self, kind: StyleKind, name: &str) -> Option<String> {
        if let Some(s) = self.styles.find_of(kind, name) {
            return Some(s.id.clone());
        }
        self.added
            .get(&name.to_lowercase())
            .filter(|(_, k)| *k == kind)
            .map(|(id, _)| id.clone())
    }

    /// The default paragraph style's id.
    fn default_paragraph(&self) -> Option<(String, String)> {
        self.styles
            .default_of(StyleKind::Paragraph)
            .map(|s| (s.id.clone(), s.name.clone()))
    }

    /// A free style id from a name.
    fn free_id(&self, name: &str) -> String {
        let base: String = name.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        let base = if base.is_empty() || base.starts_with(|c: char| c.is_ascii_digit()) {
            format!("Style{base}")
        } else {
            base
        };
        let mut id = base.clone();
        let mut k = 2;
        while self.ids.contains(&id.to_lowercase()) {
            id = format!("{base}{k}");
            k += 1;
        }
        id
    }

    /// Appends a `w:style` to the styles part, creating the part if the
    /// document has none.
    fn append(&mut self, doc: &mut Document, xml: &str) -> Result<(), Error> {
        let part = match &self.part {
            Some(p) => p.clone(),
            None => {
                let main = doc.main_part().to_string();
                let name = super::ids::free_part(doc, "word/styles", "xml");
                let body = format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n<w:styles xmlns:w=\"{W}\"></w:styles>"
                );
                doc.add_part(&name, STYLES_CONTENT, body.into_bytes())?;
                doc.add_relationship(&main, STYLES_TYPE, crate::Target::Part(&name))?;
                self.part = Some(name.clone());
                name
            }
        };
        let t = doc.xml(&part)?;
        let root = t.root_element();
        let nodes = t.parse_fragment(xml, &[("w", W)])?;
        for n in nodes {
            t.append(root, n)?;
        }
        Ok(())
    }

    /// Adds Word's definition of a built-in style and returns its id.
    fn add_builtin(&mut self, doc: &mut Document, name: &str) -> Result<String, Error> {
        let Some(def) = builtin(name) else {
            return Err(Error::Invalid(format!(
                "the document has no style {name:?}, and tabellion has no definition of it to add"
            )));
        };
        // Table Grid is based on Normal Table; add that first if missing.
        if name == "Table Grid" && self.find(StyleKind::Table, "Normal Table").is_none() {
            self.add_builtin(doc, "Normal Table")?;
        }
        let want_id = def
            .split("w:styleId=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .unwrap_or("Style")
            .to_string();
        let id = if self.ids.contains(&want_id.to_lowercase()) {
            self.free_id(&want_id)
        } else {
            want_id.clone()
        };
        let xml = def.replace(
            &format!("w:styleId=\"{want_id}\""),
            &format!("w:styleId=\"{}\"", attr(&id)),
        );
        let kind = if def.contains("w:type=\"table\"") {
            StyleKind::Table
        } else if def.contains("w:type=\"character\"") {
            StyleKind::Character
        } else {
            StyleKind::Paragraph
        };
        self.append(doc, &xml)?;
        self.note_added(&id, name, kind);
        Ok(id)
    }

    fn note_added(&mut self, id: &str, name: &str, kind: StyleKind) {
        self.ids.insert(id.to_lowercase());
        self.added
            .insert(name.to_lowercase(), (id.to_string(), kind));
        self.added.insert(id.to_lowercase(), (id.to_string(), kind));
        self.added_names.push(name.to_string());
    }

    /// The style for a role, resolved once per session.
    pub fn role(
        &mut self,
        doc: &mut Document,
        roles: &Roles,
        role: Role,
    ) -> Result<Option<String>, Error> {
        if let Some((id, _)) = self.resolved.get(&role) {
            return Ok(id.clone());
        }
        let (id, source) = self.resolve(doc, roles, role)?;
        self.resolved.insert(role, (id.clone(), source));
        Ok(id)
    }

    /// The roles resolved so far, with where each style came from.
    pub fn sources(&self) -> Vec<(Role, RoleSource)> {
        let mut out: Vec<(Role, RoleSource)> = self
            .resolved
            .iter()
            .map(|(r, (_, s))| (*r, s.clone()))
            .collect();
        out.sort_by_key(|(r, _)| *r);
        out
    }

    fn resolve(
        &mut self,
        doc: &mut Document,
        roles: &Roles,
        role: Role,
    ) -> Result<(Option<String>, RoleSource), Error> {
        let kind = role.kind();
        if let Some(name) = roles.get(role) {
            return match self.find(kind, name) {
                Some(id) => Ok((Some(id), RoleSource::Chosen(name.to_string()))),
                None => Err(Error::Invalid(format!(
                    "the style {name:?} chosen for {} is not a {} style of the document; it has {}",
                    role.describe(),
                    kind_word(kind),
                    self.list_of(kind)
                ))),
            };
        }
        let wanted = role.built_in();
        if let Some(w) = &wanted
            && let Some(id) = self.find(kind, w)
        {
            return Ok((Some(id), RoleSource::BuiltIn(w.clone())));
        }
        let wanted_name = wanted
            .clone()
            .unwrap_or_else(|| format!("a style for {}", role.describe()));
        match role.fallback() {
            Fallback::DefaultParagraph => match self.default_paragraph() {
                Some((id, name)) => Ok((
                    Some(id),
                    RoleSource::Fallback {
                        wanted: wanted_name,
                        used: name,
                    },
                )),
                None => Ok((None, RoleSource::Unstyled)),
            },
            Fallback::Role(other) => {
                let id = self.role(doc, roles, other)?;
                let used = id
                    .as_deref()
                    .and_then(|i| self.styles.get(i).map(|s| s.name.clone()))
                    .or_else(|| id.clone())
                    .unwrap_or_else(|| "no style".into());
                Ok((
                    id,
                    RoleSource::Fallback {
                        wanted: wanted_name,
                        used,
                    },
                ))
            }
            Fallback::Add(name) => match self.find(kind, name) {
                Some(id) => Ok((
                    Some(id),
                    RoleSource::Fallback {
                        wanted: wanted_name,
                        used: name.to_string(),
                    },
                )),
                None => {
                    let id = self.add_builtin(doc, name)?;
                    Ok((
                        Some(id),
                        RoleSource::Fallback {
                            wanted: wanted_name,
                            used: format!("{name} (added)"),
                        },
                    ))
                }
            },
            Fallback::AddOwn => {
                let name = wanted.unwrap_or_default();
                let id = self.add_builtin(doc, &name)?;
                Ok((Some(id), RoleSource::Added(name)))
            }
        }
    }

    fn list_of(&self, kind: StyleKind) -> String {
        let mut names: Vec<&str> = self
            .styles
            .iter()
            .filter(|s| s.kind == kind)
            .map(|s| s.display_name.as_str())
            .collect();
        names.sort_unstable();
        names.dedup();
        if names.is_empty() {
            return format!("no {} styles", kind_word(kind));
        }
        let more = if names.len() > 20 { ", …" } else { "" };
        format!(
            "{}{more}",
            names
                .iter()
                .take(20)
                .map(|n| format!("{n:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }

    /// The id of a table style: one defined with the content, then one the
    /// document has, by id or name. A name that resolves to nothing is an
    /// error that lists the table styles the document has.
    pub fn table_style(
        &mut self,
        doc: &mut Document,
        defs: &[TableStyleDef],
        name: &str,
    ) -> Result<String, Error> {
        if let Some(def) = defs.iter().find(|d| d.name.eq_ignore_ascii_case(name)) {
            return self.ensure(doc, &TableStyle::Defined(def.clone()));
        }
        self.ensure(doc, &TableStyle::Named(name.to_string()))
    }

    /// Finds or adds a table style.
    pub fn ensure(&mut self, doc: &mut Document, style: &TableStyle) -> Result<String, Error> {
        match style {
            TableStyle::Named(name) => self.find(StyleKind::Table, name).ok_or_else(|| {
                Error::Invalid(format!(
                    "the document has no table style {name:?}; it has {}",
                    self.list_of(StyleKind::Table)
                ))
            }),
            TableStyle::Defined(def) => {
                if let Some(id) = self.find(StyleKind::Table, &def.name) {
                    return Ok(id);
                }
                if def.name.trim().is_empty() {
                    return Err(Error::Invalid("a table style needs a name".into()));
                }
                // The base: Normal Table, added if missing.
                let base = match self.find(StyleKind::Table, "Normal Table") {
                    Some(id) => id,
                    None => self.add_builtin(doc, "Normal Table")?,
                };
                let id = self.free_id(&def.name);
                let xml = table_style_xml(def, &id, &base);
                self.append(doc, &xml)?;
                self.note_added(&id, &def.name, StyleKind::Table);
                Ok(id)
            }
        }
    }
}

fn kind_word(k: StyleKind) -> &'static str {
    match k {
        StyleKind::Paragraph => "paragraph",
        StyleKind::Character => "character",
        StyleKind::Table => "table",
        StyleKind::Numbering => "numbering",
    }
}

/// A border element.
pub(crate) fn border_xml(side: &str, b: &Border) -> String {
    let val = match b.style {
        LineStyle::Single => "single",
        LineStyle::Double => "double",
        LineStyle::Dashed => "dashed",
        LineStyle::Dotted => "dotted",
        LineStyle::None => "nil",
    };
    if b.style == LineStyle::None {
        return format!("<w:{side} w:val=\"nil\"/>");
    }
    format!(
        "<w:{side} w:val=\"{val}\" w:sz=\"{}\" w:space=\"0\" w:color=\"{}\"/>",
        eighths(b.width),
        b.color.hex()
    )
}

fn shd(fill: &crate::content::Color) -> String {
    format!(
        "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>",
        fill.hex()
    )
}

fn look_rpr(l: &RowLook) -> String {
    let mut r = String::new();
    if l.bold {
        r.push_str("<w:b/><w:bCs/>");
    }
    if let Some(c) = &l.color {
        r.push_str(&format!("<w:color w:val=\"{}\"/>", c.hex()));
    }
    if r.is_empty() {
        String::new()
    } else {
        format!("<w:rPr>{r}</w:rPr>")
    }
}

/// A table style as Word stores one: the whole table's text, borders and
/// padding, then a section for the header row, the first column and the
/// banded rows.
pub(crate) fn table_style_xml(def: &TableStyleDef, id: &str, base: &str) -> String {
    let mut x = format!(
        "<w:style w:type=\"table\" w:customStyle=\"1\" w:styleId=\"{}\"><w:name w:val=\"{}\"/><w:basedOn w:val=\"{}\"/><w:uiPriority w:val=\"59\"/>",
        attr(id),
        attr(&def.name),
        attr(base)
    );
    x.push_str("<w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr>");
    let mut rpr = String::new();
    if let Some(f) = &def.font {
        let f = attr(f);
        rpr.push_str(&format!(
            "<w:rFonts w:ascii=\"{f}\" w:hAnsi=\"{f}\" w:cs=\"{f}\"/>"
        ));
    }
    if let Some(s) = def.size {
        let half = (s * 2.0).round() as i64;
        rpr.push_str(&format!(
            "<w:sz w:val=\"{half}\"/><w:szCs w:val=\"{half}\"/>"
        ));
    }
    if !rpr.is_empty() {
        x.push_str(&format!("<w:rPr>{rpr}</w:rPr>"));
    }
    x.push_str("<w:tblPr>");
    if def.band_fill.is_some() {
        x.push_str("<w:tblStyleRowBandSize w:val=\"1\"/>");
    }
    let b = &def.borders;
    let sides = [
        ("top", &b.top),
        ("left", &b.left),
        ("bottom", &b.bottom),
        ("right", &b.right),
        ("insideH", &b.inside_h),
        ("insideV", &b.inside_v),
    ];
    if sides.iter().any(|(_, s)| s.is_some()) {
        x.push_str("<w:tblBorders>");
        for (side, s) in sides {
            if let Some(s) = s {
                x.push_str(&border_xml(side, s));
            }
        }
        x.push_str("</w:tblBorders>");
    }
    if let Some(p) = &def.padding {
        x.push_str(&format!(
            "<w:tblCellMar><w:top w:w=\"{}\" w:type=\"dxa\"/><w:left w:w=\"{}\" w:type=\"dxa\"/><w:bottom w:w=\"{}\" w:type=\"dxa\"/><w:right w:w=\"{}\" w:type=\"dxa\"/></w:tblCellMar>",
            twips(p.top),
            twips(p.left),
            twips(p.bottom),
            twips(p.right)
        ));
    }
    x.push_str("</w:tblPr>");
    if let Some(h) = &def.header {
        x.push_str("<w:tblStylePr w:type=\"firstRow\">");
        x.push_str(&look_rpr(h));
        // Header rows repeat by the rows' own w:tblHeader. In the style,
        // it makes Word drop the rows' mark when it saves and then report
        // no header rows.
        x.push_str("<w:tblPr/><w:tcPr>");
        if let Some(bd) = &h.border {
            x.push_str(&format!(
                "<w:tcBorders>{}</w:tcBorders>",
                border_xml("bottom", bd)
            ));
        }
        if let Some(f) = &h.fill {
            x.push_str(&shd(f));
        }
        x.push_str("</w:tcPr></w:tblStylePr>");
    }
    if let Some(c) = &def.first_column {
        x.push_str("<w:tblStylePr w:type=\"firstCol\">");
        x.push_str(&look_rpr(c));
        x.push_str("<w:tblPr/><w:tcPr>");
        if let Some(bd) = &c.border {
            x.push_str(&format!(
                "<w:tcBorders>{}</w:tcBorders>",
                border_xml("right", bd)
            ));
        }
        if let Some(f) = &c.fill {
            x.push_str(&shd(f));
        }
        x.push_str("</w:tcPr></w:tblStylePr>");
    }
    if let Some(f) = &def.band_fill {
        x.push_str(&format!(
            "<w:tblStylePr w:type=\"band1Horz\"><w:tblPr/><w:tcPr>{}</w:tcPr></w:tblStylePr>",
            shd(f)
        ));
    }
    x.push_str("</w:style>");
    x
}

/// [`Document::ensure_table_style`] without the snapshot.
pub(crate) fn ensure_table_style(doc: &mut Document, style: &TableStyle) -> Result<String, Error> {
    let part = super::session::styles_part(doc)?;
    let styles = doc.views()?.styles().clone();
    let mut book = StyleBook::new(styles, part);
    book.ensure(doc, style)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_the_writer_may_add_is_defined() {
        let mut names: Vec<String> = (1..=9).map(|n| format!("heading {n}")).collect();
        names.extend(
            [
                "caption",
                "List Paragraph",
                "footnote text",
                "footnote reference",
                "Hyperlink",
                "HTML Code",
                "Normal Table",
                "Table Grid",
            ]
            .map(String::from),
        );
        for n in names {
            assert!(builtin(&n).is_some(), "no definition of {n}");
        }
    }
}
