//! A Word document: the package with its content types, relationships and
//! main part.

use std::path::Path;

use crate::package::{Package, check_part_name};
use crate::xml::Tree;
use crate::{Error, Limits, ns};

const CONTENT_TYPES: &str = "[Content_Types].xml";
const RELS_TYPE: &str = "application/vnd.openxmlformats-package.relationships+xml";
const OFFICE_DOCUMENT: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
const STRICT_OFFICE_DOCUMENT: &str =
    "http://purl.oclc.org/ooxml/officeDocument/relationships/officeDocument";
const WORD_MAIN_TYPES: [&str; 4] = [
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml",
    "application/vnd.ms-word.document.macroEnabled.main+xml",
    "application/vnd.ms-word.template.macroEnabledTemplate.main+xml",
];

/// A relationship from a part, or from the package, to another part or to
/// an external resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relationship {
    /// The id the source part refers to it by, such as `rId3`.
    pub id: String,
    /// The relationship type, a URI.
    pub kind: String,
    /// The target as written: a path relative to the source part, or a URI.
    pub target: String,
    /// True for a target outside the package, such as a hyperlink.
    pub external: bool,
    /// The name of the target part, if the target is inside the package and
    /// the part exists.
    pub part: Option<String>,
}

/// What a new relationship points to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target<'a> {
    /// A part of the package, by name (`word/media/image1.png`).
    Part(&'a str),
    /// A resource outside the package, by URI.
    External(&'a str),
}

impl Target<'_> {
    fn as_str(&self) -> &str {
        match self {
            Target::Part(s) | Target::External(s) => s,
        }
    }
}

/// A character XML allows.
fn xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..)
}

/// A Word document held in memory. See the [crate documentation](crate).
///
/// Part names are written as in the zip archive, without a leading slash
/// (`word/document.xml`), and compare without ASCII case, as the package
/// format requires.
pub struct Document {
    package: Package,
    main: String,
    /// The views as last read.
    pub(crate) views: Option<Box<crate::view::Cache>>,
    /// The number of insertions made, which seeds the ids the next one
    /// creates.
    pub(crate) writes: u64,
}

impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document")
            .field("main", &self.main)
            .finish_non_exhaustive()
    }
}

impl Document {
    /// Opens a .docx, .docm, .dotx or .dotm file with the default [`Limits`].
    pub fn open(path: impl AsRef<Path>) -> Result<Document, Error> {
        Document::open_with_limits(path, Limits::default())
    }

    /// Opens a file with the given limits.
    pub fn open_with_limits(path: impl AsRef<Path>, limits: Limits) -> Result<Document, Error> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|source| Error::Io {
            action: "read",
            path: path.into(),
            source,
        })?;
        Document::from_bytes_with_limits(bytes, limits)
    }

    /// Reads a document from bytes with the default [`Limits`].
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Result<Document, Error> {
        Document::from_bytes_with_limits(bytes, Limits::default())
    }

    /// Reads a document from bytes with the given limits.
    ///
    /// The package must have content types, a main document part related
    /// from the package, and that part must be a WordprocessingML document
    /// or template. Other parts are read only when asked for.
    pub fn from_bytes_with_limits(
        bytes: impl Into<Vec<u8>>,
        limits: Limits,
    ) -> Result<Document, Error> {
        let mut package = Package::from_bytes(bytes.into(), limits)?;
        if package.find(CONTENT_TYPES).is_none() {
            return Err(Error::NotWord(format!(
                "it has no {CONTENT_TYPES}, so it is not an Office Open XML package"
            )));
        }
        let types = package.xml(CONTENT_TYPES)?;
        if !types.is(types.root_element(), Some(ns::CONTENT_TYPES), "Types")? {
            return Err(Error::NotWord(format!(
                "{CONTENT_TYPES} does not list content types"
            )));
        }
        let mut doc = Document {
            package,
            main: String::new(),
            views: None,
            writes: 0,
        };
        let rels = doc.relationships("")?;
        let Some(rel) = rels
            .iter()
            .find(|r| r.kind == OFFICE_DOCUMENT || r.kind == STRICT_OFFICE_DOCUMENT)
        else {
            return Err(Error::NotWord("the package has no main document".into()));
        };
        if rel.kind == STRICT_OFFICE_DOCUMENT {
            return Err(Error::Unsupported(
                "Strict Open XML documents; save the file in Word as a Word Document (.docx)"
                    .into(),
            ));
        }
        let Some(main) = rel.part.clone() else {
            return Err(Error::MissingPart(format!(
                "{} (the main document)",
                rel.target
            )));
        };
        match doc.content_type(&main) {
            Some(t) if WORD_MAIN_TYPES.contains(&t.as_str()) => {}
            Some(t) => {
                return Err(Error::NotWord(format!(
                    "its main part {main} has the content type {t}"
                )));
            }
            None => {
                return Err(Error::NotWord(format!(
                    "its main part {main} has no content type"
                )));
            }
        }
        doc.main = main;
        Ok(doc)
    }

    /// Writes the document to a file. The file is written next to its final
    /// place first and then renamed, so a failed save leaves an existing file
    /// as it was.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), Error> {
        let path = path.as_ref();
        let bytes = self.to_bytes()?;
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let tmp = path.with_file_name(format!(".{file}.{}.tmp", std::process::id()));
        let written = std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, path));
        if let Err(source) = written {
            let _ = std::fs::remove_file(&tmp);
            return Err(Error::Io {
                action: "write",
                path: path.into(),
                source,
            });
        }
        Ok(())
    }

    /// The document as the bytes of a .docx file. Parts nobody changed are
    /// the same zip entries as in the source.
    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        self.package.to_bytes()
    }

    /// The SHA-256 of the file the document was read from, in lower-case
    /// hex. A `#<index>` block id holds for this file only.
    pub fn sha256(&self) -> String {
        crate::sha256::hex(self.package.source_bytes())
    }

    /// The parts as they are, to put back with [`Document::restore`].
    pub(crate) fn snapshot(&self) -> (crate::package::Snapshot, u64) {
        (self.package.snapshot(), self.writes)
    }

    /// Puts the parts back as a snapshot had them. The views are read
    /// again on next use.
    pub(crate) fn restore(&mut self, s: (crate::package::Snapshot, u64)) {
        self.package.restore(s.0);
        self.writes = s.1;
        self.views = None;
    }

    /// A copy of one parsed part's tree, to put back with
    /// [`Document::restore_part`] if an edit of that part fails.
    pub(crate) fn snapshot_part(&mut self, name: &str) -> Result<Tree, Error> {
        self.package.snapshot_part(name)
    }

    /// Puts back a part tree taken by [`Document::snapshot_part`]. The
    /// views are read again on next use.
    pub(crate) fn restore_part(&mut self, name: &str, tree: Tree) {
        self.package.restore_part(name, tree);
        self.views = None;
    }

    /// The stamp of the parsed trees and the list of parts.
    pub(crate) fn stamp(&self) -> Vec<(u64, u64)> {
        self.package.stamp()
    }

    /// The limits the document was opened with.
    pub(crate) fn limits(&self) -> &Limits {
        self.package.limits()
    }

    /// The tree of a part that has been parsed already.
    pub(crate) fn parsed(&self, name: &str) -> Option<&Tree> {
        self.package.parsed(name)
    }

    /// The main document part, usually `word/document.xml`.
    pub fn main_part(&self) -> &str {
        &self.main
    }

    /// The names of all parts in archive order.
    pub fn parts(&self) -> impl Iterator<Item = &str> {
        self.package.names()
    }

    /// The part's name as the package spells it, if the part exists.
    pub fn part_name(&self, name: &str) -> Option<&str> {
        self.package.find(name)
    }

    /// The bytes of a part, with the changes made so far.
    pub fn read_part(&self, name: &str) -> Result<Vec<u8>, Error> {
        self.package.read(name)
    }

    /// The XML tree of a part, parsed on first use. Changes to the tree are
    /// saved with the document.
    pub fn xml(&mut self, name: &str) -> Result<&mut Tree, Error> {
        self.package.xml(name)
    }

    /// The content type of a part, from an override for the part or the
    /// default for its extension.
    pub fn content_type(&self, name: &str) -> Option<String> {
        let t = self.package.parsed(CONTENT_TYPES)?;
        // The handles come from the tree itself, so the lookups cannot fail.
        content_type_in(t, name).ok().flatten()
    }

    /// The relationships of a part, in the order of its relationships part.
    /// An empty `source` names the package itself.
    pub fn relationships(&mut self, source: &str) -> Result<Vec<Relationship>, Error> {
        let source = self.source_name(source)?;
        let rels = rels_name(&source);
        let Some(rels) = self.package.find(&rels).map(str::to_string) else {
            return Ok(vec![]);
        };
        let t = self.package.xml(&rels)?;
        let root = t.root_element();
        let mut list: Vec<(String, String, String, bool)> = vec![];
        for r in t.elements(root)? {
            if !t.is(r, Some(ns::RELATIONSHIPS), "Relationship")? {
                continue;
            }
            let a = |n| -> Result<String, Error> {
                Ok(t.attribute(r, None, n)?
                    .map(|v| v.into_owned())
                    .unwrap_or_default())
            };
            list.push((
                a("Id")?,
                a("Type")?,
                a("Target")?,
                a("TargetMode")? == "External",
            ));
        }
        Ok(list
            .into_iter()
            .map(|(id, kind, target, external)| {
                let part = if external {
                    None
                } else {
                    resolve(&source, &target)
                        .and_then(|p| self.package.find(&p).map(str::to_string))
                };
                Relationship {
                    id,
                    kind,
                    target,
                    external,
                    part,
                }
            })
            .collect())
    }

    /// Adds a relationship from a part (or, with an empty `source`, from the
    /// package) and returns its new id. A target part must exist.
    pub fn add_relationship(
        &mut self,
        source: &str,
        kind: &str,
        target: Target<'_>,
    ) -> Result<String, Error> {
        let source = self.source_name(source)?;
        for v in [kind, target.as_str()] {
            if let Some(c) = v.chars().find(|&c| !xml_char(c)) {
                return Err(Error::Edit(format!(
                    "U+{:04X} cannot appear in a relationship",
                    c as u32
                )));
            }
        }
        let (target, external) = match target {
            Target::Part(p) => {
                let Some(p) = self.package.find(p) else {
                    return Err(Error::MissingPart(p.to_string()));
                };
                (relative(&source, p), false)
            }
            Target::External(uri) => (uri.to_string(), true),
        };
        let next = self.next_relationship_id(&source)?;
        let id = format!("rId{next}");
        self.write_relationship(&source, &id, kind, &target, external)?;
        Ok(id)
    }

    /// The number after the highest `rIdN` of a part's relationships.
    pub(crate) fn next_relationship_id(&mut self, source: &str) -> Result<u64, Error> {
        Ok(self
            .relationships(source)?
            .iter()
            .filter_map(|r| r.id.strip_prefix("rId")?.parse::<u64>().ok())
            .max()
            .unwrap_or(0)
            + 1)
    }

    /// Adds a relationship with an id the caller has made free, without
    /// reading the relationships again. The target is checked as
    /// [`add_relationship`](Self::add_relationship) checks it.
    pub(crate) fn add_relationship_as(
        &mut self,
        source: &str,
        id: &str,
        kind: &str,
        target: Target<'_>,
    ) -> Result<(), Error> {
        let source = self.source_name(source)?;
        for v in [kind, target.as_str(), id] {
            if let Some(c) = v.chars().find(|&c| !xml_char(c)) {
                return Err(Error::Edit(format!(
                    "U+{:04X} cannot appear in a relationship",
                    c as u32
                )));
            }
        }
        let (target, external) = match target {
            Target::Part(p) => {
                let Some(p) = self.package.find(p) else {
                    return Err(Error::MissingPart(p.to_string()));
                };
                (relative(&source, p), false)
            }
            Target::External(uri) => (uri.to_string(), true),
        };
        self.write_relationship(&source, id, kind, &target, external)
    }

    fn write_relationship(
        &mut self,
        source: &str,
        id: &str,
        kind: &str,
        target: &str,
        external: bool,
    ) -> Result<(), Error> {
        let rels = rels_name(source);
        if self.package.find(&rels).is_none() {
            let empty = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n<Relationships xmlns=\"{}\"></Relationships>",
                ns::RELATIONSHIPS
            );
            self.add_part(&rels, RELS_TYPE, empty.into_bytes())?;
        }
        let t = self.package.xml(&rels)?;
        let root = t.root_element();
        let r = t.create_element(Some(ns::RELATIONSHIPS), "Relationship")?;
        t.set_attribute(r, None, "Id", id)?;
        t.set_attribute(r, None, "Type", kind)?;
        t.set_attribute(r, None, "Target", target)?;
        if external {
            t.set_attribute(r, None, "TargetMode", "External")?;
        }
        t.append(root, r)?;
        Ok(())
    }

    /// Adds a `Default` content type for an extension, which parts of that
    /// extension then take without an override of their own.
    pub(crate) fn add_default_type(&mut self, ext: &str, content_type: &str) -> Result<(), Error> {
        let t = self.package.xml(CONTENT_TYPES)?;
        let root = t.root_element();
        let d = t.create_element(Some(ns::CONTENT_TYPES), "Default")?;
        t.set_attribute(d, None, "Extension", ext)?;
        t.set_attribute(d, None, "ContentType", content_type)?;
        t.append(root, d)?;
        Ok(())
    }

    /// Adds a part whose content type the content types part gives
    /// already, without reading it again. The name is checked as
    /// [`add_part`](Self::add_part) checks it.
    pub(crate) fn add_typed_part(&mut self, name: &str, bytes: Vec<u8>) -> Result<(), Error> {
        check_part_name(name)?;
        if !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~/".contains(&b))
            || name.ends_with('/')
        {
            return Err(Error::PartName {
                name: name.into(),
                reason: "use ASCII letters, digits, '-', '.', '_', '~' and '/'",
            });
        }
        if self.package.find(name).is_some() {
            return Err(Error::PartExists(name.to_string()));
        }
        self.package.add(name, bytes);
        Ok(())
    }

    /// Adds a part with its content type. The content types part gets an
    /// override unless the default for the extension is already that type.
    /// Names of new parts use ASCII letters, digits, `-`, `.`, `_`, `~`
    /// and `/`.
    pub fn add_part(
        &mut self,
        name: &str,
        content_type: &str,
        bytes: Vec<u8>,
    ) -> Result<(), Error> {
        check_part_name(name)?;
        if name.ends_with('/')
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-._~/".contains(&b))
        {
            return Err(Error::PartName {
                name: name.into(),
                reason: "use ASCII letters, digits, '-', '.', '_', '~' and '/'",
            });
        }
        if self.package.find(name).is_some() || name.eq_ignore_ascii_case(CONTENT_TYPES) {
            return Err(Error::PartExists(name.to_string()));
        }
        if content_type.is_empty()
            || content_type
                .chars()
                .any(|c| c.is_control() || c.is_whitespace() || !xml_char(c))
        {
            return Err(Error::Edit(format!(
                "{content_type:?} is not a content type"
            )));
        }
        if self.content_type(name).as_deref() != Some(content_type) {
            let t = self.package.xml(CONTENT_TYPES)?;
            let root = t.root_element();
            let o = t.create_element(Some(ns::CONTENT_TYPES), "Override")?;
            t.set_attribute(o, None, "PartName", &format!("/{name}"))?;
            t.set_attribute(o, None, "ContentType", content_type)?;
            t.append(root, o)?;
        }
        self.package.add(name, bytes);
        Ok(())
    }

    /// Swaps the bytes of an image part for a new export of the same
    /// picture: PNG for PNG, JPEG for JPEG, and so on. The drawing keeps
    /// its placed size, and every reference to the part shows the new
    /// bytes. Anything else is an error that names the part and both
    /// kinds.
    pub fn replace_image(&mut self, name: &str, bytes: Vec<u8>) -> Result<(), Error> {
        let actual = self
            .package
            .find(name)
            .map(str::to_string)
            .ok_or_else(|| Error::MissingPart(name.to_string()))?;
        let old = self.content_type(&actual).unwrap_or_default();
        let limits = self.limits().clone();
        crate::writer::media::check_replacement(&actual, &old, &bytes, &limits)?;
        self.package.set_bytes(&actual, bytes);
        Ok(())
    }

    /// The source part's name as the package spells it; empty for the
    /// package.
    fn source_name(&self, source: &str) -> Result<String, Error> {
        if source.is_empty() {
            return Ok(String::new());
        }
        self.package
            .find(source)
            .map(str::to_string)
            .ok_or_else(|| Error::MissingPart(source.to_string()))
    }
}

/// The content type of a part in the content types tree, from an override
/// for the part or the default for its extension.
fn content_type_in(t: &Tree, name: &str) -> Result<Option<String>, Error> {
    let part = format!("/{name}");
    let ext = name
        .rsplit('/')
        .next()
        .and_then(|f| f.rsplit_once('.'))
        .map(|(_, e)| e);
    let mut default = None;
    for c in t.elements(t.root_element())? {
        let ctype = || -> Result<Option<String>, Error> {
            Ok(t.attribute(c, None, "ContentType")?.map(|v| v.into_owned()))
        };
        if t.is(c, Some(ns::CONTENT_TYPES), "Override")?
            && t.attribute(c, None, "PartName")?
                .is_some_and(|p| percent_decode(&p).eq_ignore_ascii_case(&part))
        {
            return ctype();
        }
        if default.is_none()
            && t.is(c, Some(ns::CONTENT_TYPES), "Default")?
            && let (Some(e), Some(want)) = (t.attribute(c, None, "Extension")?, ext)
            && e.eq_ignore_ascii_case(want)
        {
            default = ctype()?;
        }
    }
    Ok(default)
}

/// The relationships part of a part: `word/_rels/document.xml.rels` for
/// `word/document.xml`, `_rels/.rels` for the package.
fn rels_name(source: &str) -> String {
    match source.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{source}.rels"),
    }
}

/// The directory segments of a part's name.
fn dir_of(part: &str) -> Vec<&str> {
    let mut segs: Vec<&str> = part.split('/').collect();
    segs.pop();
    segs.retain(|s| !s.is_empty());
    segs
}

/// Resolves a relationship target against its source part. `None` if it
/// leaves the package.
fn resolve(source: &str, target: &str) -> Option<String> {
    let target = target.split(['#', '?']).next().unwrap_or_default();
    let target = percent_decode(target);
    let (mut segs, rest) = match target.strip_prefix('/') {
        Some(abs) => (vec![], abs.to_string()),
        None => (
            dir_of(source)
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>(),
            target.clone(),
        ),
    };
    for seg in rest.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                segs.pop()?;
            }
            s => segs.push(s.to_string()),
        }
    }
    (!segs.is_empty()).then(|| segs.join("/"))
}

/// The target that leads from `source` to `part`.
fn relative(source: &str, part: &str) -> String {
    let from = dir_of(source);
    let to: Vec<&str> = part.split('/').collect();
    let common = from
        .iter()
        .zip(&to)
        .take_while(|(a, b)| a.eq_ignore_ascii_case(b))
        .count();
    let common = common.min(to.len().saturating_sub(1));
    let mut segs: Vec<&str> = vec![".."; from.len() - common];
    segs.extend(&to[common..]);
    segs.join("/")
}

/// Decodes `%XX` escapes; leaves a malformed escape as written.
fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%'
            && let (Some(h), Some(l)) = (
                b.get(i + 1).and_then(|&c| hex(c)),
                b.get(i + 2).and_then(|&c| hex(c)),
            )
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_resolve_against_the_source() {
        assert_eq!(
            resolve("word/document.xml", "media/image1.png").as_deref(),
            Some("word/media/image1.png")
        );
        assert_eq!(
            resolve("word/document.xml", "../customXml/item1.xml").as_deref(),
            Some("customXml/item1.xml")
        );
        assert_eq!(
            resolve("", "word/document.xml").as_deref(),
            Some("word/document.xml")
        );
        assert_eq!(
            resolve("word/document.xml", "/word/styles.xml").as_deref(),
            Some("word/styles.xml")
        );
        assert_eq!(
            resolve("word/document.xml", "media/a%20b.png#x").as_deref(),
            Some("word/media/a b.png")
        );
        assert_eq!(resolve("word/document.xml", "../../etc/passwd"), None);
    }

    #[test]
    fn targets_are_written_relative() {
        assert_eq!(
            relative("word/document.xml", "word/media/image1.png"),
            "media/image1.png"
        );
        assert_eq!(
            relative("word/document.xml", "customXml/item1.xml"),
            "../customXml/item1.xml"
        );
        assert_eq!(relative("", "word/document.xml"), "word/document.xml");
        assert_eq!(
            relative("word/document.xml", "word/styles.xml"),
            "styles.xml"
        );
    }

    #[test]
    fn rels_names() {
        assert_eq!(
            rels_name("word/document.xml"),
            "word/_rels/document.xml.rels"
        );
        assert_eq!(rels_name(""), "_rels/.rels");
    }
}
