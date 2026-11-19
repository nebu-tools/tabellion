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
    next_revision: u64,
    used_revision: HashSet<u64>,
}

impl Ids {
    /// Collects the ids of these parts, which must be parsed.
    pub fn collect(doc: &Document, parts: &[String], seed: u64) -> Result<Ids, Error> {
        let mut para = HashSet::new();
        let mut bookmark = 0u64;
        let mut drawing = 0u64;
        let mut revision = 0u64;
        let mut used_revision = HashSet::new();
        for p in parts {
            let Some(t) = doc.parsed(p) else { continue };
            scan(
                t,
                &mut para,
                &mut bookmark,
                &mut drawing,
                &mut revision,
                &mut used_revision,
            )?;
        }
        Ok(Ids {
            para,
            state: seed | 1,
            next_bookmark: bookmark,
            next_drawing: drawing,
            next_revision: revision,
            used_revision,
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

    /// A revision id (`w:ins/@w:id`) no part uses. Wraps past `u64::MAX`
    /// to an unused id instead of panicking or reusing one.
    pub fn revision(&mut self) -> u64 {
        loop {
            self.next_revision = self.next_revision.wrapping_add(1);
            if self.used_revision.insert(self.next_revision) {
                return self.next_revision;
            }
            // The used set holds at most as many ids as the document's
            // revision records, so an unused id is always found quickly;
            // a document cannot hold 2^64 records.
        }
    }
}

fn scan(
    t: &Tree,
    para: &mut HashSet<u32>,
    bookmark: &mut u64,
    drawing: &mut u64,
    revision: &mut u64,
    used_revision: &mut HashSet<u64>,
) -> Result<(), Error> {
    for n in t.descendants(t.root_element())? {
        let Some(name) = t.name(n)? else { continue };
        if name.namespace == Some(W) && name.local == "bookmarkStart" {
            if let Some(v) = t.attribute(n, Some(W), "id")?
                && let Ok(v) = v.trim().parse::<u64>()
            {
                *bookmark = (*bookmark).max(v);
            }
            continue;
        }
        if name.namespace == Some(WP) && name.local == "docPr" {
            if let Some(v) = t.attribute(n, None, "id")?
                && let Ok(v) = v.trim().parse::<u64>()
            {
                *drawing = (*drawing).max(v);
            }
            continue;
        }
        // Any revision record with w:id: content insertions/deletions,
        // moves, cell marks, custom XML revision range markers, Word's
        // w14 conflict records, and every property-change record
        // (pPrChange, rPrChange, tblPrChange, trPrChange, tcPrChange,
        // sectPrChange, tblPrExChange, tblGridChange, numberingChange, and
        // whatever future *Change the schema adds). Matching the suffix
        // instead of a fixed list keeps a new change kind from reusing an
        // id; custom XML markers share no suffix, so they match by prefix.
        // Conflict records live in the w14 namespace but carry a plain
        // w:id, so `<w14:conflictIns w:id="1">` would otherwise collide
        // with the first generated `<w:ins w:id="1">`.
        let is_revision = (name.namespace == Some(W)
            && (matches!(
                name.local,
                "ins"
                    | "del"
                    | "moveFrom"
                    | "moveTo"
                    | "cellIns"
                    | "cellDel"
                    | "cellMerge"
                    | "moveFromRangeStart"
                    | "moveFromRangeEnd"
                    | "moveToRangeStart"
                    | "moveToRangeEnd"
            ) || name.local.ends_with("Change")
                || name.local.starts_with("customXml")))
            || (name.namespace == Some(W14) && name.local.to_lowercase().contains("conflict"));
        if is_revision {
            if let Some(v) = t.attribute(n, Some(W), "id")?
                && let Ok(v) = v.trim().parse::<u64>()
            {
                *revision = (*revision).max(v);
                used_revision.insert(v);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn scanned_max(name: &str, id: u64) -> u64 {
        let xml = format!(
            "<w:root xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:{name} w:id=\"{id}\"/></w:root>",
        );
        let t = Tree::parse(xml.into_bytes()).unwrap();
        let mut para = HashSet::new();
        let mut bookmark = 0;
        let mut drawing = 0;
        let mut revision = 0;
        let mut used = HashSet::new();
        scan(
            &t,
            &mut para,
            &mut bookmark,
            &mut drawing,
            &mut revision,
            &mut used,
        )
        .unwrap();
        assert!(
            used.contains(&id),
            "{name} with id {id} was not recorded as used"
        );
        revision
    }

    fn scanned_max_ns(prefix: &str, ns: &str, name: &str, id: u64) -> u64 {
        let xml = format!(
            "<root xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:w14=\"http://schemas.microsoft.com/office/word/2010/wordml\"><{prefix}:{name} w:id=\"{id}\"/></root>",
        );
        let _ = ns;
        let t = Tree::parse(xml.into_bytes()).unwrap();
        let mut para = HashSet::new();
        let mut bookmark = 0;
        let mut drawing = 0;
        let mut revision = 0;
        let mut used = HashSet::new();
        scan(
            &t,
            &mut para,
            &mut bookmark,
            &mut drawing,
            &mut revision,
            &mut used,
        )
        .unwrap();
        assert!(
            used.contains(&id),
            "{prefix}:{name} with id {id} was not recorded as used"
        );
        revision
    }

    #[test]
    fn revision_scan_covers_every_change_kind() {
        // Each marker on its own, so a miss cannot hide behind a larger
        // maximum from another marker.
        let cases = [
            ("ins", 10),
            ("del", 11),
            ("moveFrom", 12),
            ("moveTo", 13),
            ("cellIns", 14),
            ("cellDel", 15),
            ("cellMerge", 16),
            ("moveFromRangeStart", 17),
            ("moveToRangeEnd", 18),
            ("pPrChange", 900),
            ("rPrChange", 901),
            ("tblPrChange", 902),
            ("trPrChange", 903),
            ("tcPrChange", 904),
            ("sectPrChange", 905),
            ("tblPrExChange", 906),
            ("tblGridChange", 907),
            ("numberingChange", 908),
            ("customChange", 909),
            ("customXmlInsRangeStart", 910),
            ("customXmlInsRangeEnd", 911),
            ("customXmlDelRangeStart", 912),
            ("customXmlDelRangeEnd", 913),
            ("customXmlMoveFromRangeStart", 914),
            ("customXmlMoveToRangeEnd", 915),
        ];
        for (name, id) in cases {
            assert_eq!(scanned_max(name, id), id, "scan missed {name} with id {id}");
        }
        // Word's w14 conflict records share the w:id space but live in
        // another namespace; each on its own so a miss cannot hide.
        for (name, id) in [
            ("conflictIns", 916),
            ("conflictDel", 917),
            ("customXmlConflictInsRangeStart", 918),
            ("customXmlConflictInsRangeEnd", 919),
            ("customXmlConflictDelRangeStart", 920),
            ("customXmlConflictDelRangeEnd", 921),
        ] {
            assert_eq!(
                scanned_max_ns(
                    "w14",
                    "http://schemas.microsoft.com/office/word/2010/wordml",
                    name,
                    id
                ),
                id,
                "scan missed w14:{name} with id {id}"
            );
        }
    }

    #[test]
    fn revision_scan_trims_whitespace_padded_ids() {
        // `w:id=" 42 "` is a valid XML Schema integer; without trimming
        // the scan would miss it and later allocate 42 again.
        let xml = "<w:root xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:ins w:id=\" 42 \"/></w:root>";
        let t = Tree::parse(xml.as_bytes().to_vec()).unwrap();
        let mut para = HashSet::new();
        let mut bookmark = 0;
        let mut drawing = 0;
        let mut revision = 0;
        let mut used = HashSet::new();
        scan(
            &t,
            &mut para,
            &mut bookmark,
            &mut drawing,
            &mut revision,
            &mut used,
        )
        .unwrap();
        assert_eq!(revision, 42);
        assert!(used.contains(&42));
    }

    #[test]
    fn revision_allocation_wraps_past_u64_max_without_reuse() {
        let mut ids = Ids {
            para: HashSet::new(),
            state: 1,
            next_bookmark: 0,
            next_drawing: 0,
            next_revision: u64::MAX,
            used_revision: [u64::MAX, 0, 1].into_iter().collect(),
        };
        // Would panic on overflow with `+= 1` and would reuse 0 or 1 with
        // a wrapping counter that does not track use.
        assert_eq!(ids.revision(), 2);
        assert!(ids.used_revision.contains(&2));
    }
}
