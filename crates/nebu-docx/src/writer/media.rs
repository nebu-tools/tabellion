//! Pictures: the format and pixel size from the file's header, the media
//! part, and the inline drawing.
//!
//! Reading a header is bounded: PNG and JPEG are read chunk by chunk and
//! segment by segment, each step past its length, so the work is at most
//! the number of chunks; an SVG is parsed with the engine's own parser
//! under the document's limits. The same bytes inserted twice are one
//! part, named after their hash.

use std::collections::HashMap;

use crate::content::{Image, Source};
use crate::{Document, Error, Target};

use super::session::{Ctx, Session};
use super::xml::{attr, emu};

const IMAGE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
const SVG_EXT: &str = "{96DAC541-7B7A-43D3-8B79-37D633B846F1}";
const SVG_NS: &str = "http://schemas.microsoft.com/office/drawing/2016/SVG/main";

#[derive(Default)]
pub(crate) struct Media {
    /// Parts written in this session, by hash.
    parts: HashMap<String, String>,
}

/// A picture format the writer takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
    Png,
    Jpeg,
    Gif,
    Svg,
}

impl Format {
    fn ext(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Jpeg => "jpeg",
            Format::Gif => "gif",
            Format::Svg => "svg",
        }
    }

    fn content_type(self) -> &'static str {
        match self {
            Format::Png => "image/png",
            Format::Jpeg => "image/jpeg",
            Format::Gif => "image/gif",
            Format::Svg => "image/svg+xml",
        }
    }
}

/// The format and the natural size in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Header {
    pub format: Format,
    /// Width and height in points, if the file says.
    pub size: Option<(f64, f64)>,
}

fn be16(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from(*b.get(at)?) << 8 | u32::from(*b.get(at + 1)?))
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(be16(b, at)? << 16 | be16(b, at + 2)?)
}

fn le16(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from(*b.get(at)?) | u32::from(*b.get(at + 1)?) << 8)
}

fn points(px: u32, dpi: f64) -> f64 {
    f64::from(px) * 72.0 / dpi
}

fn png(b: &[u8]) -> Result<Header, String> {
    let w = be32(b, 16).ok_or("the PNG is cut short")?;
    let h = be32(b, 20).ok_or("the PNG is cut short")?;
    if &b[12..16] != b"IHDR" || w == 0 || h == 0 {
        return Err("the PNG has no image header".into());
    }
    let (mut dx, mut dy) = (96.0, 96.0);
    let mut at = 8usize;
    while let (Some(len), Some(kind)) = (be32(b, at), b.get(at + 4..at + 8)) {
        crate::cost::tick(1);
        if kind == b"pHYs"
            && let (Some(x), Some(y), Some(&unit)) =
                (be32(b, at + 8), be32(b, at + 12), b.get(at + 16))
            && unit == 1
            && x > 0
            && y > 0
        {
            // Pixels per metre.
            dx = f64::from(x) * 0.0254;
            dy = f64::from(y) * 0.0254;
        }
        if kind == b"IDAT" || kind == b"IEND" {
            break;
        }
        at = match at.checked_add(12).and_then(|a| a.checked_add(len as usize)) {
            Some(a) => a,
            None => break,
        };
    }
    Ok(Header {
        format: Format::Png,
        size: Some((points(w, dx), points(h, dy))),
    })
}

fn jpeg(b: &[u8]) -> Result<Header, String> {
    let mut at = 2usize;
    let (mut dx, mut dy) = (96.0, 96.0);
    while at + 4 <= b.len() {
        crate::cost::tick(1);
        if b[at] != 0xFF {
            return Err("the JPEG is damaged".into());
        }
        let marker = b[at + 1];
        if marker == 0xFF {
            at += 1;
            continue;
        }
        let len = be16(b, at + 2).ok_or("the JPEG is cut short")? as usize;
        if marker == 0xE0 && b.get(at + 4..at + 9) == Some(b"JFIF\0") {
            let unit = b.get(at + 11).copied().unwrap_or(0);
            let (x, y) = (be16(b, at + 12).unwrap_or(0), be16(b, at + 14).unwrap_or(0));
            if x > 0 && y > 0 {
                match unit {
                    1 => (dx, dy) = (f64::from(x), f64::from(y)),
                    2 => (dx, dy) = (f64::from(x) * 2.54, f64::from(y) * 2.54),
                    _ => {}
                }
            }
        }
        if matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            let h = be16(b, at + 5).ok_or("the JPEG is cut short")?;
            let w = be16(b, at + 7).ok_or("the JPEG is cut short")?;
            if w == 0 || h == 0 {
                return Err("the JPEG has no size".into());
            }
            return Ok(Header {
                format: Format::Jpeg,
                size: Some((points(w, dx), points(h, dy))),
            });
        }
        if len < 2 {
            return Err("the JPEG is damaged".into());
        }
        at += 2 + len;
    }
    Err("the JPEG has no frame header".into())
}

fn gif(b: &[u8]) -> Result<Header, String> {
    let w = le16(b, 6).ok_or("the GIF is cut short")?;
    let h = le16(b, 8).ok_or("the GIF is cut short")?;
    if w == 0 || h == 0 {
        return Err("the GIF has no size".into());
    }
    Ok(Header {
        format: Format::Gif,
        size: Some((points(w, 96.0), points(h, 96.0))),
    })
}

/// A length of SVG in points: a number with an optional unit.
fn svg_length(s: &str) -> Option<f64> {
    let s = s.trim();
    let split = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+' || c == 'e'))
        .unwrap_or(s.len());
    let (n, unit) = s.split_at(split);
    let n: f64 = n.parse().ok()?;
    let k = match unit.trim() {
        "" | "px" => 0.75,
        "pt" => 1.0,
        "pc" => 12.0,
        "in" => 72.0,
        "cm" => 72.0 / 2.54,
        "mm" => 72.0 / 25.4,
        _ => return None,
    };
    let v = n * k;
    (v.is_finite() && v > 0.0).then_some(v)
}

fn svg(b: &[u8], limits: &crate::Limits) -> Result<Header, String> {
    let t = crate::xml::Tree::parse_with_limits(b.to_vec(), limits)
        .map_err(|e| format!("the SVG is not well-formed XML: {e}"))?;
    let root = t.root_element();
    let is_svg = t
        .name(root)
        .ok()
        .flatten()
        .is_some_and(|n| n.local == "svg");
    if !is_svg {
        return Err("the file is not an SVG".into());
    }
    let get = |a| t.attribute(root, None, a).ok().flatten();
    let w = get("width").and_then(|v| svg_length(&v));
    let h = get("height").and_then(|v| svg_length(&v));
    let size = match (w, h) {
        (Some(w), Some(h)) => Some((w, h)),
        _ => get("viewBox").and_then(|v| {
            let n: Vec<f64> = v
                .split(|c: char| c == ',' || c.is_whitespace())
                .filter(|s| !s.is_empty())
                .filter_map(|s| s.parse().ok())
                .collect();
            match n.as_slice() {
                [_, _, w, h] if *w > 0.0 && *h > 0.0 => Some((w * 0.75, h * 0.75)),
                _ => None,
            }
        }),
    };
    Ok(Header {
        format: Format::Svg,
        size,
    })
}

/// The format and size of a picture from its first bytes.
pub(crate) fn header(b: &[u8], limits: &crate::Limits) -> Result<Header, String> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        return png(b);
    }
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return jpeg(b);
    }
    if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        return gif(b);
    }
    let head = &b[..b.len().min(512)];
    let text = String::from_utf8_lossy(head);
    let t = text.trim_start_matches('\u{FEFF}').trim_start();
    if t.starts_with("<?xml") || t.starts_with("<svg") || t.starts_with("<!--") {
        return svg(b, limits);
    }
    Err("the picture is not a PNG, JPEG, GIF or SVG".into())
}

fn bytes_of(s: &Session<'_>, src: &Source, path: &str) -> Result<Vec<u8>, Error> {
    match src {
        Source::Data(d) => Ok(d.clone()),
        Source::File(f) => {
            let Some(dir) = &s.opts.files else {
                return Err(Error::Invalid(format!(
                    "{path}.file: the writer reads no files unless WriteOptions::files names their directory; give the picture as data"
                )));
            };
            let p = dir.join(f);
            std::fs::read(&p).map_err(|e| Error::Io {
                action: "read",
                path: p,
                source: e,
            })
        }
    }
}

/// Stores a picture once and returns the relationship id from the part.
fn store(
    s: &mut Session<'_>,
    doc: &mut Document,
    bytes: Vec<u8>,
    format: Format,
    part: &str,
) -> Result<String, Error> {
    let hash = crate::sha256::hex(&bytes);
    let name = match s.media.parts.get(&hash) {
        Some(n) => n.clone(),
        None => {
            let name = format!("word/media/image-{}.{}", &hash[..16], format.ext());
            match doc.part_name(&name) {
                Some(existing) if doc.read_part(existing)? == bytes => {}
                Some(_) => {
                    return Err(Error::Unsupported(format!(
                        "the document has a part {name} with other bytes"
                    )));
                }
                None => doc.add_part(&name, format.content_type(), bytes)?,
            }
            s.media.parts.insert(hash, name.clone());
            name
        }
    };
    // An existing relationship to the part from this source serves.
    let rels = doc.relationships(part)?;
    if let Some(r) = rels.iter().find(|r| {
        r.kind == IMAGE
            && r.part
                .as_deref()
                .is_some_and(|p| p.eq_ignore_ascii_case(&name))
    }) {
        return Ok(r.id.clone());
    }
    s.rel(doc, part, IMAGE, Target::Part(&name))
}

pub(crate) fn picture(
    s: &mut Session<'_>,
    doc: &mut Document,
    image: &Image,
    ctx: &Ctx,
    path: &str,
) -> Result<String, Error> {
    let bytes = bytes_of(s, &image.source, path)?;
    let limits = doc.limits().clone();
    let head = header(&bytes, &limits).map_err(|e| Error::Invalid(format!("{path}: {e}")))?;
    let mut fallback = None;
    let mut natural = head.size;
    if head.format == Format::Svg {
        let Some(fb) = &image.fallback else {
            return Err(Error::Invalid(format!(
                "{path}: an SVG needs a PNG fallback, which Word shows where it cannot draw SVG; give it as fallback"
            )));
        };
        let fb_bytes = bytes_of(s, fb, &format!("{path}.fallback"))?;
        let fh = header(&fb_bytes, &limits)
            .map_err(|e| Error::Invalid(format!("{path}.fallback: {e}")))?;
        if fh.format != Format::Png {
            return Err(Error::Invalid(format!(
                "{path}.fallback: the fallback of an SVG is a PNG"
            )));
        }
        if natural.is_none() {
            natural = fh.size;
        }
        fallback = Some(fb_bytes);
    } else if image.fallback.is_some() {
        return Err(Error::Invalid(format!(
            "{path}.fallback: only an SVG takes a fallback"
        )));
    }
    let (nw, nh) = natural.unwrap_or((144.0, 108.0));
    let (mut w, mut h) = match (image.width, image.height) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, w * nh / nw),
        (None, Some(h)) => (h * nw / nh, h),
        (None, None) => (nw, nh),
    };
    if w > ctx.width {
        h *= ctx.width / w;
        w = ctx.width;
    }
    let (cx, cy) = (emu(w).max(1), emu(h).max(1));
    let id = s.ids.drawing();
    let name = format!("Picture {id}");
    let blip = match fallback {
        Some(fb) => {
            let png = store(s, doc, fb, Format::Png, &ctx.part)?;
            let svg = store(s, doc, bytes, Format::Svg, &ctx.part)?;
            format!(
                "<a:blip r:embed=\"{}\"><a:extLst><a:ext uri=\"{SVG_EXT}\"><asvg:svgBlip xmlns:asvg=\"{SVG_NS}\" r:embed=\"{}\"/></a:ext></a:extLst></a:blip>",
                attr(&png),
                attr(&svg)
            )
        }
        None => {
            let r = store(s, doc, bytes, head.format, &ctx.part)?;
            format!("<a:blip r:embed=\"{}\"/>", attr(&r))
        }
    };
    let descr = image
        .alt
        .as_deref()
        .map(|a| format!(" descr=\"{}\"", attr(a)))
        .unwrap_or_default();
    let title = image
        .title
        .as_deref()
        .map(|a| format!(" title=\"{}\"", attr(a)))
        .unwrap_or_default();
    Ok(format!(
        "<w:r><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\"><wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:effectExtent l=\"0\" t=\"0\" r=\"0\" b=\"0\"/><wp:docPr id=\"{id}\" name=\"{name}\"{descr}{title}/><wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect=\"1\"/></wp:cNvGraphicFramePr><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><pic:pic><pic:nvPicPr><pic:cNvPr id=\"{id}\" name=\"{name}\"{descr}/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill>{blip}<a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_of(w: u32, h: u32, ppm: Option<u32>) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
        b.extend(13u32.to_be_bytes());
        b.extend(b"IHDR");
        b.extend(w.to_be_bytes());
        b.extend(h.to_be_bytes());
        b.extend([8, 2, 0, 0, 0]);
        b.extend([0; 4]);
        if let Some(p) = ppm {
            b.extend(9u32.to_be_bytes());
            b.extend(b"pHYs");
            b.extend(p.to_be_bytes());
            b.extend(p.to_be_bytes());
            b.push(1);
            b.extend([0; 4]);
        }
        b
    }

    #[test]
    fn sizes_come_from_pixels_and_resolution() {
        let l = crate::Limits::default();
        let h = header(&png_of(192, 96, None), &l).unwrap();
        assert_eq!(h.size, Some((144.0, 72.0)));
        // 7874 pixels per metre is 200 per inch.
        let h = header(&png_of(200, 100, Some(7874)), &l).unwrap();
        let (w, _) = h.size.unwrap();
        assert!((w - 72.0).abs() < 0.1, "{w}");
        let gif = [b"GIF89a".to_vec(), vec![96, 0, 48, 0]].concat();
        assert_eq!(header(&gif, &l).unwrap().size, Some((72.0, 36.0)));
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100"/>"#;
        assert_eq!(header(svg, &l).unwrap().size, Some((150.0, 75.0)));
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="2in" height="1in"/>"#;
        assert_eq!(header(svg, &l).unwrap().size, Some((144.0, 72.0)));
    }

    #[test]
    fn damaged_headers_are_errors() {
        let l = crate::Limits::default();
        for b in [
            &b"\x89PNG\r\n\x1a\n"[..],
            &[0xFF, 0xD8, 0xFF],
            &[0xFF, 0xD8, 0xFF, 0xE0, 0, 0],
            b"GIF89a",
            b"<svg",
            b"hello",
        ] {
            assert!(header(b, &l).is_err(), "{b:?}");
        }
    }
}
