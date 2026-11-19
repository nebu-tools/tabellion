//! The ids the writer creates: paragraph ids, bookmark ids and drawing ids.
//!
//! They never collide with ids the document has: one pass over the story
//! parts collects those first. They are deterministic: paragraph ids come
//! from a generator seeded with the SHA-256 of the file the document was
//! read from and the number of insertions made so far, so the same call on
//! the same file gives the same bytes.

use std::collections::HashSet;

use crate::xml::Tree;
use crate::{Document, Error, ns};

const W: &str = ns::W;
const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
const WP: &str = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing";

pub(crate) struct Ids {
    para: HashSet<u32>,
    state: u64,
    next_bookmark: u64,
    next_drawing: u64,
}

impl Ids {
    /// Collects the ids of these parts, which must be parsed.
    pub fn collect(doc: &Document, parts: &[String], seed: u64) -> Result<Ids, Error> {
        let mut para = HashSet::new();
        let mut bookmark = 0u64;
        let mut drawing = 0u64;
        for p in parts {
            let Some(t) = doc.parsed(p) else { continue };
            scan(t, &mut para, &mut bookmark, &mut drawing)?;
        }
        Ok(Ids {
            para,
            state: seed | 1,
            next_bookmark: bookmark,
            next_drawing: drawing,
        })
    }

    /// A paragraph id no part uses, below `0x80000000`.
    pub fn para(&mut self) -> u32 {
        loop {
            // xorshift64*
            let mut x = self.state;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.state = x;
            let v = (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as u32 & 0x7FFF_FFFF;
            if v != 0 && self.para.insert(v) {
                return v;
            }
        }
    }

    /// A bookmark id no part uses.
    pub fn bookmark(&mut self) -> u64 {
        self.next_bookmark += 1;
        self.next_bookmark
    }

    /// A drawing id (`wp:docPr/@id`) no part uses.
    pub fn drawing(&mut self) -> u64 {
        self.next_drawing += 1;
        self.next_drawing
    }
}

fn scan(
    t: &Tree,
    para: &mut HashSet<u32>,
    bookmark: &mut u64,
    drawing: &mut u64,
) -> Result<(), Error> {
    for n in t.descendants(t.root_element())? {
        let Some(name) = t.name(n)? else { continue };
        if name.namespace == Some(W) && name.local == "bookmarkStart" {
            if let Some(v) = t.attribute(n, Some(W), "id")?
                && let Ok(v) = v.parse::<u64>()
            {
                *bookmark = (*bookmark).max(v);
            }
            continue;
        }
        if name.namespace == Some(WP) && name.local == "docPr" {
            if let Some(v) = t.attribute(n, None, "id")?
                && let Ok(v) = v.parse::<u64>()
            {
                *drawing = (*drawing).max(v);
            }
            continue;
        }
        if name.namespace == Some(W)
            && matches!(name.local, "p" | "tr")
            && let Some(v) = t.attribute(n, Some(W14), "paraId")?
            && let Ok(v) = u32::from_str_radix(&v, 16)
        {
            para.insert(v);
        }
    }
    Ok(())
}

/// A seed from the source file's hash and the number of insertions.
pub(crate) fn seed(doc: &Document) -> u64 {
    let sha = doc.sha256();
    let mut s = u64::from_str_radix(&sha[..16], 16).unwrap_or(0x9E37_79B9_7F4A_7C15);
    s ^= doc.writes.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    s
}

/// `stem.ext`, or `stem2.ext` and so on if the document has a part of that
/// name.
pub(crate) fn free_part(doc: &Document, stem: &str, ext: &str) -> String {
    let first = format!("{stem}.{ext}");
    if doc.part_name(&first).is_none() {
        return first;
    }
    let mut k = 2;
    loop {
        let n = format!("{stem}{k}.{ext}");
        if doc.part_name(&n).is_none() {
            return n;
        }
        k += 1;
    }
}
