//! The styles of a document, read from its styles part, and the cascade
//! that gives a run its toggle properties.

use std::collections::{HashMap, HashSet};

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

impl StyleKind {
    fn slot(self) -> usize {
        match self {
            StyleKind::Paragraph => 0,
            StyleKind::Character => 1,
            StyleKind::Table => 2,
            StyleKind::Numbering => 3,
        }
    }
}

/// The name of a built-in style in a language other than English.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LocalName {
    /// A language tag, such as `de`.
    pub language: &'static str,
    /// The name Word shows in that language.
    pub name: &'static str,
}

/// A toggle property of text (ECMA-376 part 1, 17.7.3): one that a style
/// switches on and off rather than sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Toggle {
    /// `w:b`.
    Bold,
    /// `w:bCs`, bold for complex scripts.
    BoldComplex,
    /// `w:i`.
    Italic,
    /// `w:iCs`, italic for complex scripts.
    ItalicComplex,
    /// `w:caps`.
    AllCaps,
    /// `w:smallCaps`.
    SmallCaps,
    /// `w:strike`.
    Strike,
    /// `w:dstrike`.
    DoubleStrike,
    /// `w:outline`.
    Outline,
    /// `w:shadow`.
    Shadow,
    /// `w:emboss`.
    Emboss,
    /// `w:imprint`.
    Imprint,
    /// `w:vanish`: hidden text.
    Hidden,
}

/// Every toggle property with its element name.
const TOGGLES: [(Toggle, &str); 13] = [
    (Toggle::Bold, "b"),
    (Toggle::BoldComplex, "bCs"),
    (Toggle::Italic, "i"),
    (Toggle::ItalicComplex, "iCs"),
    (Toggle::AllCaps, "caps"),
    (Toggle::SmallCaps, "smallCaps"),
    (Toggle::Strike, "strike"),
    (Toggle::DoubleStrike, "dstrike"),
    (Toggle::Outline, "outline"),
    (Toggle::Shadow, "shadow"),
    (Toggle::Emboss, "emboss"),
    (Toggle::Imprint, "imprint"),
    (Toggle::Hidden, "vanish"),
];

impl Toggle {
    fn bit(self) -> u16 {
        1 << TOGGLES.iter().position(|t| t.0 == self).unwrap_or(0)
    }
}

/// The toggle properties that are on for some text, after the style
/// cascade. Serialises as the list of those that are on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Toggles(u16);

impl Toggles {
    /// True if this property is on.
    pub fn has(self, t: Toggle) -> bool {
        self.0 & t.bit() != 0
    }

    /// The properties that are on.
    pub fn iter(self) -> impl Iterator<Item = Toggle> {
        TOGGLES.iter().map(|t| t.0).filter(move |&t| self.has(t))
    }
}

impl Serialize for Toggles {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(self.iter())
    }
}

/// The toggle properties one level of the cascade states: which it names,
/// and which of those it sets on. A property it does not name is not
/// specified there.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Stated {
    named: u16,
    on: u16,
}

impl Stated {
    /// Reads the toggle properties of a `w:rPr`, in one pass over it.
    pub(crate) fn read(tree: &Tree, rpr: Option<NodeId>) -> Result<Stated, Error> {
        let mut out = Stated::default();
        let Some(rpr) = rpr else { return Ok(out) };
        for c in tree.elements(rpr)? {
            let Some(name) = tree.name(c)? else { continue };
            if name.namespace != Some(W) {
                continue;
            }
            if let Some((t, _)) = TOGGLES.iter().find(|t| t.1 == name.local) {
                let bit = t.bit();
                out.named |= bit;
                if on_off(tree, c)? {
                    out.on |= bit;
                } else {
                    out.on &= !bit;
                }
            }
        }
        Ok(out)
    }

    /// What a style states after the styles it is based on: the nearest
    /// style that names a property decides it.
    fn over(self, base: Stated) -> Stated {
        Stated {
            named: self.named | base.named,
            on: (self.on & self.named) | (base.on & base.named & !self.named),
        }
    }
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
    custom: bool,
    #[serde(skip)]
    own_toggles: Stated,
    /// The toggle properties through the styles it is based on.
    #[serde(skip)]
    toggles: Stated,
}

/// The styles of a document. Find a style by id, by stored name, or by the
/// name Word shows.
///
/// Every lookup is a hash lookup; building the styles costs one pass over
/// the styles part and at most 64 steps of `basedOn` for each style.
#[derive(Debug, Clone, Default)]
pub struct Styles {
    list: Vec<Style>,
    by_id: HashMap<String, usize>,
    /// Lower-case names to styles, one map for each kind of name in the
    /// order [`Styles::find`] tries them: stored name, English name Word
    /// shows, names in other languages, id.
    by_name: [HashMap<String, Vec<usize>>; 4],
    defaults: [Option<usize>; 4],
    /// The document defaults' toggle properties.
    doc_defaults: Stated,
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
        let mut latent: HashSet<String> = HashSet::new();
        for el in t.elements(root)? {
            if t.is(el, Some(W), "latentStyles")? {
                for e in t.elements(el)? {
                    if let Some(n) = t.attribute(e, Some(W), "name")? {
                        latent.insert(n.to_lowercase());
                    }
                }
            } else if t.is(el, Some(W), "docDefaults")? {
                if let Some(rd) = t.child(el, Some(W), "rPrDefault")? {
                    out.doc_defaults = Stated::read(t, t.child(rd, Some(W), "rPr")?)?;
                }
            } else if t.is(el, Some(W), "style")? {
                out.list.push(style(t, el)?);
            }
        }
        for (i, s) in out.list.iter_mut().enumerate() {
            s.built_in = s.built_in || (!s.custom && latent.contains(&s.name.to_lowercase()));
            out.by_id.entry(s.id.clone()).or_insert(i);
            let slot = s.kind.slot();
            if s.default && out.defaults[slot].is_none() {
                out.defaults[slot] = Some(i);
            }
            let mut add = |map: usize, name: &str| {
                out.by_name[map]
                    .entry(name.trim().to_lowercase())
                    .or_default()
                    .push(i);
            };
            add(0, &s.name);
            add(1, &s.display_name);
            for l in &s.local_names {
                add(2, l.name);
            }
            add(3, &s.id);
        }
        out.resolve();
        Ok(out)
    }

    fn resolve(&mut self) {
        for i in 0..self.list.len() {
            let mut outline = None;
            let mut numbering = None;
            let mut toggles = Stated::default();
            let mut j = Some(i);
            for _ in 0..MAX_CHAIN {
                let Some(k) = j else { break };
                crate::cost::tick(1);
                let s = &self.list[k];
                outline = outline.or(s.own_outline);
                numbering = numbering.or_else(|| s.own_numbering.clone());
                toggles = toggles.over(s.own_toggles);
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
            s.toggles = toggles;
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

    /// Finds a style of any kind by id, by stored name, by the English name
    /// Word shows, or by its name in another language of the table of
    /// built-in names, in this order. Names compare without case.
    pub fn find(&self, name: &str) -> Option<&Style> {
        self.search(name, None)
    }

    /// Like [`find`](Self::find), among the styles of one kind only: a
    /// paragraph style and a table style may share a name in a file.
    pub fn find_of(&self, kind: StyleKind, name: &str) -> Option<&Style> {
        self.search(name, Some(kind))
    }

    fn search(&self, name: &str, kind: Option<StyleKind>) -> Option<&Style> {
        let ok = |s: &&Style| kind.is_none_or(|k| s.kind == k);
        if let Some(s) = self.get(name).filter(ok) {
            return Some(s);
        }
        let key = name.trim().to_lowercase();
        self.by_name.iter().find_map(|map| {
            map.get(&key)?
                .iter()
                .map(|&i| &self.list[i])
                .find(|s| ok(s))
        })
    }

    /// The default style of a kind.
    pub fn default_of(&self, kind: StyleKind) -> Option<&Style> {
        self.defaults[kind.slot()].map(|i| &self.list[i])
    }

    /// The paragraph style with this id, or the default paragraph style.
    pub(crate) fn paragraph_style(&self, id: Option<&str>) -> Option<&Style> {
        match id.and_then(|i| self.get(i)) {
            Some(s) => Some(s),
            None => self.default_of(StyleKind::Paragraph),
        }
    }

    /// The toggle properties of text after the cascade of ECMA-376 part 1,
    /// 17.7.2: document defaults, then the table style, the paragraph style
    /// and the character style, then direct formatting.
    ///
    /// Between the levels of styles a property toggles (17.7.3): a style
    /// that sets it on turns it over, one that sets it off leaves it as it
    /// was. Within one style, the nearest style of its `basedOn` chain that
    /// names the property decides it. Direct formatting sets it outright.
    /// A missing paragraph or table style is the default of its kind.
    pub(crate) fn toggles(
        &self,
        direct: Stated,
        character: Option<&str>,
        paragraph: Option<&str>,
        table: Option<&str>,
    ) -> Toggles {
        // The styles toggle among themselves; the document defaults give
        // only what no style states. Word does so: with bold in the
        // defaults, a character style that is bold leaves its text bold.
        let mut on = 0u16;
        let mut stated = 0u16;
        let table = match table {
            Some(id) => self.get(id),
            None => self.default_of(StyleKind::Table),
        };
        let styles = [
            table,
            self.paragraph_style(paragraph),
            character.and_then(|c| self.get(c)),
        ];
        for s in styles.into_iter().flatten() {
            on ^= s.toggles.on & s.toggles.named;
            stated |= s.toggles.named;
        }
        let defaults = self.doc_defaults.on & self.doc_defaults.named & !stated;
        on |= defaults;
        on = (on & !direct.named) | (direct.on & direct.named);
        Toggles(on)
    }
}

fn style(t: &Tree, el: NodeId) -> Result<Style, Error> {
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
    let display_name = row
        .map(|r| r.display.to_string())
        .unwrap_or_else(|| name.clone());
    let local_names = row.map(|r| r.local_names()).unwrap_or_default();
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
    let own_toggles = Stated::read(t, t.child(el, Some(W), "rPr")?)?;
    Ok(Style {
        id,
        name,
        display_name,
        local_names,
        kind,
        // Completed with the latent styles once they are all read.
        built_in: row.is_some(),
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
        custom,
        own_toggles,
        toggles: Stated::default(),
    })
}
