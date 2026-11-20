//! The zip archive and its parts.
//!
//! The archive is read once. Its entries become parts; a part is read and
//! decompressed only when someone asks for it, within the [`Limits`]. A
//! document saved with no change is the file it was read from, byte for
//! byte. After a change, every entry that did not change keeps its complete
//! record, and only changed and new parts are compressed again (see
//! [`archive`](crate::archive)).

use std::cell::Cell;
use std::collections::HashMap;

use crate::archive::Archive;
use crate::xml::Tree;
use crate::{Error, Limits};

enum Body {
    /// Unchanged: the source entry.
    Source,
    Bytes(Vec<u8>),
    Xml(Box<Tree>),
}

struct Entry {
    name: String,
    /// The index of the source entry, `None` for an added part.
    source: Option<usize>,
    body: Body,
}

pub(crate) struct Package {
    archive: Archive,
    entries: Vec<Entry>,
    /// Lower-case name to entry: part names compare without ASCII case.
    index: HashMap<String, usize>,
    limits: Limits,
    /// Decompressed bytes read so far.
    read: Cell<u64>,
}

/// The parts of a package as they were, to put back after a failed edit.
pub(crate) struct Snapshot {
    entries: Vec<Entry>,
    index: HashMap<String, usize>,
    read: u64,
}

/// Checks a name for use as a part: relative, `/`-separated, no empty, `.`
/// or `..` segments, no backslash or control characters. A trailing `/`
/// marks a directory entry, which is kept but is not a part.
pub(crate) fn check_part_name(name: &str) -> Result<(), Error> {
    let bad = |reason| {
        Err(Error::PartName {
            name: name.to_string(),
            reason,
        })
    };
    if name.is_empty() {
        return bad("it is empty");
    }
    if name.starts_with('/') {
        return bad("it starts with '/'");
    }
    if name.contains('\\') {
        return bad("it contains a backslash");
    }
    if name.chars().any(char::is_control) {
        return bad("it contains a control character");
    }
    let path = name.strip_suffix('/').unwrap_or(name);
    for seg in path.split('/') {
        if seg.is_empty() {
            return bad("it has an empty segment");
        }
        if seg == "." || seg == ".." {
            return bad("it has a '.' or '..' segment");
        }
        if seg.ends_with('.') {
            return bad("a segment ends with '.'");
        }
    }
    Ok(())
}

impl Package {
    pub(crate) fn from_bytes(bytes: Vec<u8>, limits: Limits) -> Result<Package, Error> {
        if bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]) {
            return Err(Error::Unsupported(
                "the file is a compound file: a password-protected document or a legacy .doc; remove the password or save it as .docx in Word"
                    .into(),
            ));
        }
        let archive = Archive::read(bytes, limits.max_entries)?;
        let mut entries = Vec::with_capacity(archive.entries.len());
        let mut index = HashMap::with_capacity(archive.entries.len());
        for (i, e) in archive.entries.iter().enumerate() {
            let name = e.name.clone();
            check_part_name(&name)?;
            if index
                .insert(name.to_ascii_lowercase(), entries.len())
                .is_some()
            {
                return Err(Error::PartName {
                    name,
                    reason: "another entry has the same name, ignoring case",
                });
            }
            entries.push(Entry {
                name,
                source: Some(i),
                body: Body::Source,
            });
        }
        Ok(Package {
            archive,
            entries,
            index,
            limits,
            read: Cell::new(0),
        })
    }

    /// The parts as they are now. Trees are copied with their identity,
    /// so handles into them stay good after [`Package::restore`].
    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot {
            entries: self
                .entries
                .iter()
                .map(|e| Entry {
                    name: e.name.clone(),
                    source: e.source,
                    body: match &e.body {
                        Body::Source => Body::Source,
                        Body::Bytes(b) => Body::Bytes(b.clone()),
                        Body::Xml(t) => Body::Xml(Box::new(t.snapshot())),
                    },
                })
                .collect(),
            index: self.index.clone(),
            read: self.read.get(),
        }
    }

    /// Puts back the parts of a snapshot.
    pub(crate) fn restore(&mut self, s: Snapshot) {
        self.entries = s.entries;
        self.index = s.index;
        self.read.set(s.read);
    }

    /// What the parsed trees and the list of parts are now. Equal stamps
    /// mean that nothing was edited or added in between.
    pub(crate) fn stamp(&self) -> Vec<(u64, u64)> {
        let mut out = vec![(self.entries.len() as u64, 0)];
        for e in &self.entries {
            if let Body::Xml(t) = &e.body {
                out.push(t.stamp());
            }
        }
        out
    }

    /// The limits the package was opened with.
    pub(crate) fn limits(&self) -> &Limits {
        &self.limits
    }

    /// The bytes the archive was read from.
    pub(crate) fn source_bytes(&self) -> &[u8] {
        &self.archive.bytes
    }

    /// The names of all parts in archive order, directory entries excluded.
    pub(crate) fn names(&self) -> impl Iterator<Item = &str> {
        self.entries
            .iter()
            .map(|e| e.name.as_str())
            .filter(|n| !n.ends_with('/'))
    }

    /// The part's name as the archive spells it.
    pub(crate) fn find(&self, name: &str) -> Option<&str> {
        let &i = self.index.get(&name.to_ascii_lowercase())?;
        let n = &self.entries[i].name;
        (!n.ends_with('/')).then_some(n.as_str())
    }

    fn entry(&self, name: &str) -> Result<usize, Error> {
        self.index
            .get(&name.to_ascii_lowercase())
            .copied()
            .filter(|&i| !self.entries[i].name.ends_with('/'))
            .ok_or_else(|| Error::MissingPart(name.to_string()))
    }

    /// The bytes of a part as they would be saved now.
    pub(crate) fn read(&self, name: &str) -> Result<Vec<u8>, Error> {
        let i = self.entry(name)?;
        let e = &self.entries[i];
        match (&e.body, e.source) {
            (Body::Bytes(b), _) => Ok(b.clone()),
            (Body::Xml(t), _) => Ok(t.to_bytes()),
            (Body::Source, Some(k)) => self.decompress(k, &e.name),
            (Body::Source, None) => Err(Error::MissingPart(name.to_string())),
        }
    }

    fn decompress(&self, index: usize, name: &str) -> Result<Vec<u8>, Error> {
        let limit = self.limits.max_part_size;
        let too_big = || Error::Limit {
            what: format!("a part {name} larger than {limit} bytes"),
            field: "max_part_size",
            limit,
        };
        let e = &self.archive.entries[index];
        if e.size > limit {
            return Err(too_big());
        }
        let Some(bytes) = self.archive.decompress(e, limit)? else {
            return Err(too_big());
        };
        let total = self.read.get().saturating_add(bytes.len() as u64);
        if total > self.limits.max_total_size {
            return Err(Error::Limit {
                what: format!("more than {} bytes of parts", self.limits.max_total_size),
                field: "max_total_size",
                limit: self.limits.max_total_size,
            });
        }
        self.read.set(total);
        Ok(bytes)
    }

    /// The parsed tree of a part, parsed on first use.
    pub(crate) fn xml(&mut self, name: &str) -> Result<&mut Tree, Error> {
        let i = self.entry(name)?;
        if !matches!(self.entries[i].body, Body::Xml(_)) {
            let bytes = self.read(name)?;
            let part = &self.entries[i].name;
            let tree = Tree::parse_with_limits(bytes, &self.limits).map_err(|e| e.in_part(part))?;
            self.entries[i].body = Body::Xml(Box::new(tree));
        }
        match &mut self.entries[i].body {
            Body::Xml(t) => Ok(t),
            _ => unreachable!("parsed above"),
        }
    }

    /// The tree of a part that has been parsed already.
    pub(crate) fn parsed(&self, name: &str) -> Option<&Tree> {
        let i = self.entry(name).ok()?;
        match &self.entries[i].body {
            Body::Xml(t) => Some(t),
            _ => None,
        }
    }

    /// A copy of one parsed part's tree, with its identity, to put back
    /// if an edit of that part fails half-way. Parses the part first,
    /// like [`Package::xml`].
    pub(crate) fn snapshot_part(&mut self, name: &str) -> Result<Tree, Error> {
        Ok(self.xml(name)?.snapshot())
    }

    /// Puts back a part tree taken by [`Package::snapshot_part`].
    pub(crate) fn restore_part(&mut self, name: &str, tree: Tree) {
        if let Ok(i) = self.entry(name) {
            self.entries[i].body = Body::Xml(Box::new(tree));
        }
    }

    /// Adds a new part. The caller has checked the name and that it is free.
    pub(crate) fn add(&mut self, name: &str, bytes: Vec<u8>) {
        self.index
            .insert(name.to_ascii_lowercase(), self.entries.len());
        self.entries.push(Entry {
            name: name.to_string(),
            source: None,
            body: Body::Bytes(bytes),
        });
    }

    /// Replaces the bytes of an existing part. The caller has checked
    /// the name.
    pub(crate) fn set_bytes(&mut self, name: &str, bytes: Vec<u8>) {
        let i = self.index[&name.to_ascii_lowercase()];
        self.entries[i].body = Body::Bytes(bytes);
    }

    /// The archive as it would be saved now. With no change, the bytes it
    /// was read from.
    pub(crate) fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let mut parts: Vec<Option<Vec<u8>>> = vec![None; self.archive.entries.len()];
        let mut added: Vec<(&str, Vec<u8>)> = vec![];
        for e in &self.entries {
            let bytes = match &e.body {
                Body::Source => continue,
                Body::Xml(t) if !t.is_changed() && e.source.is_some() => continue,
                Body::Xml(t) => t.to_bytes(),
                Body::Bytes(b) => b.clone(),
            };
            match e.source {
                Some(k) => parts[k] = Some(bytes),
                None => added.push((&e.name, bytes)),
            }
        }
        if parts.iter().all(Option::is_none) && added.is_empty() {
            return Ok(self.archive.bytes.clone());
        }
        let parts: Vec<Option<&[u8]>> = parts.iter().map(|p| p.as_deref()).collect();
        let added: Vec<(&str, &[u8])> = added.iter().map(|(n, b)| (*n, b.as_slice())).collect();
        self.archive.write(&parts, &added)
    }
}
