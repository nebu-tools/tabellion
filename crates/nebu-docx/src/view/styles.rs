//! The styles of a document, read from its styles part.

use std::collections::HashMap;

use serde::Serialize;

use crate::Error;
use crate::xml::{NodeId, Tree};

use super::NumberingRef;
use super::names;
use super::props::{child_val, numbering_ref, on_off};
use super::walk::W;

/// What a style applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StyleKind {
    /// Paragraph styles.
    Paragraph,
    /// Character styles.
    Character,
    /// Table styles.
    Table,
    /// Numbering styles.
    Numbering,
}

/// The name of a built-in style in a language other than English.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LocalName {
    /// A language tag: `de`, `fr`.
    pub language: &'static str,
    /// The name Word shows in that language.
    pub name: &'static str,
}

/// One style.
#[derive(Debug, Clone, Serialize)]
pub struct Style {
    /// The id paragraphs and runs refer to it by (`w:styleId`).
    pub id: String,
    /// The name as the file stores it (`w:name`), the id if it has none.
    pub name: String,
    /// The name Word shows in English: `Heading 1` for the stored
    /// `heading 1`, `Comment Text` for `annotation text`.
    pub display_name: String,
    /// The names Word shows in other languages, for built-in styles.
    pub local_names: Vec<LocalName>,
    /// What it applies to.
    pub kind: StyleKind,
    /// One of Word's built-in styles.
    pub built_in: bool,
    /// The default style of its kind.
    pub default: bool,
    /// Hidden from Word's style lists.
    pub hidden: bool,
    /// The style it inherits from.
    pub based_on: Option<String>,
    /// The style of the paragraph after this one.
    pub next: Option<String>,
    /// The linked paragraph or character style.
    pub linked: Option<String>,
    /// The outline level of its paragraphs, 1 to 9, through the styles it
    /// is based on.
    pub outline_level: Option<u8>,
    /// The list its paragraphs are in, through the styles it is based on.
    pub numbering: Option<NumberingRef>,
    #[serde(skip)]
    own_outline: Option<u8>,
    #[serde(skip)]
    own_numbering: Option<NumberingRef>,
    #[serde(skip)]
    own_hidden_text: Option<bool>,
    #[serde(skip)]
    hidden_text: bool,
}

/// The styles of a document. Find a style by id, by stored name, or by the
/// name Word shows in English, German or French.
#[derive(Debug, Clone, Default)]
pub struct Styles {
    list: Vec<Style>,
    by_id: HashMap<String, usize>,
    default_hidden: bool,
}

/// A chain of `basedOn` longer than this is a loop or an attack.
const MAX_CHAIN: usize = 64;

impl Styles {
    /// Reads the styles part. `None` gives no styles.
    pub(crate) fn read(tree: Option<&Tree>) -> Result<Styles, Error> {
        let mut out = Styles::default();
        let Some(t) = tree else {
            return Ok(out);
        };
        let root = t.root_element();
        let mut latent: Vec<String> = vec![];
        for el in t.elements(root)? {
            if t.is(el, Some(W), "latentStyles")? {
                for e in t.elements(el)? {
                    if let Some(n) = t.attribute(e, Some(W), "name")? {
                        latent.push(n.to_ascii_lowercase());
                    }
                }
            } else if t.is(el, Some(W), "docDefaults")? {
                out.default_hidden = default_hidden(t, el)?;
            } else if t.is(el, Some(W), "style")? {
                out.list.push(style(t, el, &latent)?);
            }
        }
        for (i, s) in out.list.iter().enumerate() {
            out.by_id.entry(s.id.clone()).or_insert(i);
        }
        out.resolve();
        Ok(out)
    }

    fn resolve(&mut self) {
        for i in 0..self.list.len() {
            let mut outline = None;
            let mut numbering = None;
            let mut hidden = None;
            let mut j = Some(i);
            for _ in 0..MAX_CHAIN {
                let Some(k) = j else { break };
                let s = &self.list[k];
                outline = outline.or(s.own_outline);
                numbering = numbering.or_else(|| s.own_numbering.clone());
                hidden = hidden.or(s.own_hidden_text);
                j = s.based_on.as_ref().and_then(|b| self.by_id.get(b).copied());
                if j == Some(i) {
                    break;
                }
            }
            let s = &mut self.list[i];
            if outline.is_none()
                && let Some(n) = s.name.to_ascii_lowercase().strip_prefix("heading ")
                && let Ok(n @ 1..=9) = n.parse::<u8>()
            {
                outline = Some(n);
            }
            s.outline_level = outline;
            s.numbering = numbering;
            s.hidden_text = hidden.unwrap_or(self.default_hidden);
        }
    }

    /// All styles in the order of the styles part.
    pub fn iter(&self) -> impl Iterator<Item = &Style> {
        self.list.iter()
    }

    /// The number of styles.
    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// True if the document has no styles.
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// The style with this id.
    pub fn get(&self, id: &str) -> Option<&Style> {
        self.by_id.get(id).map(|&i| &self.list[i])
    }

    /// Finds a style by id, by stored name, by the English name Word shows,
    /// or by the German or French name of a built-in style, in this order.
    /// Names compare without case.
    pub fn find(&self, name: &str) -> Option<&Style> {
        self.find_kind(name, None)
    }

    /// Like [`find`](Self::find), among the styles of one kind.
    pub fn find_kind(&self, name: &str, kind: Option<StyleKind>) -> Option<&Style> {
        let ok = |s: &&Style| kind.is_none_or(|k| s.kind == k);
        if let Some(s) = self.get(name).filter(ok) {
            return Some(s);
        }
        let name = name.trim();
        let same = |a: &str| a.to_lowercase() == name.to_lowercase();
        self.list
            .iter()
            .filter(ok)
            .find(|s| same(&s.name))
            .or_else(|| self.list.iter().filter(ok).find(|s| same(&s.display_name)))
            .or_else(|| {
                self.list
                    .iter()
                    .filter(ok)
                    .find(|s| s.local_names.iter().any(|l| same(l.name)))
            })
            .or_else(|| self.list.iter().filter(ok).find(|s| same(&s.id)))
    }

    /// The default style of a kind.
    pub fn default_of(&self, kind: StyleKind) -> Option<&Style> {
        self.list.iter().find(|s| s.kind == kind && s.default)
    }

    /// The paragraph style with this id, or the default paragraph style.
    pub(crate) fn paragraph_style(&self, id: Option<&str>) -> Option<&Style> {
        match id.and_then(|i| self.get(i)) {
            Some(s) => Some(s),
            None => self.default_of(StyleKind::Paragraph),
        }
    }

    /// Whether text in this character style, in a paragraph of this style,
    /// is hidden.
    pub(crate) fn hidden(&self, char_style: Option<&str>, para_style: Option<&str>) -> bool {
        if let Some(s) = char_style.and_then(|c| self.get(c)) {
            return s.hidden_text;
        }
        match self.paragraph_style(para_style) {
            Some(s) => s.hidden_text,
            None => self.default_hidden,
        }
    }
}

fn default_hidden(t: &Tree, defaults: NodeId) -> Result<bool, Error> {
    if let Some(rd) = t.child(defaults, Some(W), "rPrDefault")?
        && let Some(rpr) = t.child(rd, Some(W), "rPr")?
        && let Some(v) = t.child(rpr, Some(W), "vanish")?
    {
        return on_off(t, v);
    }
    Ok(false)
}

fn style(t: &Tree, el: NodeId, latent: &[String]) -> Result<Style, Error> {
    let kind = match t.attribute(el, Some(W), "type")?.as_deref() {
        Some("character") => StyleKind::Character,
        Some("table") => StyleKind::Table,
        Some("numbering") => StyleKind::Numbering,
        _ => StyleKind::Paragraph,
    };
    let id = t
        .attribute(el, Some(W), "styleId")?
        .map(|v| v.into_owned())
        .unwrap_or_default();
    let name = child_val(t, el, "name")?.unwrap_or_else(|| id.clone());
    let custom = t
        .attribute(el, Some(W), "customStyle")?
        .is_some_and(|v| matches!(&*v, "1" | "true" | "on"));
    let row = if custom { None } else { names::built_in(&name) };
    let built_in = !custom && (row.is_some() || latent.contains(&name.to_ascii_lowercase()));
    let display_name = row.map(|r| r.1.to_string()).unwrap_or_else(|| name.clone());
    let local_names = row
        .map(|r| {
            vec![
                LocalName {
                    language: names::LANGUAGES[0],
                    name: r.2,
                },
                LocalName {
                    language: names::LANGUAGES[1],
                    name: r.3,
                },
            ]
        })
        .unwrap_or_default();
    let flag = |local| -> Result<bool, Error> {
        match t.child(el, Some(W), local)? {
            Some(c) => on_off(t, c),
            None => Ok(false),
        }
    };
    let mut own_outline = None;
    let mut own_numbering = None;
    if let Some(ppr) = t.child(el, Some(W), "pPr")? {
        own_outline = child_val(t, ppr, "outlineLvl")?
            .and_then(|v| v.parse::<u8>().ok())
            .filter(|&l| l < 9)
            .map(|l| l + 1);
        own_numbering = numbering_ref(t, ppr)?.map(|(id, level)| NumberingRef { id, level });
    }
    let mut own_hidden_text = None;
    if let Some(rpr) = t.child(el, Some(W), "rPr")?
        && let Some(v) = t.child(rpr, Some(W), "vanish")?
    {
        own_hidden_text = Some(on_off(t, v)?);
    }
    Ok(Style {
        id,
        name,
        display_name,
        local_names,
        kind,
        built_in,
        default: t
            .attribute(el, Some(W), "default")?
            .is_some_and(|v| matches!(&*v, "1" | "true" | "on")),
        hidden: flag("hidden")? || flag("semiHidden")?,
        based_on: child_val(t, el, "basedOn")?,
        next: child_val(t, el, "next")?,
        linked: child_val(t, el, "link")?,
        outline_level: None,
        numbering: None,
        own_outline,
        own_numbering,
        own_hidden_text,
        hidden_text: false,
    })
}
