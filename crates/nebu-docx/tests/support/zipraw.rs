//! Zip archives built byte by byte, with the features the engine must carry
//! through, and a reader that splits an archive into its records.
//!
//! The reader shares no code with the engine. It takes each local record to
//! run from its offset to the next record or the central directory, so it
//! sees every byte of the file.

use std::io::Write;

/// How an entry is stored.
#[derive(Debug, Clone)]
pub struct Spec {
    pub name: String,
    pub data: Vec<u8>,
    pub deflate: bool,
    /// A data descriptor after the data: `Some((signature, zip64 sizes))`.
    pub descriptor: Option<(bool, bool)>,
    /// Sizes and offset in a zip64 extra field, with 0xFFFFFFFF in the
    /// header fields.
    pub zip64: bool,
    pub local_extra: Vec<u8>,
    pub central_extra: Vec<u8>,
    pub comment: Vec<u8>,
    pub utf8: bool,
}

impl Spec {
    pub fn new(name: &str, data: &[u8]) -> Spec {
        Spec {
            name: name.to_string(),
            data: data.to_vec(),
            deflate: true,
            descriptor: None,
            zip64: false,
            local_extra: vec![],
            central_extra: vec![],
            comment: vec![],
            utf8: false,
        }
    }
}

/// How the archive is laid out.
#[derive(Debug, Clone, Default)]
pub struct Layout {
    /// Bytes before the first entry.
    pub prefix: Vec<u8>,
    /// The order of the local records, as indexes into the entries; the
    /// central directory keeps the entries' order. Empty for the same order.
    pub local_order: Vec<usize>,
    pub comment: Vec<u8>,
    /// A zip64 end of central directory record.
    pub zip64_end: bool,
}

fn p16(o: &mut Vec<u8>, v: u16) {
    o.extend_from_slice(&v.to_le_bytes());
}
fn p32(o: &mut Vec<u8>, v: u32) {
    o.extend_from_slice(&v.to_le_bytes());
}
fn p64(o: &mut Vec<u8>, v: u64) {
    o.extend_from_slice(&v.to_le_bytes());
}

/// An extra field with this id and data.
pub fn extra(id: u16, data: &[u8]) -> Vec<u8> {
    let mut v = vec![];
    p16(&mut v, id);
    p16(&mut v, data.len() as u16);
    v.extend_from_slice(data);
    v
}

pub fn deflate(data: &[u8]) -> Vec<u8> {
    let mut w = flate2::write::DeflateEncoder::new(vec![], flate2::Compression::new(9));
    w.write_all(data).unwrap();
    w.finish().unwrap()
}

pub fn build(entries: &[Spec], layout: &Layout) -> Vec<u8> {
    let mut out = layout.prefix.clone();
    let order: Vec<usize> = if layout.local_order.is_empty() {
        (0..entries.len()).collect()
    } else {
        layout.local_order.clone()
    };
    let mut offsets = vec![0u64; entries.len()];
    let packed: Vec<Vec<u8>> = entries
        .iter()
        .map(|e| {
            if e.deflate {
                deflate(&e.data)
            } else {
                e.data.clone()
            }
        })
        .collect();
    let flags =
        |e: &Spec| u16::from(e.descriptor.is_some()) << 3 | if e.utf8 { 1 << 11 } else { 0 } | 2;
    let crc = |e: &Spec| crc32fast::hash(&e.data);
    for &i in &order {
        let e = &entries[i];
        offsets[i] = out.len() as u64;
        p32(&mut out, 0x0403_4b50);
        p16(&mut out, if e.zip64 { 45 } else { 20 });
        p16(&mut out, flags(e));
        p16(&mut out, if e.deflate { 8 } else { 0 });
        p16(&mut out, 0x6000);
        p16(&mut out, 0x5A21);
        let (c, cs, us) = match (e.descriptor.is_some(), e.zip64) {
            (true, _) => (0, 0, 0),
            (false, true) => (crc(e), u32::MAX, u32::MAX),
            (false, false) => (crc(e), packed[i].len() as u32, e.data.len() as u32),
        };
        p32(&mut out, c);
        p32(&mut out, cs);
        p32(&mut out, us);
        let mut ex = vec![];
        if e.zip64 {
            let mut z = vec![];
            p64(&mut z, e.data.len() as u64);
            p64(&mut z, packed[i].len() as u64);
            ex.extend(extra(1, &z));
        }
        ex.extend_from_slice(&e.local_extra);
        p16(&mut out, e.name.len() as u16);
        p16(&mut out, ex.len() as u16);
        out.extend_from_slice(e.name.as_bytes());
        out.extend_from_slice(&ex);
        out.extend_from_slice(&packed[i]);
        if let Some((signed, wide)) = e.descriptor {
            if signed {
                p32(&mut out, 0x0807_4b50);
            }
            p32(&mut out, crc(e));
            if wide {
                p64(&mut out, packed[i].len() as u64);
                p64(&mut out, e.data.len() as u64);
            } else {
                p32(&mut out, packed[i].len() as u32);
                p32(&mut out, e.data.len() as u32);
            }
        }
    }
    let cd = out.len() as u64;
    for (i, e) in entries.iter().enumerate() {
        p32(&mut out, 0x0201_4b50);
        p16(&mut out, 0x031E);
        p16(&mut out, if e.zip64 { 45 } else { 20 });
        p16(&mut out, flags(e));
        p16(&mut out, if e.deflate { 8 } else { 0 });
        p16(&mut out, 0x6000);
        p16(&mut out, 0x5A21);
        p32(&mut out, crc(e));
        let mut ex = vec![];
        if e.zip64 {
            p32(&mut out, u32::MAX);
            p32(&mut out, u32::MAX);
            let mut z = vec![];
            p64(&mut z, e.data.len() as u64);
            p64(&mut z, packed[i].len() as u64);
            p64(&mut z, offsets[i]);
            ex.extend(extra(1, &z));
        } else {
            p32(&mut out, packed[i].len() as u32);
            p32(&mut out, e.data.len() as u32);
        }
        ex.extend_from_slice(&e.central_extra);
        p16(&mut out, e.name.len() as u16);
        p16(&mut out, ex.len() as u16);
        p16(&mut out, e.comment.len() as u16);
        p16(&mut out, 0);
        p16(&mut out, 1);
        p32(&mut out, 0o100644 << 16);
        p32(&mut out, if e.zip64 { u32::MAX } else { offsets[i] as u32 });
        out.extend_from_slice(e.name.as_bytes());
        out.extend_from_slice(&ex);
        out.extend_from_slice(&e.comment);
    }
    let cd_size = out.len() as u64 - cd;
    let n = entries.len() as u64;
    if layout.zip64_end {
        let at = out.len() as u64;
        p32(&mut out, 0x0606_4b50);
        p64(&mut out, 44);
        p16(&mut out, 0x031E);
        p16(&mut out, 45);
        p32(&mut out, 0);
        p32(&mut out, 0);
        p64(&mut out, n);
        p64(&mut out, n);
        p64(&mut out, cd_size);
        p64(&mut out, cd);
        p32(&mut out, 0x0706_4b50);
        p32(&mut out, 0);
        p64(&mut out, at);
        p32(&mut out, 1);
    }
    p32(&mut out, 0x0605_4b50);
    p16(&mut out, 0);
    p16(&mut out, 0);
    let m16 = |v: u64| if layout.zip64_end { u16::MAX } else { v as u16 };
    let m32 = |v: u64| if layout.zip64_end { u32::MAX } else { v as u32 };
    p16(&mut out, m16(n));
    p16(&mut out, m16(n));
    p32(&mut out, m32(cd_size));
    p32(&mut out, m32(cd));
    p16(&mut out, layout.comment.len() as u16);
    out.extend_from_slice(&layout.comment);
    out
}

/// One entry as the archive stores it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub name: String,
    /// Local header, extra fields, data and data descriptor.
    pub local: Vec<u8>,
    /// The central directory record with its offset set to zero.
    pub central: Vec<u8>,
    pub offset: u64,
}

/// An archive split into its parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Records {
    pub prefix: Vec<u8>,
    /// In central directory order.
    pub entries: Vec<Record>,
    /// The order of the local records, as names.
    pub local_order: Vec<String>,
    pub comment: Vec<u8>,
}

fn g16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn g32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn g64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

pub fn records(b: &[u8]) -> Records {
    let end = (0..=b.len() - 22)
        .rev()
        .find(|&i| g32(b, i) == 0x0605_4b50 && i + 22 + g16(b, i + 20) as usize == b.len())
        .expect("an end record");
    let comment = b[end + 22..].to_vec();
    let (mut n, mut cd) = (g16(b, end + 10) as u64, g32(b, end + 16) as u64);
    if end >= 20 && g32(b, end - 20) == 0x0706_4b50 {
        let z = g64(b, end - 20 + 8) as usize;
        n = g64(b, z + 32);
        cd = g64(b, z + 48);
    }
    let mut at = cd as usize;
    let mut entries = vec![];
    for _ in 0..n {
        assert_eq!(g32(b, at), 0x0201_4b50);
        let (nl, xl, cl) = (
            g16(b, at + 28) as usize,
            g16(b, at + 30) as usize,
            g16(b, at + 32) as usize,
        );
        let len = 46 + nl + xl + cl;
        let mut central = b[at..at + len].to_vec();
        let name = String::from_utf8_lossy(&b[at + 46..at + 46 + nl]).into_owned();
        let mut offset = g32(b, at + 42) as u64;
        central[42..46].fill(0);
        if offset == u32::MAX as u64 {
            // The offset is the last field the zip64 extra holds here.
            let mut x = 46 + nl;
            while x < 46 + nl + xl {
                let (id, l) = (g16(&central, x), g16(&central, x + 2) as usize);
                if id == 1 {
                    let mut k = x + 4;
                    if g32(&central, 24) == u32::MAX {
                        k += 8;
                    }
                    if g32(&central, 20) == u32::MAX {
                        k += 8;
                    }
                    offset = g64(&central, k);
                    central[k..k + 8].fill(0);
                }
                x += 4 + l;
            }
        }
        entries.push(Record {
            name,
            local: vec![],
            central,
            offset,
        });
        at += len;
    }
    let mut starts: Vec<(u64, usize)> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| (e.offset, i))
        .collect();
    starts.sort();
    for (k, &(o, i)) in starts.iter().enumerate() {
        let next = starts.get(k + 1).map_or(cd, |s| s.0);
        entries[i].local = b[o as usize..next as usize].to_vec();
    }
    Records {
        prefix: b[..starts.first().map_or(cd, |s| s.0) as usize].to_vec(),
        local_order: starts
            .iter()
            .map(|&(_, i)| entries[i].name.clone())
            .collect(),
        entries,
        comment,
    }
}

/// Every entry of `before` except `changed` has the same complete records
/// in `after`, the entries keep their order, and prefix and comment stay.
pub fn assert_records_kept(before: &[u8], after: &[u8], changed: &[&str]) {
    let a = records(before);
    let b = records(after);
    assert_eq!(a.prefix, b.prefix, "the bytes before the first entry");
    assert_eq!(a.comment, b.comment, "the archive comment");
    let names = |r: &Records| r.entries.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&b)[..a.entries.len()], names(&a)[..], "entry order");
    assert_eq!(
        b.local_order[..a.entries.len()],
        a.local_order[..],
        "local record order"
    );
    for (x, y) in a.entries.iter().zip(&b.entries) {
        if changed.contains(&x.name.as_str()) {
            continue;
        }
        assert!(x.local == y.local, "{}: the local record changed", x.name);
        assert!(
            x.central == y.central,
            "{}: the central directory record changed",
            x.name
        );
    }
}
