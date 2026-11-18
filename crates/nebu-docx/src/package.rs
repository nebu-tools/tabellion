//! The OPC package: a zip of parts with content types and relationships.
//!
//! Parts that nobody asked to change are copied into the output as the same
//! compressed bytes, so they stay identical down to the zip entry. A part that
//! was parsed but not edited is also copied raw.

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::path::Path;

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::xml::{self, Tree};
use crate::Error;

pub const REL_IMAGE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
pub const REL_FOOTNOTES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes";
pub const REL_HYPERLINK: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";
pub const REL_OFFICE_DOCUMENT: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";

enum Body {
    /// Unchanged: copy entry `index` of the source archive.
    Source(usize),
    Bytes(Vec<u8>),
    Xml(Box<Tree>),
}

struct Part {
    name: String,
    body: Body,
    removed: bool,
}

pub struct Package {
    archive: ZipArchive<Cursor<Vec<u8>>>,
    parts: Vec<Part>,
    index: BTreeMap<String, usize>,
    /// Parsed parts whose source entry can still be copied if they stay unedited.
    pristine: BTreeMap<usize, usize>,
}

/// One relationship of a part.
#[derive(Debug, Clone)]
pub struct Rel {
    pub id: String,
    pub kind: String,
    pub target: String,
    pub external: bool,
}

impl Package {
    pub fn open(path: impl AsRef<Path>) -> Result<Package, Error> {
        let bytes = std::fs::read(path.as_ref())?;
        Package::from_bytes(bytes)
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Package, Error> {
        let archive = ZipArchive::new(Cursor::new(bytes))?;
        let mut parts = vec![];
        let mut index = BTreeMap::new();
        for i in 0..archive.len() {
            let name = archive.name_for_index(i).and_then(|r| r.ok()).map(|s| s.to_string()).unwrap_or_default();
            index.insert(name.clone(), parts.len());
            parts.push(Part { name, body: Body::Source(i), removed: false });
        }
        Ok(Package { archive, parts, index, pristine: BTreeMap::new() })
    }

    pub fn part_names(&self) -> impl Iterator<Item = &str> {
        self.parts.iter().filter(|p| !p.removed).map(|p| p.name.as_str())
    }

    pub fn has(&self, name: &str) -> bool {
        self.index.get(name).is_some_and(|&i| !self.parts[i].removed)
    }

    /// The bytes of a part as they would be written now.
    pub fn read(&mut self, name: &str) -> Result<Vec<u8>, Error> {
        let i = *self.index.get(name).ok_or_else(|| Error::MissingPart(name.into()))?;
        match &self.parts[i].body {
            Body::Source(k) => {
                let mut f = self.archive.by_index(*k)?;
                let mut v = Vec::with_capacity(f.size() as usize);
                f.read_to_end(&mut v)?;
                Ok(v)
            }
            Body::Bytes(b) => Ok(b.clone()),
            Body::Xml(t) => Ok(t.write()),
        }
    }

    /// The parsed XML of a part. The tree records edits; untouched nodes keep
    /// their bytes, and an unedited tree is written as the original entry.
    pub fn xml(&mut self, name: &str) -> Result<&mut Tree, Error> {
        let i = *self.index.get(name).ok_or_else(|| Error::MissingPart(name.into()))?;
        if let Body::Source(_) | Body::Bytes(_) = self.parts[i].body {
            let bytes = self.read(name)?;
            let tree = Tree::parse(bytes).map_err(|e| Error::Xml(name.into(), e))?;
            let keep = match self.parts[i].body {
                Body::Source(k) => Some(k),
                _ => None,
            };
            self.parts[i].body = Body::Xml(Box::new(tree));
            if let Some(k) = keep {
                self.pristine.insert(i, k);
            }
        }
        match &mut self.parts[i].body {
            Body::Xml(t) => Ok(t),
            _ => unreachable!(),
        }
    }

    pub fn put(&mut self, name: &str, bytes: Vec<u8>) {
        match self.index.get(name) {
            Some(&i) => {
                self.parts[i].body = Body::Bytes(bytes);
                self.parts[i].removed = false;
                self.pristine.remove(&i);
            }
            None => {
                self.index.insert(name.into(), self.parts.len());
                self.parts.push(Part { name: name.into(), body: Body::Bytes(bytes), removed: false });
            }
        }
    }

    pub fn put_xml(&mut self, name: &str, xml: &str) -> Result<(), Error> {
        self.put(name, xml.as_bytes().to_vec());
        self.xml(name)?;
        Ok(())
    }

    pub fn remove(&mut self, name: &str) {
        if let Some(&i) = self.index.get(name) {
            self.parts[i].removed = true;
        }
    }

    // ---- content types and relationships ---------------------------------

    /// Declares the content type of a part, or a default for an extension.
    pub fn ensure_content_type(&mut self, part: Option<&str>, ext: Option<&str>, ctype: &str) -> Result<(), Error> {
        let t = self.xml("[Content_Types].xml")?;
        let root = t.document_element();
        let want = part.map(|p| format!("/{p}"));
        let exists = t.elements(root).any(|c| match (&want, ext) {
            (Some(p), _) => t.is(c, "Override") && t.attr(c, "PartName").as_deref() == Some(p.as_str()),
            (None, Some(e)) => t.is(c, "Default") && t.attr(c, "Extension").is_some_and(|x| x.eq_ignore_ascii_case(e)),
            _ => true,
        });
        if exists {
            return Ok(());
        }
        let n = match (part, ext) {
            (Some(p), _) => t.new_element("Override", &[("PartName", &format!("/{p}")), ("ContentType", ctype)]),
            (_, Some(e)) => t.new_element("Default", &[("Extension", e), ("ContentType", ctype)]),
            _ => return Ok(()),
        };
        // Defaults come first by convention.
        if ext.is_some() {
            t.insert_at(root, 0, n);
        } else {
            t.append(root, n);
        }
        Ok(())
    }

    fn rels_name(part: &str) -> String {
        match part.rsplit_once('/') {
            Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
            None => format!("_rels/{part}.rels"),
        }
    }

    pub fn rels(&mut self, part: &str) -> Result<Vec<Rel>, Error> {
        let name = Self::rels_name(part);
        if !self.has(&name) {
            return Ok(vec![]);
        }
        let t = self.xml(&name)?;
        let root = t.document_element();
        Ok(t.elements(root)
            .map(|r| Rel {
                id: t.attr(r, "Id").unwrap_or_default(),
                kind: t.attr(r, "Type").unwrap_or_default(),
                target: t.attr(r, "Target").unwrap_or_default(),
                external: t.attr(r, "TargetMode").as_deref() == Some("External"),
            })
            .collect())
    }

    /// Adds a relationship from `part` and returns its id. Ids never collide
    /// with existing ones.
    pub fn add_rel(&mut self, part: &str, kind: &str, target: &str, external: bool) -> Result<String, Error> {
        let name = Self::rels_name(part);
        if !self.has(&name) {
            self.put_xml(&name, "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"></Relationships>")?;
        }
        let existing = self.rels(part)?;
        let mut n = existing.len() + 1;
        let id = loop {
            let id = format!("rIdT{n}");
            if !existing.iter().any(|r| r.id == id) {
                break id;
            }
            n += 1;
        };
        let t = self.xml(&name)?;
        let root = t.document_element();
        let mut attrs = vec![("Id", id.as_str()), ("Type", kind), ("Target", target)];
        if external {
            attrs.push(("TargetMode", "External"));
        }
        let r = t.new_element("Relationship", &attrs);
        t.append(root, r);
        Ok(id)
    }

    /// The main document part, from the package relationships.
    pub fn main_part(&mut self) -> Result<String, Error> {
        let rels = self.rels("")?;
        rels.iter()
            .find(|r| r.kind == REL_OFFICE_DOCUMENT || r.kind.ends_with("/officeDocument"))
            .map(|r| r.target.trim_start_matches('/').to_string())
            .ok_or_else(|| Error::MissingPart("officeDocument relationship".into()))
    }

    // ---- writing ----------------------------------------------------------

    pub fn to_bytes(&mut self) -> Result<Vec<u8>, Error> {
        let mut w = ZipWriter::new(Cursor::new(Vec::new()));
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for i in 0..self.parts.len() {
            if self.parts[i].removed {
                continue;
            }
            let raw = match &self.parts[i].body {
                Body::Source(k) => Some(*k),
                Body::Xml(t) if t.is_pristine() => self.pristine.get(&i).copied(),
                _ => None,
            };
            if let Some(k) = raw {
                let f = self.archive.by_index_raw(k)?;
                w.raw_copy_file(f)?;
                continue;
            }
            let bytes = match &self.parts[i].body {
                Body::Bytes(b) => b.clone(),
                Body::Xml(t) => t.write(),
                Body::Source(_) => unreachable!(),
            };
            w.start_file(self.parts[i].name.clone(), opts)?;
            w.write_all(&bytes)?;
        }
        Ok(w.finish()?.into_inner())
    }

    pub fn save(&mut self, path: impl AsRef<Path>) -> Result<(), Error> {
        let bytes = self.to_bytes()?;
        let path = path.as_ref();
        let tmp = path.with_extension("docx.tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

/// Escapes text for use in hand-written XML.
pub fn esc(s: &str) -> String {
    xml::esc(s)
}
