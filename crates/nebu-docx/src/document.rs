//! A WordprocessingML document on top of the package: body blocks, styles,
//! ids, and the places where content can go.

use std::collections::HashMap;
use std::path::Path;

use crate::package::Package;
use crate::styles::Styles;
use crate::xml::{NodeId, Tree};
use crate::Error;

pub const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

/// What a top-level body child is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BlockKind {
    Paragraph,
    Table,
    /// A block-level content control.
    Control,
    /// The final section properties.
    Section,
    /// Anything else (bookmarks between blocks, custom XML, alternate content).
    Other,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BlockInfo {
    /// Stable across saves by Word when the paragraph has a `w14:paraId`.
    pub id: String,
    pub index: usize,
    pub kind: BlockKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading_level: Option<u8>,
    pub text: String,
    #[serde(skip)]
    pub node: NodeId,
}

/// Who and when, for tracked changes.
#[derive(Debug, Clone)]
pub struct Revision {
    pub author: String,
    /// ISO 8601 without fractional seconds, as Word writes it.
    pub date: String,
}

impl Revision {
    pub fn now(author: &str) -> Revision {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Revision { author: author.into(), date: iso8601(secs) }
    }
}

fn iso8601(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    // Civil from days, Howard Hinnant's algorithm.
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Hands out `w:id` values that no annotation or revision in the package
/// uses yet.
#[derive(Debug, Clone)]
pub struct Ids {
    next: u64,
    names: std::collections::HashSet<String>,
}

impl Ids {
    pub fn next(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    /// A bookmark name in Word's hidden `_Ref` form that is not taken.
    pub fn ref_name(&mut self) -> String {
        loop {
            let n = format!("_Ref{:09}", 100_000_000 + self.next());
            if self.names.insert(n.clone()) {
                return n;
            }
        }
    }

    /// The hidden bookmark that carries a label. Labels stay findable after
    /// the file is saved and opened again, by this tool or another.
    pub fn label_name(&mut self, label: &str) -> String {
        let n = label_bookmark(label);
        if self.names.insert(n.clone()) {
            return n;
        }
        // The label is taken; keep the new target reachable but distinct.
        self.ref_name()
    }
}

/// Bookmark name for a label: `_Ref_` and the label's letters, digits and
/// underscores, at most 40 characters as Word allows.
pub fn label_bookmark(label: &str) -> String {
    let clean: String = label.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    let mut n = format!("_Ref_{clean}");
    n.truncate(40);
    n
}

pub struct Document {
    pub pkg: Package,
    pub main: String,
    styles: Option<Styles>,
    ids: Option<Ids>,
}

impl Document {
    pub fn open(path: impl AsRef<Path>) -> Result<Document, Error> {
        Document::from_package(Package::open(path)?)
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Document, Error> {
        Document::from_package(Package::from_bytes(bytes)?)
    }

    pub fn from_package(mut pkg: Package) -> Result<Document, Error> {
        let main = pkg.main_part()?;
        let t = pkg.xml(&main)?;
        if t.ns_uri("w").as_deref() != Some(W_NS) {
            return Err(Error::Unsupported("the main part does not bind the prefix w to WordprocessingML".into()));
        }
        Ok(Document { pkg, main, styles: None, ids: None })
    }

    pub fn save(&mut self, path: impl AsRef<Path>) -> Result<(), Error> {
        self.pkg.save(path)
    }

    pub fn to_bytes(&mut self) -> Result<Vec<u8>, Error> {
        self.pkg.to_bytes()
    }

    pub fn tree(&mut self) -> &mut Tree {
        let main = self.main.clone();
        self.pkg.xml(&main).expect("main part parsed at open")
    }

    pub fn body(&mut self) -> NodeId {
        let t = self.tree();
        let root = t.document_element();
        t.child(root, "w:body").expect("document has a body")
    }

    pub fn styles(&mut self) -> Result<&Styles, Error> {
        if self.styles.is_none() {
            let s = if self.pkg.has("word/styles.xml") {
                Styles::parse(self.pkg.xml("word/styles.xml")?)
            } else {
                Styles::default()
            };
            self.styles = Some(s);
        }
        Ok(self.styles.as_ref().unwrap())
    }

    pub(crate) fn invalidate_styles(&mut self) {
        self.styles = None;
    }

    pub fn ids(&mut self) -> Result<&mut Ids, Error> {
        if self.ids.is_none() {
            let mut max = 0u64;
            let mut names = std::collections::HashSet::new();
            let parts: Vec<String> = self
                .pkg
                .part_names()
                .filter(|n| n.starts_with("word/") && n.ends_with(".xml"))
                .map(String::from)
                .collect();
            for p in parts {
                let bytes = self.pkg.read(&p)?;
                max = max.max(max_attr_number(&bytes, b" w:id=\""));
                collect_bookmark_names(&bytes, &mut names);
            }
            self.ids = Some(Ids { next: max, names });
        }
        Ok(self.ids.as_mut().unwrap())
    }

    /// The top-level blocks of the body, in order.
    pub fn blocks(&mut self) -> Result<Vec<BlockInfo>, Error> {
        let body = self.body();
        let styles = self.styles()?.clone();
        let t = self.tree();
        let mut out = vec![];
        for (index, node) in t.elements(body).enumerate() {
            let kind = match t.name(node) {
                "w:p" => BlockKind::Paragraph,
                "w:tbl" => BlockKind::Table,
                "w:sdt" => BlockKind::Control,
                "w:sectPr" => BlockKind::Section,
                _ => BlockKind::Other,
            };
            let first_p = if kind == BlockKind::Paragraph {
                Some(node)
            } else {
                t.descendants(node).find(|&d| t.is(d, "w:p"))
            };
            let para_id = first_p.and_then(|p| t.attr(p, "w14:paraId"));
            let id = match (kind, para_id) {
                (BlockKind::Paragraph, Some(p)) => format!("p:{p}"),
                (BlockKind::Table, Some(p)) => format!("t:{p}"),
                _ => format!("#{index}"),
            };
            let style_id = if kind == BlockKind::Paragraph { paragraph_style(t, node) } else { None };
            let style = style_id.as_deref().and_then(|s| styles.get(s));
            out.push(BlockInfo {
                id,
                index,
                kind,
                heading_level: style_id.as_deref().and_then(|s| styles.heading_level(s)),
                style_name: style.map(|s| s.name.clone()),
                style_id,
                text: visible_text(t, node),
                node,
            });
        }
        Ok(out)
    }
}

pub fn paragraph_style(t: &Tree, p: NodeId) -> Option<String> {
    let ppr = t.child(p, "w:pPr")?;
    let ps = t.child(ppr, "w:pStyle")?;
    t.attr(ps, "w:val")
}

/// Text a reader sees: no deleted text, no field codes.
pub fn visible_text(t: &Tree, node: NodeId) -> String {
    let mut s = String::new();
    visible_into(t, node, &mut s);
    s
}

fn visible_into(t: &Tree, node: NodeId, s: &mut String) {
    for &c in t.children(node) {
        match t.name(c) {
            "w:t" => s.push_str(&t.inner_text(c)),
            "w:tab" => s.push('\t'),
            "w:br" | "w:cr" => s.push('\n'),
            "w:noBreakHyphen" => s.push('\u{2011}'),
            "w:softHyphen" => s.push('\u{00AD}'),
            "w:p" if !s.is_empty() => {
                s.push('\n');
                visible_into(t, c, s);
            }
            "w:del" | "w:moveFrom" | "w:instrText" | "w:delText" | "w:delInstrText" | "w:rPr" | "w:pPr"
            | "w:tcPr" | "w:trPr" | "w:tblPr" => {}
            "w:tc" => {
                if !s.is_empty() && !s.ends_with('\n') {
                    s.push('\t');
                }
                visible_into(t, c, s);
            }
            _ if t.is_element(c) => visible_into(t, c, s),
            _ => {}
        }
    }
}

fn max_attr_number(bytes: &[u8], needle: &[u8]) -> u64 {
    let mut max = 0;
    let mut i = 0;
    while let Some(k) = bytes[i..].windows(needle.len()).position(|w| w == needle) {
        let s = i + k + needle.len();
        let e = bytes[s..].iter().position(|&c| c == b'"').map_or(s, |x| s + x);
        if let Ok(n) = std::str::from_utf8(&bytes[s..e]).unwrap_or("").parse::<u64>() {
            max = max.max(n);
        }
        i = e;
    }
    max
}

fn collect_bookmark_names(bytes: &[u8], names: &mut std::collections::HashSet<String>) {
    let needle = b"<w:bookmarkStart ";
    let mut i = 0;
    while let Some(k) = bytes[i..].windows(needle.len()).position(|w| w == needle) {
        let s = i + k;
        let e = bytes[s..].iter().position(|&c| c == b'>').map_or(bytes.len(), |x| s + x);
        let tag = &bytes[s..e];
        if let Some(p) = tag.windows(8).position(|w| w == b"w:name=\"") {
            let v = &tag[p + 8..];
            let end = v.iter().position(|&c| c == b'"').unwrap_or(v.len());
            names.insert(String::from_utf8_lossy(&v[..end]).into_owned());
        }
        i = e;
    }
}

/// Bookmark name -> the paragraph that holds its start.
pub fn bookmarks(t: &Tree, body: NodeId) -> HashMap<String, NodeId> {
    let mut m = HashMap::new();
    for d in t.descendants(body) {
        if t.is(d, "w:bookmarkStart") {
            if let Some(n) = t.attr(d, "w:name") {
                m.insert(n, d);
            }
        }
    }
    m
}

#[cfg(test)]
mod tests {
    #[test]
    fn iso_dates() {
        assert_eq!(super::iso8601(0), "1970-01-01T00:00:00Z");
        assert_eq!(super::iso8601(1_795_000_000), "2026-11-18T11:06:40Z");
    }
}

/// A place in the document, the way a person names it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Locator {
    /// A block id from [`Document::blocks`]: `p:<paraId>`, `t:<paraId>` or `#<index>`.
    Block(String),
    /// The body block that holds the start of a bookmark.
    Bookmark(String),
    /// The first heading whose text matches, ignoring case and surrounding space.
    Heading(String),
    /// The paragraph that holds `{{name}}`.
    Placeholder(String),
    /// The first body paragraph that contains this text.
    Text(String),
}

impl std::str::FromStr for Locator {
    type Err = Error;
    /// `bookmark:summary`, `heading:Results`, `placeholder:results`,
    /// `block:p:1A2B3C4D`, `text:goes here`, or `{{results}}`.
    fn from_str(s: &str) -> Result<Locator, Error> {
        if let Some(n) = s.strip_prefix("{{").and_then(|x| x.strip_suffix("}}")) {
            return Ok(Locator::Placeholder(n.trim().into()));
        }
        let (k, v) = s.split_once(':').ok_or_else(|| Error::Invalid(format!("not a place: {s:?}")))?;
        Ok(match k {
            "block" => Locator::Block(v.into()),
            "bookmark" => Locator::Bookmark(v.into()),
            "heading" => Locator::Heading(v.into()),
            "placeholder" => Locator::Placeholder(v.into()),
            "text" => Locator::Text(v.into()),
            _ => return Err(Error::Invalid(format!("unknown kind of place {k:?}"))),
        })
    }
}

impl Document {
    /// The body block a locator names. Fails if it names none or several.
    pub fn locate(&mut self, at: &Locator) -> Result<BlockInfo, Error> {
        let blocks = self.blocks()?;
        let hits: Vec<BlockInfo> = match at {
            Locator::Block(id) => blocks.into_iter().filter(|b| &b.id == id).collect(),
            Locator::Heading(h) => {
                let h = h.trim().to_lowercase();
                blocks.into_iter().filter(|b| b.heading_level.is_some() && b.text.trim().to_lowercase() == h).take(1).collect()
            }
            Locator::Placeholder(n) => {
                let pat = format!("{{{{{n}}}}}");
                blocks.into_iter().filter(|b| b.text.contains(&pat)).collect()
            }
            Locator::Text(s) => blocks.into_iter().filter(|b| b.kind == BlockKind::Paragraph && b.text.contains(s.as_str())).take(1).collect(),
            Locator::Bookmark(name) => {
                let body = self.body();
                let t = self.tree();
                let start = t.descendants(body).find(|&d| t.is(d, "w:bookmarkStart") && t.attr(d, "w:name").as_deref() == Some(name));
                match start {
                    None => vec![],
                    Some(s) => {
                        let top = std::iter::once(s).chain(t.ancestors(s)).find(|&a| t.parent(a) == Some(body));
                        blocks.into_iter().filter(|b| Some(b.node) == top).collect()
                    }
                }
            }
        };
        match hits.len() {
            1 => Ok(hits.into_iter().next().unwrap()),
            0 => Err(Error::Invalid(format!("nothing in the document matches {at:?}"))),
            n => Err(Error::Invalid(format!("{n} blocks match {at:?}; name one with a block id"))),
        }
    }
}
