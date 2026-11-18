//! The zip format, as far as the engine needs it.
//!
//! The reader finds every entry's complete record, its local header, data
//! and data descriptor, and its central directory record, as byte ranges of
//! the file. The writer copies those ranges for entries that did not change
//! and writes changed and new entries afresh. Whatever the reader cannot
//! carry through as it is, it refuses: encrypted entries, archives on more
//! than one disk, bytes between records that belong to no entry.

use std::io::{Read, Write};
use std::ops::Range;

use crate::Error;

const LOCAL: u32 = 0x0403_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const END: u32 = 0x0605_4b50;
const END64: u32 = 0x0606_4b50;
const LOCATOR64: u32 = 0x0706_4b50;
const DESCRIPTOR: u32 = 0x0807_4b50;
const ZIP64_EXTRA: u16 = 0x0001;

const FLAG_ENCRYPTED: u16 = 1;
const FLAG_DESCRIPTOR: u16 = 1 << 3;
const FLAG_STRONG: u16 = 1 << 6;
const FLAG_UTF8: u16 = 1 << 11;
const FLAG_MASKED: u16 = 1 << 13;

pub(crate) const STORED: u16 = 0;
pub(crate) const DEFLATED: u16 = 8;

/// One entry of the archive, as ranges of the file and the fields the
/// engine reads.
#[derive(Debug, Clone)]
pub(crate) struct Entry {
    pub name: String,
    /// The name as stored.
    name_raw: Range<usize>,
    /// The local record: header, data and data descriptor.
    pub local: Range<usize>,
    /// The compressed data.
    data: Range<usize>,
    /// The central directory record.
    pub central: Range<usize>,
    /// Where the record's local header offset is, relative to `central`,
    /// and whether it is the 8-byte field of a zip64 extra.
    offset_at: (usize, bool),
    pub method: u16,
    flags: u16,
    time: u16,
    date: u16,
    crc: u32,
    pub compressed: u64,
    pub size: u64,
    made_by: u16,
    internal: u16,
    external: u32,
    comment: Range<usize>,
}

/// The zip64 end of central directory record, kept for writing back.
#[derive(Debug, Clone)]
struct End64 {
    made_by: u16,
    needed: u16,
    extensible: Range<usize>,
}

/// A zip archive read from bytes.
#[derive(Debug)]
pub(crate) struct Archive {
    pub bytes: Vec<u8>,
    /// Bytes before the first entry, such as a program that unpacks the
    /// archive.
    prefix: Range<usize>,
    /// In central directory order.
    pub entries: Vec<Entry>,
    comment: Range<usize>,
    end64: Option<End64>,
}

fn damaged(m: impl Into<String>) -> Error {
    Error::Zip(m.into())
}

/// Little-endian fields at fixed places of a record.
struct Fields<'a>(&'a [u8]);

impl Fields<'_> {
    fn u16(&self, at: usize) -> u16 {
        u16::from_le_bytes([self.0[at], self.0[at + 1]])
    }
    fn u32(&self, at: usize) -> u32 {
        u32::from_le_bytes([self.0[at], self.0[at + 1], self.0[at + 2], self.0[at + 3]])
    }
    fn u64(&self, at: usize) -> u64 {
        let mut b = [0u8; 8];
        b.copy_from_slice(&self.0[at..at + 8]);
        u64::from_le_bytes(b)
    }
}

/// `len` bytes at `at`, or an error naming `what`.
fn slice<'a>(b: &'a [u8], at: u64, len: u64, what: &str) -> Result<(&'a [u8], usize), Error> {
    let end = at.checked_add(len).filter(|&e| e <= b.len() as u64);
    match end {
        Some(end) => Ok((&b[at as usize..end as usize], at as usize)),
        None => Err(damaged(format!("{what} lies outside the file"))),
    }
}

/// The extra fields of a record as (id, data range relative to `extra`).
fn extras(extra: &[u8]) -> Result<Vec<(u16, Range<usize>)>, Error> {
    let mut out = vec![];
    let mut i = 0;
    while i < extra.len() {
        if i + 4 > extra.len() {
            return Err(damaged("an extra field is cut short"));
        }
        let f = Fields(&extra[i..]);
        let (id, len) = (f.u16(0), f.u16(2) as usize);
        let start = i + 4;
        if start + len > extra.len() {
            return Err(damaged("an extra field is longer than its record"));
        }
        out.push((id, start..start + len));
        i = start + len;
    }
    Ok(out)
}

/// Code page 437, the encoding of names without the UTF-8 flag, from 0x80.
const CP437: &str = "ÇüéâäàåçêëèïîìÄÅÉæÆôöòûùÿÖÜ¢£¥₧ƒáíóúñÑªº¿⌐¬½¼¡«»░▒▓│┤╡╢╖╕╣║╗╝╜╛┐└┴┬├─┼╞╟╚╔╩╦╠═╬╧╨╤╥╙╘╒╓╫╪┘┌█▄▌▐▀αßΓπΣσµτΦΘΩδ∞φε∩≡±≥≤⌠⌡÷≈°∙·√ⁿ²■\u{A0}";

fn decode_name(raw: &[u8], utf8: bool) -> Result<String, Error> {
    if utf8 || raw.is_ascii() {
        return String::from_utf8(raw.to_vec())
            .map_err(|_| damaged("an entry name is marked as UTF-8 but is not"));
    }
    let high: Vec<char> = CP437.chars().collect();
    Ok(raw
        .iter()
        .map(|&b| {
            if b < 0x80 {
                b as char
            } else {
                high[(b - 0x80) as usize]
            }
        })
        .collect())
}

impl Archive {
    pub(crate) fn read(bytes: Vec<u8>, max_entries: usize) -> Result<Archive, Error> {
        let b = &bytes[..];
        let n = b.len();
        if n < 22 {
            return Err(damaged("the file is too short to be a zip archive"));
        }
        // The end record is the last 22 bytes plus its comment.
        let lowest = n.saturating_sub(22 + u16::MAX as usize);
        let end = (lowest..=n - 22)
            .rev()
            .find(|&i| {
                let f = Fields(&b[i..]);
                f.u32(0) == END && i + 22 + f.u16(20) as usize == n
            })
            .ok_or_else(|| {
                damaged("it has no end of central directory record at the end of the file")
            })?;
        let f = Fields(&b[end..]);
        let (disk, cd_disk) = (f.u16(4), f.u16(6));
        let (on_disk, total) = (f.u16(8) as u64, f.u16(10) as u64);
        let (mut cd_size, mut cd_offset) = (f.u32(12) as u64, f.u32(16) as u64);
        let comment = end + 22..n;
        let mut total = total;
        let mut on_disk = on_disk;
        let mut cd_end = end;
        let mut end64 = None;
        let mut disks = (disk as u32, cd_disk as u32);
        if end >= 20 && Fields(&b[end - 20..]).u32(0) == LOCATOR64 {
            let l = Fields(&b[end - 20..end]);
            let at = l.u64(8);
            if l.u32(4) != 0 || l.u32(16) > 1 {
                return Err(Error::Unsupported(
                    "zip archives split over several disks".into(),
                ));
            }
            let (head, at) = slice(b, at, 56, "the zip64 end record")?;
            let g = Fields(head);
            if g.u32(0) != END64 {
                return Err(damaged("the zip64 end record is missing"));
            }
            let record_end = (at as u64 + 12).checked_add(g.u64(4));
            if record_end != Some(end as u64 - 20) {
                return Err(damaged(
                    "the zip64 end record does not end where the archive's end starts",
                ));
            }
            disks = (g.u32(16), g.u32(20));
            on_disk = g.u64(24);
            total = g.u64(32);
            cd_size = g.u64(40);
            cd_offset = g.u64(48);
            cd_end = at;
            end64 = Some(End64 {
                made_by: g.u16(12),
                needed: g.u16(14),
                extensible: at + 56..end - 20,
            });
        }
        if disks != (0, 0) || on_disk != total {
            return Err(Error::Unsupported(
                "zip archives split over several disks".into(),
            ));
        }
        if total > max_entries as u64 {
            return Err(Error::Limit {
                what: format!("{total} zip entries"),
                field: "max_entries",
                limit: max_entries as u64,
            });
        }
        if cd_offset.checked_add(cd_size) != Some(cd_end as u64) {
            return Err(damaged(
                "the central directory is not where the archive says it is",
            ));
        }
        let cd_start = cd_offset as usize;
        if total.saturating_mul(46) > cd_size {
            return Err(damaged(
                "the central directory is too small for its entries",
            ));
        }

        let mut entries = Vec::with_capacity(total as usize);
        let mut at = cd_start;
        for _ in 0..total {
            entries.push(central_record(b, &mut at, cd_end)?);
        }
        if at != cd_end {
            return Err(damaged(
                "the central directory has bytes that belong to no entry",
            ));
        }
        for e in &mut entries {
            local_record(b, e)?;
        }

        // The local records follow one another without gaps or overlaps.
        let mut order: Vec<usize> = (0..entries.len()).collect();
        order.sort_by_key(|&i| entries[i].local.start);
        let first = order.first().map_or(cd_start, |&i| entries[i].local.start);
        let mut pos = first;
        for &i in &order {
            let e = &entries[i];
            if e.local.start < pos {
                return Err(damaged(format!("the entry {} overlaps another", e.name)));
            }
            if e.local.start > pos {
                return Err(Error::Unsupported(format!(
                    "zip archives with bytes between their entries (before {})",
                    e.name
                )));
            }
            pos = e.local.end;
        }
        if pos > cd_start {
            return Err(damaged("an entry overlaps the central directory"));
        }
        if pos < cd_start {
            return Err(Error::Unsupported(
                "zip archives with bytes between their last entry and the central directory".into(),
            ));
        }
        Ok(Archive {
            prefix: 0..first,
            entries,
            comment,
            end64,
            bytes,
        })
    }

    /// The compressed data of an entry.
    pub(crate) fn data(&self, e: &Entry) -> &[u8] {
        &self.bytes[e.data.clone()]
    }

    /// Decompresses an entry, reading at most `limit` bytes; `None` if the
    /// entry holds more. Checks the size and checksum.
    pub(crate) fn decompress(&self, e: &Entry, limit: u64) -> Result<Option<Vec<u8>>, Error> {
        let data = self.data(e);
        // Deflate expands at most about 1,032 times; a header cannot make
        // the engine reserve more than that.
        let room = e
            .size
            .min(limit)
            .min(e.compressed.saturating_mul(1032) + 64);
        let mut out = Vec::with_capacity(room as usize);
        match e.method {
            STORED => {
                let keep = usize::try_from(limit.saturating_add(1)).unwrap_or(usize::MAX);
                out.extend_from_slice(&data[..data.len().min(keep)]);
            }
            DEFLATED => {
                flate2::read::DeflateDecoder::new(data)
                    .take(limit.saturating_add(1))
                    .read_to_end(&mut out)
                    .map_err(|err| damaged(format!("{}: {err}", e.name)))?;
            }
            m => {
                return Err(Error::Unsupported(format!(
                    "{}: compression method {m}",
                    e.name
                )));
            }
        }
        if out.len() as u64 > limit {
            return Ok(None);
        }
        if out.len() as u64 != e.size {
            return Err(damaged(format!(
                "{}: the entry holds {} bytes but its header says {}",
                e.name,
                out.len(),
                e.size
            )));
        }
        if crc32fast::hash(&out) != e.crc {
            return Err(damaged(format!("{}: the checksum does not match", e.name)));
        }
        Ok(Some(out))
    }

    /// Writes the archive. `parts[i]` is `None` to keep source entry `i` as
    /// it is, or the new bytes of its part; `added` are new entries, written
    /// after all others.
    pub(crate) fn write(
        &self,
        parts: &[Option<&[u8]>],
        added: &[(&str, &[u8])],
    ) -> Result<Vec<u8>, Error> {
        let b = &self.bytes[..];
        let mut out = Vec::with_capacity(b.len() + 4096);
        out.extend_from_slice(&b[self.prefix.clone()]);
        let mut offsets = vec![0u64; self.entries.len()];
        let mut fresh: Vec<Option<Fresh>> = vec![None; self.entries.len()];
        let mut order: Vec<usize> = (0..self.entries.len()).collect();
        order.sort_by_key(|&i| self.entries[i].local.start);
        for i in order {
            let e = &self.entries[i];
            offsets[i] = out.len() as u64;
            match parts.get(i).copied().flatten() {
                None => out.extend_from_slice(&b[e.local.clone()]),
                Some(data) => {
                    let method = if e.method == STORED { STORED } else { DEFLATED };
                    let f = Fresh::new(
                        &b[e.name_raw.clone()],
                        e.flags & FLAG_UTF8,
                        method,
                        (e.time, e.date),
                        data,
                    )?;
                    f.write_local(&mut out);
                    fresh[i] = Some(f);
                }
            }
        }
        let mut new = vec![];
        for (name, data) in added {
            let flags = if name.is_ascii() { 0 } else { FLAG_UTF8 };
            // 1980-01-01 00:00, the earliest time a zip entry can have.
            let f = Fresh::new(name.as_bytes(), flags, DEFLATED, (0, 0x21), data)?;
            let at = out.len() as u64;
            f.write_local(&mut out);
            new.push((f, at));
        }

        let cd_start = out.len() as u64;
        for (i, e) in self.entries.iter().enumerate() {
            match &fresh[i] {
                None => {
                    let record = &b[e.central.clone()];
                    let start = out.len();
                    out.extend_from_slice(record);
                    let (at, wide) = e.offset_at;
                    let at = start + at;
                    if wide {
                        out[at..at + 8].copy_from_slice(&offsets[i].to_le_bytes());
                    } else {
                        let Ok(o) = u32::try_from(offsets[i]) else {
                            return Err(Error::Unsupported(format!(
                                "saving: {} would start past 4 GiB, and its record has no room for that",
                                e.name
                            )));
                        };
                        if o == u32::MAX {
                            return Err(Error::Unsupported(format!(
                                "saving: {} would start at 4 GiB, and its record has no room for that",
                                e.name
                            )));
                        }
                        out[at..at + 4].copy_from_slice(&o.to_le_bytes());
                    }
                }
                Some(f) => f.write_central(
                    &mut out,
                    offsets[i],
                    e.made_by,
                    e.internal,
                    e.external,
                    &b[e.comment.clone()],
                ),
            }
        }
        for (f, at) in &new {
            f.write_central(&mut out, *at, 20, 0, 0, &[]);
        }
        let cd_end = out.len() as u64;
        let count = (self.entries.len() + new.len()) as u64;
        let cd_size = cd_end - cd_start;
        let big = count >= 0xFFFF || cd_size >= 0xFFFF_FFFF || cd_start >= 0xFFFF_FFFF;
        if self.end64.is_some() || big {
            let (made_by, needed, ext) = match &self.end64 {
                Some(e) => (e.made_by, e.needed, &b[e.extensible.clone()]),
                None => (45, 45, &b[0..0]),
            };
            let at = out.len() as u64;
            put32(&mut out, END64);
            put64(&mut out, 44 + ext.len() as u64);
            put16(&mut out, made_by);
            put16(&mut out, needed);
            put32(&mut out, 0);
            put32(&mut out, 0);
            put64(&mut out, count);
            put64(&mut out, count);
            put64(&mut out, cd_size);
            put64(&mut out, cd_start);
            out.extend_from_slice(ext);
            put32(&mut out, LOCATOR64);
            put32(&mut out, 0);
            put64(&mut out, at);
            put32(&mut out, 1);
        }
        let c16 = |v: u64| {
            u16::try_from(v)
                .ok()
                .filter(|&v| v != u16::MAX)
                .unwrap_or(u16::MAX)
        };
        let c32 = |v: u64| {
            u32::try_from(v)
                .ok()
                .filter(|&v| v != u32::MAX)
                .unwrap_or(u32::MAX)
        };
        put32(&mut out, END);
        put16(&mut out, 0);
        put16(&mut out, 0);
        put16(&mut out, c16(count));
        put16(&mut out, c16(count));
        put32(&mut out, c32(cd_size));
        put32(&mut out, c32(cd_start));
        let comment = &b[self.comment.clone()];
        put16(&mut out, comment.len() as u16);
        out.extend_from_slice(comment);
        Ok(out)
    }
}

/// Reads the central directory record at `*at` and moves past it.
fn central_record(b: &[u8], at: &mut usize, cd_end: usize) -> Result<Entry, Error> {
    let start = *at;
    if start + 46 > cd_end {
        return Err(damaged("a central directory record is cut short"));
    }
    let f = Fields(&b[start..]);
    if f.u32(0) != CENTRAL {
        return Err(damaged("a central directory record is damaged"));
    }
    let (nlen, xlen, clen) = (f.u16(28) as usize, f.u16(30) as usize, f.u16(32) as usize);
    let end = start + 46 + nlen + xlen + clen;
    if end > cd_end {
        return Err(damaged("a central directory record is cut short"));
    }
    let flags = f.u16(8);
    let method = f.u16(10);
    let name_raw = start + 46..start + 46 + nlen;
    let name = decode_name(&b[name_raw.clone()], flags & FLAG_UTF8 != 0)?;
    if flags & (FLAG_ENCRYPTED | FLAG_STRONG | FLAG_MASKED) != 0 || method == 99 {
        return Err(Error::Unsupported(format!(
            "encrypted zip entries ({name}); remove the password in Word and save again"
        )));
    }
    let mut size = f.u32(24) as u64;
    let mut compressed = f.u32(20) as u64;
    let mut offset = f.u32(42) as u64;
    let mut offset_at = (42, false);
    let mut disk = f.u16(34) as u32;
    let extra_start = name_raw.end;
    let extra = &b[extra_start..extra_start + xlen];
    if let Some((_, r)) = extras(extra)?
        .into_iter()
        .find(|(id, _)| *id == ZIP64_EXTRA)
    {
        let z = &extra[r.clone()];
        let mut k = 0;
        let next = |what: &str, k: &mut usize| -> Result<u64, Error> {
            if *k + 8 > z.len() {
                return Err(damaged(format!("the zip64 field of {name} has no {what}")));
            }
            let v = Fields(z).u64(*k);
            *k += 8;
            Ok(v)
        };
        if size == 0xFFFF_FFFF {
            size = next("size", &mut k)?;
        }
        if compressed == 0xFFFF_FFFF {
            compressed = next("compressed size", &mut k)?;
        }
        if offset == 0xFFFF_FFFF {
            offset_at = (extra_start - start + r.start + k, true);
            offset = next("offset", &mut k)?;
        }
        if disk == 0xFFFF {
            if k + 4 > z.len() {
                return Err(damaged(format!("the zip64 field of {name} has no disk")));
            }
            disk = Fields(z).u32(k);
        }
    }
    if disk != 0 {
        return Err(Error::Unsupported(
            "zip archives split over several disks".into(),
        ));
    }
    *at = end;
    Ok(Entry {
        name,
        name_raw,
        local: offset as usize..offset as usize,
        data: 0..0,
        central: start..end,
        offset_at,
        method,
        flags,
        time: f.u16(12),
        date: f.u16(14),
        crc: f.u32(16),
        compressed,
        size,
        made_by: f.u16(4),
        internal: f.u16(36),
        external: f.u32(38),
        comment: end - clen..end,
    })
}

/// Finds the entry's local record: header, data and data descriptor.
fn local_record(b: &[u8], e: &mut Entry) -> Result<(), Error> {
    let name = e.name.clone();
    let what = format!("the local header of {name}");
    let (head, start) = slice(b, e.local.start as u64, 30, &what)?;
    let f = Fields(head);
    if f.u32(0) != LOCAL {
        return Err(damaged(format!("{what} is missing")));
    }
    let (nlen, xlen) = (f.u16(26) as u64, f.u16(28) as u64);
    let (rest, _) = slice(b, start as u64 + 30, nlen + xlen, &what)?;
    if rest[..nlen as usize] != b[e.name_raw.clone()] {
        return Err(damaged(format!(
            "{what} names another entry; the archive is ambiguous"
        )));
    }
    let flags = f.u16(6);
    if flags & (FLAG_ENCRYPTED | FLAG_STRONG) != 0 || f.u16(8) != e.method {
        return Err(damaged(format!(
            "{what} does not match its directory record"
        )));
    }
    let zip64 = extras(&rest[nlen as usize..])?
        .iter()
        .any(|(id, _)| *id == ZIP64_EXTRA);
    let data_start = start as u64 + 30 + nlen + xlen;
    let (_, ds) = slice(b, data_start, e.compressed, &format!("the data of {name}"))?;
    let data_end = ds + e.compressed as usize;
    let mut end = data_end;
    if flags & FLAG_DESCRIPTOR != 0 {
        let after = &b[data_end..];
        let signed = after.len() >= 4 && Fields(after).u32(0) == DESCRIPTOR;
        let body = if signed { &after[4..] } else { after };
        let narrow = body.len() >= 12 && {
            let g = Fields(body);
            g.u32(0) == e.crc && g.u32(4) as u64 == e.compressed && g.u32(8) as u64 == e.size
        };
        let wide = body.len() >= 20 && {
            let g = Fields(body);
            g.u32(0) == e.crc && g.u64(4) == e.compressed && g.u64(12) == e.size
        };
        // A descriptor with zip64 sizes follows a header with a zip64 field;
        // try that width first.
        let len = match (zip64, wide, narrow) {
            (true, true, _) | (false, true, false) => 20,
            (_, _, true) => 12,
            _ => {
                return Err(damaged(format!(
                    "the data descriptor of {name} does not match its directory record"
                )));
            }
        };
        end = data_end + usize::from(signed) * 4 + len;
    }
    e.data = ds..data_end;
    e.local = start..end;
    Ok(())
}

/// A changed or new entry, compressed and ready to write.
#[derive(Clone)]
struct Fresh {
    name: Vec<u8>,
    flags: u16,
    method: u16,
    time: (u16, u16),
    crc: u32,
    size: u64,
    data: Vec<u8>,
}

impl Fresh {
    fn new(
        name: &[u8],
        flags: u16,
        method: u16,
        time: (u16, u16),
        bytes: &[u8],
    ) -> Result<Fresh, Error> {
        let data = match method {
            STORED => bytes.to_vec(),
            _ => {
                let mut w = flate2::write::DeflateEncoder::new(
                    Vec::with_capacity(bytes.len() / 4),
                    flate2::Compression::default(),
                );
                w.write_all(bytes)
                    .and_then(|_| w.finish())
                    .map_err(|e| damaged(e.to_string()))?
            }
        };
        if name.len() > u16::MAX as usize {
            return Err(Error::Edit(
                "a part name is too long for a zip entry".into(),
            ));
        }
        Ok(Fresh {
            name: name.to_vec(),
            flags,
            method,
            time,
            crc: crc32fast::hash(bytes),
            size: bytes.len() as u64,
            data,
        })
    }

    fn big(&self) -> bool {
        self.size >= 0xFFFF_FFFF || self.data.len() as u64 >= 0xFFFF_FFFF
    }

    fn write_local(&self, out: &mut Vec<u8>) {
        let big = self.big();
        put32(out, LOCAL);
        put16(out, if big { 45 } else { 20 });
        put16(out, self.flags);
        put16(out, self.method);
        put16(out, self.time.0);
        put16(out, self.time.1);
        put32(out, self.crc);
        let (c, s) = if big {
            (u32::MAX, u32::MAX)
        } else {
            (self.data.len() as u32, self.size as u32)
        };
        put32(out, c);
        put32(out, s);
        put16(out, self.name.len() as u16);
        put16(out, if big { 20 } else { 0 });
        out.extend_from_slice(&self.name);
        if big {
            put16(out, ZIP64_EXTRA);
            put16(out, 16);
            put64(out, self.size);
            put64(out, self.data.len() as u64);
        }
        out.extend_from_slice(&self.data);
    }

    fn write_central(
        &self,
        out: &mut Vec<u8>,
        offset: u64,
        made_by: u16,
        internal: u16,
        external: u32,
        comment: &[u8],
    ) {
        let wide_size = self.big();
        let wide_offset = offset >= 0xFFFF_FFFF;
        let mut z = vec![];
        if wide_size {
            put64(&mut z, self.size);
            put64(&mut z, self.data.len() as u64);
        }
        if wide_offset {
            put64(&mut z, offset);
        }
        put32(out, CENTRAL);
        put16(out, made_by);
        put16(out, if z.is_empty() { 20 } else { 45 });
        put16(out, self.flags);
        put16(out, self.method);
        put16(out, self.time.0);
        put16(out, self.time.1);
        put32(out, self.crc);
        if wide_size {
            put32(out, u32::MAX);
            put32(out, u32::MAX);
        } else {
            put32(out, self.data.len() as u32);
            put32(out, self.size as u32);
        }
        put16(out, self.name.len() as u16);
        put16(out, if z.is_empty() { 0 } else { 4 + z.len() as u16 });
        put16(out, comment.len() as u16);
        put16(out, 0);
        put16(out, internal);
        put32(out, external);
        put32(out, if wide_offset { u32::MAX } else { offset as u32 });
        out.extend_from_slice(&self.name);
        if !z.is_empty() {
            put16(out, ZIP64_EXTRA);
            put16(out, z.len() as u16);
            out.extend_from_slice(&z);
        }
        out.extend_from_slice(comment);
    }
}

fn put16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}
