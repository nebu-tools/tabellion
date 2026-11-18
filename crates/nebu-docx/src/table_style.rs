//! One notion of a table style, whatever its source: a named table style the
//! template already has, or a definition written in Typst, R or JSON. A
//! definition becomes a real `w:style w:type="table"`, so Word lists it in the
//! table design gallery and a Word user can restyle the table later.

use serde::{Deserialize, Serialize};

use crate::document::Document;
use crate::styles::StyleKind;
use crate::xml::esc_attr;
use crate::Error;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Border {
    /// Line width in points.
    pub width: f32,
    /// RGB hex.
    #[serde(default = "black")]
    pub color: String,
}

fn black() -> String {
    "000000".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CellLook {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border_bottom: Option<Border>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TableStyleDef {
    /// The name Word shows. The style id is derived from it.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size_pt: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top: Option<Border>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bottom: Option<Border>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left: Option<Border>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right: Option<Border>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inside_h: Option<Border>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inside_v: Option<Border>,
    /// Cell padding left and right, in points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding_pt: Option<f32>,
    #[serde(default)]
    pub header: CellLook,
    /// Every other body row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub band_fill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_column: Option<CellLook>,
}

/// How a table names its style.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum TableStyle {
    /// An existing style, by id, name or localised built-in name.
    Named(String),
    Defined(TableStyleDef),
}

fn style_id(name: &str) -> String {
    let id: String = name.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if id.is_empty() { "TableStyle".into() } else { id }
}

fn border(tag: &str, b: &Option<Border>) -> String {
    match b {
        Some(b) if b.width > 0.0 => format!(
            "<w:{tag} w:val=\"single\" w:sz=\"{}\" w:space=\"0\" w:color=\"{}\"/>",
            (b.width * 8.0).round().clamp(2.0, 96.0) as i32,
            esc_attr(&b.color)
        ),
        Some(_) => format!("<w:{tag} w:val=\"nil\"/>"),
        None => String::new(),
    }
}

fn look(kind: &str, l: &CellLook) -> String {
    let mut rpr = String::new();
    if l.bold {
        rpr.push_str("<w:b/><w:bCs/>");
    }
    if l.italic {
        rpr.push_str("<w:i/><w:iCs/>");
    }
    if let Some(c) = &l.color {
        rpr.push_str(&format!("<w:color w:val=\"{}\"/>", esc_attr(c)));
    }
    let mut tcpr = String::new();
    if l.border_bottom.is_some() {
        tcpr.push_str(&format!("<w:tcBorders>{}</w:tcBorders>", border("bottom", &l.border_bottom)));
    }
    if let Some(f) = &l.fill {
        tcpr.push_str(&format!("<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>", esc_attr(f)));
    }
    format!(
        "<w:tblStylePr w:type=\"{kind}\">{}<w:tblPr/>{}</w:tblStylePr>",
        if rpr.is_empty() { String::new() } else { format!("<w:rPr>{rpr}</w:rPr>") },
        if tcpr.is_empty() { String::new() } else { format!("<w:tcPr>{tcpr}</w:tcPr>") }
    )
}

impl TableStyleDef {
    pub fn style_id(&self) -> String {
        style_id(&self.name)
    }

    pub fn to_xml(&self, based_on: Option<&str>) -> String {
        let id = self.style_id();
        let mut rpr = String::new();
        if let Some(f) = &self.font {
            let f = esc_attr(f);
            rpr.push_str(&format!("<w:rFonts w:ascii=\"{f}\" w:hAnsi=\"{f}\" w:cs=\"{f}\"/>"));
        }
        if let Some(s) = self.font_size_pt {
            let hp = (s * 2.0).round() as i32;
            rpr.push_str(&format!("<w:sz w:val=\"{hp}\"/><w:szCs w:val=\"{hp}\"/>"));
        }
        let borders = [
            border("top", &self.top),
            border("left", &self.left),
            border("bottom", &self.bottom),
            border("right", &self.right),
            border("insideH", &self.inside_h),
            border("insideV", &self.inside_v),
        ]
        .concat();
        let pad = self.padding_pt.map(|p| (p * 20.0).round() as i32).unwrap_or(108);
        let mut x = format!(
            "<w:style w:type=\"table\" w:customStyle=\"1\" w:styleId=\"{id}\"><w:name w:val=\"{}\"/>",
            esc_attr(&self.name)
        );
        if let Some(b) = based_on {
            x.push_str(&format!("<w:basedOn w:val=\"{b}\"/>"));
        }
        x.push_str("<w:uiPriority w:val=\"40\"/><w:qFormat/>");
        x.push_str("<w:pPr><w:spacing w:before=\"20\" w:after=\"20\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr>");
        if !rpr.is_empty() {
            x.push_str(&format!("<w:rPr>{rpr}</w:rPr>"));
        }
        x.push_str(&format!(
            "<w:tblPr><w:tblStyleRowBandSize w:val=\"1\"/>{}<w:tblCellMar><w:left w:w=\"{pad}\" w:type=\"dxa\"/><w:right w:w=\"{pad}\" w:type=\"dxa\"/></w:tblCellMar></w:tblPr>",
            if borders.is_empty() { String::new() } else { format!("<w:tblBorders>{borders}</w:tblBorders>") }
        ));
        x.push_str(&look("firstRow", &CellLook { ..self.header.clone() }));
        if let Some(fc) = &self.first_column {
            x.push_str(&look("firstCol", fc));
        }
        if let Some(b) = &self.band_fill {
            x.push_str(&look("band1Horz", &CellLook { fill: Some(b.clone()), ..Default::default() }));
        }
        x.push_str("</w:style>");
        x
    }
}

impl Document {
    /// Makes sure the table style exists and returns its style id. A defined
    /// style whose name the document already has is used as it stands, so a
    /// Word user's later changes to it are not overwritten.
    pub fn ensure_table_style(&mut self, style: &TableStyle) -> Result<String, Error> {
        match style {
            TableStyle::Named(n) => self
                .styles()?
                .resolve(n, Some(StyleKind::Table))
                .map(|s| s.id.clone())
                .ok_or_else(|| Error::Invalid(format!("the document has no table style named {n:?}"))),
            TableStyle::Defined(def) => {
                if let Some(s) = self.styles()?.resolve(&def.name, Some(StyleKind::Table)) {
                    return Ok(s.id.clone());
                }
                let mut id = def.style_id();
                while self.styles()?.get(&id).is_some() {
                    id.push('X');
                }
                let based = self.styles()?.resolve("Normal Table", Some(StyleKind::Table)).map(|s| s.id.clone());
                let xml = def.to_xml(based.as_deref()).replacen(&format!("w:styleId=\"{}\"", def.style_id()), &format!("w:styleId=\"{id}\""), 1);
                let t = self.pkg.xml("word/styles.xml")?;
                let root = t.document_element();
                let nodes = t.parse_fragment(&xml).map_err(|e| Error::Xml("table style".into(), e))?;
                for n in nodes {
                    t.append(root, n);
                }
                self.invalidate_styles();
                Ok(id)
            }
        }
    }
}
