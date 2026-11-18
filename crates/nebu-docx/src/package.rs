//! The zip archive and its parts.
//!
//! The archive is read once. Its entries become parts; a part is read and
//! decompressed only when someone asks for it, within the [`Limits`]. On
//! saving, a part that nobody changed is copied as the same compressed zip
//! entry, and only changed or new parts are compressed again.

use std::cell::Cell;
use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use std::sync::Arc;

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

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
    archive: ZipArchive<Cursor<Arc<[u8]>>>,
    size: usize,
    entries: Vec<Entry>,
    /// Lower-case name to entry: part names compare without ASCII case.
    index: HashMap<String, usize>,
    limits: Limits,
    /// Decompressed bytes read so far.
    read: Cell<u64>,
}

fn zip_error(e: zip::result::ZipError) -> Error {
    match e {
        zip::result::ZipError::UnsupportedArchive(m) => Error::Unsupported(m.to_string()),
        zip::result::ZipError::Io(e) => Error::Zip(e.to_string()),
        other => Error::Zip(other.to_string()),
    }
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
        let size = bytes.len();
        let data: Arc<[u8]> = bytes.into();
        let archive = ZipArchive::new(Cursor::new(data)).map_err(zip_error)?;
        if archive.len() > limits.max_entries {
            return Err(Error::Limit {
                what: format!("{} zip entries", archive.len()),
                field: "max_entries",
                limit: limits.max_entries as u64,
            });
        }
        if archive.has_duplicate_names() {
            return Err(Error::Zip("two zip entries have the same name".into()));
        }
        let mut entries = Vec::with_capacity(archive.len());
        let mut index = HashMap::with_capacity(archive.len());
        for i in 0..archive.len() {
            let name = match archive.name_for_index(i) {
                Some(Ok(n)) => n.into_owned(),
                Some(Err(e)) => return Err(zip_error(e)),
                None => return Err(Error::Zip(format!("entry {i} has no name"))),
            };
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
            size,
            entries,
            index,
            limits,
            read: Cell::new(0),
        })
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
        let mut archive = self.archive.clone();
        let file = archive.by_index(index).map_err(|e| match zip_error(e) {
            Error::Unsupported(m) => Error::Unsupported(format!("{name}: {m}")),
            Error::Zip(m) => Error::Zip(format!("{name}: {m}")),
            other => other,
        })?;
        if file.size() > limit {
            return Err(too_big());
        }
        let mut bytes = Vec::with_capacity(file.size().min(limit) as usize);
        file.take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| Error::Zip(format!("{name}: {e}")))?;
        if bytes.len() as u64 > limit {
            return Err(too_big());
        }
        let total = self.read.get() + bytes.len() as u64;
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
            let tree = Tree::parse_limited(bytes, &self.limits).map_err(|e| e.in_part(part))?;
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

    pub(crate) fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let mut archive = self.archive.clone();
        let mut w = ZipWriter::new(Cursor::new(Vec::with_capacity(self.size + 4096)));
        for e in &self.entries {
            let bytes = match (&e.body, e.source) {
                (Body::Source, Some(k)) => {
                    w.raw_copy_file(archive.by_index_raw(k).map_err(zip_error)?)
                        .map_err(zip_error)?;
                    continue;
                }
                (Body::Xml(t), Some(k)) if !t.is_changed() => {
                    w.raw_copy_file(archive.by_index_raw(k).map_err(zip_error)?)
                        .map_err(zip_error)?;
                    continue;
                }
                (Body::Source, None) => continue,
                (Body::Xml(t), _) => t.to_bytes(),
                (Body::Bytes(b), _) => b.clone(),
            };
            // A changed part keeps its entry's date and compression.
            let (time, method) = match e.source {
                Some(k) => {
                    let f = archive.by_index_raw(k).map_err(zip_error)?;
                    let method = match f.compression() {
                        CompressionMethod::Stored => CompressionMethod::Stored,
                        _ => CompressionMethod::Deflated,
                    };
                    (f.last_modified().unwrap_or_default(), method)
                }
                None => (DateTime::default(), CompressionMethod::Deflated),
            };
            let opts = SimpleFileOptions::default()
                .compression_method(method)
                .last_modified_time(time)
                .large_file(bytes.len() as u64 >= u32::MAX as u64);
            w.start_file(e.name.as_str(), opts).map_err(zip_error)?;
            w.write_all(&bytes).map_err(|e| Error::Zip(e.to_string()))?;
        }
        Ok(w.finish().map_err(zip_error)?.into_inner())
    }
}
