//! Saving keeps the zip archive as it was: the whole file when nothing
//! changed, and the complete record of every unchanged entry after a change.
//! The archives here are built byte by byte with the features real files
//! have: data descriptors, zip64, stored and deflated entries, extra fields,
//! comments, unusual order and bytes before the first entry.

mod support;

use nebu_docx::{Document, Error, ns};
use support::zipraw::{Layout, Spec, assert_records_kept, build, extra, records};
use support::{entries, sample};

const MAIN: &str = "word/document.xml";

fn specs() -> Vec<Spec> {
    entries(&sample())
        .into_iter()
        .map(|(n, d)| Spec::new(&n, &d))
        .collect()
}

/// Archives of the sample's parts, each stored another way.
fn variants() -> Vec<(&'static str, Vec<u8>)> {
    let mut out = vec![];
    let plain = specs();
    out.push(("plain", build(&plain, &Layout::default())));

    let mut d = specs();
    for (i, e) in d.iter_mut().enumerate() {
        e.descriptor = Some((i % 2 == 0, false));
    }
    out.push(("descriptors", build(&d, &Layout::default())));

    let mut z = specs();
    for (i, e) in z.iter_mut().enumerate() {
        e.zip64 = true;
        if i % 3 == 0 {
            e.descriptor = Some((true, true));
        }
    }
    let zl = Layout {
        zip64_end: true,
        ..Layout::default()
    };
    out.push(("zip64", build(&z, &zl)));

    let mut m = specs();
    for (i, e) in m.iter_mut().enumerate() {
        e.deflate = i % 2 == 1;
    }
    out.push(("stored-and-deflated", build(&m, &Layout::default())));

    let mut x = specs();
    for (i, e) in x.iter_mut().enumerate() {
        // Extended timestamp, and an unknown field, as other tools write them.
        e.local_extra = extra(0x5455, &[3, 1, 2, 3, 4, 5, 6, 7, 8]);
        e.central_extra = extra(0x5455, &[3, 1, 2, 3, 4]);
        if i % 2 == 0 {
            e.central_extra.extend(extra(0xCAFE, b"opaque"));
        }
        e.comment = format!("entry {i}").into_bytes();
    }
    let xl = Layout {
        comment: b"archive comment".to_vec(),
        ..Layout::default()
    };
    out.push(("extras-and-comments", build(&x, &xl)));

    let mut o = specs();
    o.reverse();
    let n = o.len();
    let ol = Layout {
        local_order: (0..n).map(|i| (i * 7 + 3) % n).collect(),
        ..Layout::default()
    };
    out.push(("unusual-order", build(&o, &ol)));

    let pl = Layout {
        prefix: b"#!/bin/sh\nexit 0\n".repeat(40),
        ..Layout::default()
    };
    out.push(("prefix", build(&specs(), &pl)));

    let mut u = specs();
    u.push(Spec {
        utf8: true,
        ..Spec::new("customXml/übersicht.xml", b"<a/>")
    });
    out.push(("utf8-name", build(&u, &Layout::default())));

    let mut all = specs();
    for (i, e) in all.iter_mut().enumerate() {
        e.zip64 = i % 2 == 0;
        e.descriptor = Some((i % 3 != 0, e.zip64));
        e.deflate = i % 4 != 1;
        e.local_extra = extra(0x7875, &[1, 4, 0, 0, 0, 0, 4, 0, 0, 0, 0]);
        e.comment = vec![b'c'; i];
    }
    all.reverse();
    let n = all.len();
    let al = Layout {
        prefix: vec![0xAB; 333],
        local_order: (0..n).rev().collect(),
        comment: b"all of it".to_vec(),
        zip64_end: true,
    };
    out.push(("everything", build(&all, &al)));
    out
}

fn add_paragraph(doc: &mut Document) {
    let t = doc.xml(MAIN).unwrap();
    let body = t
        .child(t.root_element(), Some(ns::W), "body")
        .unwrap()
        .unwrap();
    let p = t
        .parse_fragment("<w:p><w:r><w:t>Added</w:t></w:r></w:p>", &[("w", ns::W)])
        .unwrap();
    t.insert(body, 0, p[0]).unwrap();
}

#[test]
fn an_unchanged_save_is_the_input_file() {
    for (name, bytes) in variants().into_iter().chain(support::corpus()) {
        let mut doc = Document::from_bytes(bytes.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(doc.to_bytes().unwrap() == bytes, "{name}");
        // Parsing parts changes nothing either.
        let parts: Vec<String> = doc.parts().map(String::from).collect();
        for p in parts
            .iter()
            .filter(|p| p.ends_with(".xml") || p.ends_with(".rels"))
        {
            doc.xml(p).unwrap();
        }
        assert!(doc.to_bytes().unwrap() == bytes, "{name}, parsed");
    }
}

#[test]
fn a_change_keeps_every_other_record() {
    for (name, bytes) in variants() {
        let mut doc = Document::from_bytes(bytes.clone()).unwrap();
        add_paragraph(&mut doc);
        let out = doc.to_bytes().unwrap();
        assert_records_kept(&bytes, &out, &[MAIN]);
        // The changed entry is written fresh and reads back.
        let changed = records(&out)
            .entries
            .into_iter()
            .find(|e| e.name == MAIN)
            .unwrap();
        assert_eq!(&changed.local[..4], b"PK\x03\x04", "{name}");
        let mut again = Document::from_bytes(out.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
        let text = String::from_utf8(again.read_part(MAIN).unwrap()).unwrap();
        assert!(
            text.contains("<w:p><w:r><w:t>Added</w:t></w:r></w:p>"),
            "{name}"
        );
        for (part, data) in entries(&bytes) {
            if part != MAIN && !part.ends_with('/') {
                assert_eq!(again.read_part(&part).unwrap(), data, "{name} {part}");
            }
        }
        again.xml(MAIN).unwrap();
        // A second save of the saved file is that file.
        assert!(again.to_bytes().unwrap() == out, "{name}");
    }
}

#[test]
fn a_new_part_is_added_after_the_others() {
    for (name, bytes) in variants() {
        let mut doc = Document::from_bytes(bytes.clone()).unwrap();
        doc.add_part("word/media/image9.png", "image/png", vec![7; 5000])
            .unwrap();
        let out = doc.to_bytes().unwrap();
        assert_records_kept(&bytes, &out, &["[Content_Types].xml"]);
        let r = records(&out);
        assert_eq!(
            r.entries.last().unwrap().name,
            "word/media/image9.png",
            "{name}"
        );
        assert_eq!(r.local_order.last().unwrap(), "word/media/image9.png");
        let again = Document::from_bytes(out).unwrap();
        assert_eq!(
            again.read_part("word/media/image9.png").unwrap(),
            vec![7; 5000]
        );
    }
}

fn refused(bytes: Vec<u8>) -> Error {
    Document::from_bytes(bytes).expect_err("the file is refused")
}

#[test]
fn what_cannot_be_carried_through_is_refused_at_open() {
    let good = build(&specs(), &Layout::default());

    // An encrypted entry: flag bit 0 in both headers.
    let mut enc = good.clone();
    let r = records(&good);
    let first = r.entries[0].offset as usize;
    enc[first + 6] |= 1;
    let cd = good.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    enc[cd + 8] |= 1;
    let e = refused(enc);
    assert!(
        matches!(&e, Error::Unsupported(m) if m.contains("encrypted")),
        "{e}"
    );

    // An archive on several disks.
    let mut disks = good.clone();
    let end = good.len() - 22;
    disks[end + 4] = 1;
    let e = refused(disks);
    assert!(
        matches!(&e, Error::Unsupported(m) if m.contains("disks")),
        "{e}"
    );

    // Bytes after the end record.
    let mut trailing = good.clone();
    trailing.extend_from_slice(b"junk");
    assert!(matches!(refused(trailing), Error::Zip(_)));

    // Bytes between two entries.
    let mut s = specs();
    s[1].local_extra = vec![];
    let mut gap = build(&s, &Layout::default());
    let at = records(&gap).entries[1].offset as usize;
    gap.splice(at..at, *b"GAP!");
    // Move every later offset and the directory along.
    let fixed = shift_after(&gap, at as u32, 4);
    let e = refused(fixed);
    assert!(
        matches!(&e, Error::Unsupported(m) if m.contains("between")),
        "{e}"
    );

    // A data descriptor that does not match its directory record.
    let mut d = specs();
    d[0].descriptor = Some((true, false));
    let mut bad = build(&d, &Layout::default());
    let r = records(&bad);
    let desc = r.entries[0].offset as usize + r.entries[0].local.len() - 12;
    bad[desc + 4] ^= 0xFF;
    assert!(matches!(refused(bad), Error::Zip(m) if m.contains("descriptor")));

    // A local header that names another entry.
    let mut other = good.clone();
    let name_at = first + 30;
    other[name_at] ^= 0x20;
    assert!(matches!(refused(other), Error::Zip(m) if m.contains("names another")));

    // Two directory records for the same local record.
    let mut twice = good.clone();
    let r = records(&good);
    let second_cd = cd + r.entries[0].central.len();
    let o = (r.entries[0].offset as u32).to_le_bytes();
    twice[second_cd + 42..second_cd + 46].copy_from_slice(&o);
    assert!(matches!(refused(twice), Error::Zip(_)));
}

/// Adds `by` to every local header offset at or after `at` and to the
/// central directory offset, after `by` bytes were inserted at `at`.
fn shift_after(b: &[u8], at: u32, by: u32) -> Vec<u8> {
    let mut b = b.to_vec();
    let end = b.len() - 22;
    let cd = u32::from_le_bytes(b[end + 16..end + 20].try_into().unwrap()) + by;
    b[end + 16..end + 20].copy_from_slice(&cd.to_le_bytes());
    let mut i = cd as usize;
    while b[i..i + 4] == *b"PK\x01\x02" {
        let o = u32::from_le_bytes(b[i + 42..i + 46].try_into().unwrap());
        if o >= at {
            b[i + 42..i + 46].copy_from_slice(&(o + by).to_le_bytes());
        }
        let len = 46
            + u16::from_le_bytes([b[i + 28], b[i + 29]]) as usize
            + u16::from_le_bytes([b[i + 30], b[i + 31]]) as usize
            + u16::from_le_bytes([b[i + 32], b[i + 33]]) as usize;
        i += len;
    }
    b
}

/// The sample's parts and one more entry, `custom.bin`, stored as `spec`
/// says, with the offset of its local header.
fn with_custom(spec: Spec, layout: &Layout) -> (Vec<u8>, usize) {
    let mut s = specs();
    s.push(spec);
    let b = build(&s, layout);
    let at = records(&b)
        .entries
        .iter()
        .find(|e| e.name == "custom.bin")
        .unwrap()
        .offset as usize;
    (b, at)
}

fn custom(data: &[u8]) -> Spec {
    Spec {
        deflate: false,
        ..Spec::new("custom.bin", data)
    }
}

/// The archive opens, saves unchanged as itself, and keeps `custom.bin`'s
/// records after another part changes.
fn carried_through(bytes: Vec<u8>) {
    let mut doc = Document::from_bytes(bytes.clone()).unwrap_or_else(|e| panic!("{e}"));
    assert!(doc.to_bytes().unwrap() == bytes);
    assert_eq!(doc.read_part("custom.bin").unwrap(), entry_data(&bytes));
    add_paragraph(&mut doc);
    assert_records_kept(&bytes, &doc.to_bytes().unwrap(), &[MAIN]);
}

fn entry_data(bytes: &[u8]) -> Vec<u8> {
    support::entry(bytes, "custom.bin")
}

/// The error names the entry and the field.
fn refused_for(bytes: Vec<u8>, field: &str) {
    let e = refused(bytes);
    assert!(
        matches!(&e, Error::Zip(m) if m.contains("custom.bin") && m.contains(field)),
        "{field}: {e}"
    );
}

#[test]
fn a_local_header_that_disagrees_with_its_directory_record_is_refused() {
    let (good, at) = with_custom(custom(b"some data"), &Layout::default());
    carried_through(good.clone());
    let patch = |off: usize, v: &[u8]| {
        let mut b = good.clone();
        b[at + off..at + off + v.len()].copy_from_slice(v);
        b
    };
    // CRC, compressed size and size: what the reviewer's probe did.
    refused_for(patch(14, &123u32.to_le_bytes()), "CRC");
    refused_for(patch(18, &123u32.to_le_bytes()), "compressed size");
    refused_for(patch(22, &123u32.to_le_bytes()), "uncompressed size");
    // Without a data descriptor, zero is not allowed either.
    refused_for(patch(14, &[0; 4]), "CRC");
    refused_for(patch(8, &8u16.to_le_bytes()), "compression method");
    let flags = u16::from_le_bytes([good[at + 6], good[at + 7]]);
    refused_for(patch(6, &(flags | 1 << 11).to_le_bytes()), "UTF-8 flag");
    refused_for(
        patch(6, &(flags | 1 << 3).to_le_bytes()),
        "data descriptor flag",
    );
    let e = refused(patch(6, &(flags | 1).to_le_bytes()));
    assert!(
        matches!(&e, Error::Zip(m) if m.contains("encryption")),
        "{e}"
    );
}

#[test]
fn a_local_zip64_field_is_read_before_the_comparison() {
    let spec = Spec {
        zip64: true,
        ..custom(b"some data")
    };
    let (good, at) = with_custom(spec, &Layout::default());
    carried_through(good.clone());
    // The local zip64 field holds the size, then the compressed size.
    let z = at + 30 + "custom.bin".len() + 4;
    for (off, field) in [(0, "uncompressed size"), (8, "compressed size")] {
        let mut b = good.clone();
        b[z + off..z + off + 8].copy_from_slice(&(5u64 << 30).to_le_bytes());
        refused_for(b, field);
    }
}

#[test]
fn with_a_data_descriptor_the_local_header_may_leave_the_values_out() {
    for zip64 in [false, true] {
        let spec = Spec {
            descriptor: Some((true, zip64)),
            zip64,
            ..custom(b"some data")
        };
        let (good, at) = with_custom(spec, &Layout::default());
        // The builder writes zeros in the local header.
        assert_eq!(good[at + 14..at + 26], [0; 12]);
        carried_through(good.clone());
        // Values that are given must be right.
        let mut b = good.clone();
        b[at + 14..at + 18].copy_from_slice(&123u32.to_le_bytes());
        refused_for(b, "CRC");
        let mut b = good.clone();
        b[at + 22..at + 26].copy_from_slice(&123u32.to_le_bytes());
        refused_for(b, "uncompressed size");
        // The right values are allowed too.
        let mut b = good.clone();
        b[at + 14..at + 18].copy_from_slice(&crc32fast::hash(b"some data").to_le_bytes());
        b[at + 18..at + 22].copy_from_slice(&9u32.to_le_bytes());
        b[at + 22..at + 26].copy_from_slice(&9u32.to_le_bytes());
        carried_through(b);
    }
}

/// Four bytes whose CRC-32 is `crc`. The CRC of a 4-byte message is an
/// affine function of its bits, so a basis of single-bit messages gives it.
fn with_crc(crc: u32) -> [u8; 4] {
    let base = crc32fast::hash(&[0; 4]);
    let mut basis: [Option<(u32, u32)>; 32] = [None; 32];
    for i in 0..32 {
        let (mut v, mut m) = (
            crc32fast::hash(&(1u32 << i).to_le_bytes()) ^ base,
            1u32 << i,
        );
        while v != 0 {
            let k = 31 - v.leading_zeros() as usize;
            match basis[k] {
                Some((bv, bm)) => {
                    v ^= bv;
                    m ^= bm;
                }
                None => {
                    basis[k] = Some((v, m));
                    break;
                }
            }
        }
    }
    let (mut v, mut m) = (crc ^ base, 0);
    while v != 0 {
        let (bv, bm) = basis[31 - v.leading_zeros() as usize].unwrap();
        v ^= bv;
        m ^= bm;
    }
    let out = m.to_le_bytes();
    assert_eq!(crc32fast::hash(&out), crc);
    out
}

#[test]
fn every_data_descriptor_layout_is_read() {
    let collision = with_crc(0x0807_4b50);
    for data in [&b"some data"[..], &collision[..], b""] {
        for signed in [false, true] {
            for wide in [false, true] {
                let spec = Spec {
                    descriptor: Some((signed, wide)),
                    zip64: wide,
                    ..custom(data)
                };
                let (b, _) = with_custom(spec, &Layout::default());
                carried_through(b);
            }
        }
    }
}

#[test]
fn a_descriptor_that_fits_two_layouts_is_placed_by_the_next_record() {
    // An empty entry's wide descriptor also reads as a narrow one: CRC 0,
    // sizes 0. The next record starts after the wide one, so that is it.
    for signed in [false, true] {
        let spec = Spec {
            descriptor: Some((signed, true)),
            ..custom(b"")
        };
        let (b, at) = with_custom(spec, &Layout::default());
        carried_through(b.clone());
        // With four more bytes after it, neither layout ends at the next
        // record, and the archive is ambiguous.
        let r = records(&b);
        let me = r.entries.iter().find(|e| e.name == "custom.bin").unwrap();
        let after = at + me.local.len();
        let mut gap = b.clone();
        gap.splice(after..after, [0u8; 4]);
        let e = refused(shift_after(&gap, after as u32, 4));
        assert!(
            matches!(&e, Error::Zip(m) if m.contains("custom.bin") && m.contains("ambiguous")),
            "{e}"
        );
    }
}

#[test]
fn a_descriptor_that_disagrees_names_the_field() {
    for (signed, wide) in [(false, false), (true, false), (false, true), (true, true)] {
        let spec = Spec {
            descriptor: Some((signed, wide)),
            zip64: wide,
            ..custom(b"some data")
        };
        let (good, at) = with_custom(spec, &Layout::default());
        let d = at
            + records(&good)
                .entries
                .iter()
                .find(|e| e.name == "custom.bin")
                .unwrap()
                .local
                .len()
            - if wide { 20 } else { 12 };
        let w = if wide { 8 } else { 4 };
        for (off, field) in [
            (0, "CRC"),
            (4, "compressed size"),
            (4 + w, "uncompressed size"),
        ] {
            let mut b = good.clone();
            b[d + off] ^= 0x40;
            refused_for(b, field);
        }
    }
}

#[test]
fn an_archive_comment_with_an_end_record_signature_is_refused() {
    let comment = [b"note ".as_slice(), b"PK\x05\x06", &[0; 18]].concat();
    let b = build(
        &specs(),
        &Layout {
            comment,
            ..Layout::default()
        },
    );
    let e = refused(b);
    assert!(
        matches!(&e, Error::Zip(m) if m.contains("archive comment")),
        "{e}"
    );
}
