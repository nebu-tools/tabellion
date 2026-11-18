//! Characters, names, references and escaping, as XML 1.0 defines them.

use std::borrow::Cow;

/// A character that may start an XML name, without the colon (NCName rules).
pub(crate) fn is_name_start(c: char) -> bool {
    matches!(c,
        'A'..='Z' | '_' | 'a'..='z'
        | '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{2FF}'
        | '\u{370}'..='\u{37D}' | '\u{37F}'..='\u{1FFF}' | '\u{200C}'..='\u{200D}'
        | '\u{2070}'..='\u{218F}' | '\u{2C00}'..='\u{2FEF}' | '\u{3001}'..='\u{D7FF}'
        | '\u{F900}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FFFD}' | '\u{10000}'..='\u{EFFFF}')
}

/// A character that may continue an XML name, without the colon.
pub(crate) fn is_name_char(c: char) -> bool {
    is_name_start(c)
        || matches!(c, '-' | '.' | '0'..='9' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}')
}

/// A character XML 1.0 allows anywhere in a document.
pub(crate) fn is_xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..)
}

pub(crate) fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

/// A name without a colon.
pub(crate) fn is_ncname(s: &str) -> bool {
    let mut cs = s.chars();
    cs.next().is_some_and(is_name_start) && cs.all(is_name_char)
}

/// Splits a qualified name into prefix and local part. `None` if it is not
/// a valid qualified name.
pub(crate) fn split_qname(s: &str) -> Option<(&str, &str)> {
    match s.split_once(':') {
        Some((p, l)) if is_ncname(p) && is_ncname(l) => Some((p, l)),
        Some(_) => None,
        None if is_ncname(s) => Some(("", s)),
        None => None,
    }
}

/// The byte offset of the first character XML does not allow.
pub(crate) fn first_invalid_char(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c < 0x20 && !matches!(c, b'\t' | b'\n' | b'\r') {
            return Some(i);
        }
        // U+FFFE and U+FFFF are EF BF BE and EF BF BF.
        if c == 0xEF && b.get(i + 1) == Some(&0xBF) && matches!(b.get(i + 2), Some(0xBE | 0xBF)) {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Decodes the reference that starts at `s[0] == '&'`. Returns the character
/// and the length of the reference including `&` and `;`.
///
/// A character reference may have any number of leading zeros. The scan
/// stops at the first character that cannot continue the reference, so a
/// text of many `&` stays linear.
pub(crate) fn reference(s: &str) -> Result<(char, usize), String> {
    let b = s.as_bytes();
    let unclosed =
        || "'&' must start a reference such as &amp; (write a literal '&' as &amp;)".to_string();
    if b.get(1) == Some(&b'#') {
        let (radix, start) = if b.get(2) == Some(&b'x') {
            (16, 3)
        } else {
            (10, 2)
        };
        let mut value: u32 = 0;
        let mut i = start;
        while let Some(d) = b.get(i).and_then(|&c| (c as char).to_digit(radix)) {
            // Past U+10FFFF nothing is a character; stop counting there.
            value = value.saturating_mul(radix).saturating_add(d).min(0x11_0000);
            i += 1;
        }
        let digits = &s[start..i];
        if b.get(i) != Some(&b';') {
            return Err(unclosed());
        }
        if digits.is_empty() {
            return Err(format!("&{}; has no digits", &s[1..i]));
        }
        return match char::from_u32(value).filter(|&c| is_xml_char(c)) {
            Some(c) => Ok((c, i + 1)),
            None => Err(format!(
                "&{}; does not name a character XML allows",
                &s[1..i]
            )),
        };
    }
    let end = 1 + s[1..]
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'))
        .unwrap_or(s.len() - 1);
    if b.get(end) != Some(&b';') || end == 1 {
        return Err(unclosed());
    }
    let name = &s[1..end];
    let c = match name {
        "lt" => '<',
        "gt" => '>',
        "amp" => '&',
        "apos" => '\'',
        "quot" => '"',
        _ => {
            return Err(format!(
                "unknown entity &{name}; (only &lt; &gt; &amp; &apos; &quot; and character references are defined)"
            ));
        }
    };
    Ok((c, end + 1))
}

/// Checks every reference in text or in an attribute value. On error, returns
/// the byte offset within `s` and the reason.
pub(crate) fn check_references(s: &str) -> Result<(), (usize, String)> {
    let mut from = 0;
    while let Some(k) = s[from..].find('&') {
        let at = from + k;
        let (_, len) = reference(&s[at..]).map_err(|m| (at, m))?;
        from = at + len;
    }
    Ok(())
}

/// Decodes references and normalises line ends as an XML processor does. In
/// an attribute value, literal tabs and line ends also become spaces.
pub(crate) fn decode(s: &str, attribute: bool) -> Cow<'_, str> {
    let special = |b: &u8| matches!(b, b'&' | b'\r') || (attribute && matches!(b, b'\t' | b'\n'));
    if !s.as_bytes().iter().any(special) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(k) = rest.as_bytes().iter().position(special) {
        out.push_str(&rest[..k]);
        rest = &rest[k..];
        match rest.as_bytes()[0] {
            b'&' => match reference(rest) {
                Ok((c, len)) => {
                    out.push(c);
                    rest = &rest[len..];
                }
                // Checked when the text was parsed or set; kept as written if not.
                Err(_) => {
                    out.push('&');
                    rest = &rest[1..];
                }
            },
            b'\r' => {
                out.push(if attribute { ' ' } else { '\n' });
                rest = rest.strip_prefix("\r\n").unwrap_or(&rest[1..]);
            }
            _ => {
                out.push(' ');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// Escapes character data so that it reads back as `s`.
pub(crate) fn escape_text(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#13;"),
            _ => out.push(c),
        }
    }
}

/// Escapes an attribute value for the given quote so that it reads back as
/// `s`, tabs and line ends included.
pub(crate) fn escape_attribute(s: &str, quote: char, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '"' if quote == '"' => out.push_str("&quot;"),
            '\'' if quote == '\'' => out.push_str("&apos;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            _ => out.push(c),
        }
    }
}

/// Checks that a string can be written as XML text: every character allowed.
pub(crate) fn check_chars(s: &str) -> Result<(), String> {
    match s.chars().find(|&c| !is_xml_char(c)) {
        Some(c) => Err(format!("U+{:04X} cannot appear in XML", c as u32)),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_decode() {
        assert_eq!(decode("a &amp; b &#169; &#xA9; &lt;", false), "a & b © © <");
        assert_eq!(decode("a\r\nb\rc", false), "a\nb\nc");
        assert_eq!(decode("a\tb\r\nc&#10;", true), "a b c\n");
        assert!(check_references("&nbsp;").is_err());
        assert!(check_references("&#0;").is_err());
        assert!(check_references("&#xD800;").is_err());
        assert!(check_references("& b").is_err());
        assert!(check_references("&#x;").is_err());
        assert!(check_references("&#+1;").is_err());
        assert!(check_references("&#x10FFFF;").is_ok());
        assert_eq!(decode("&#000000000000065;&#x0000000000041;", false), "AA");
        assert!(check_references("&#0000000000000000000000000;").is_err());
        assert!(check_references("&#x110000;").is_err());
        assert!(check_references("&#99999999999999999999999;").is_err());
        assert!(check_references("&#xFFFE;").is_err());
        assert!(check_references("&#X41;").is_err());
        assert!(check_references("&;").is_err());
    }

    #[test]
    fn names() {
        assert_eq!(split_qname("w:p"), Some(("w", "p")));
        assert_eq!(split_qname("p"), Some(("", "p")));
        assert_eq!(split_qname("a:b:c"), None);
        assert_eq!(split_qname(":p"), None);
        assert_eq!(split_qname("1p"), None);
        assert_eq!(split_qname("ü-1.x"), Some(("", "ü-1.x")));
    }

    #[test]
    fn invalid_chars_are_found() {
        assert_eq!(first_invalid_char("ab\u{1}"), Some(2));
        assert_eq!(first_invalid_char("a\u{FFFE}"), Some(1));
        assert_eq!(first_invalid_char("a\t\r\n\u{FFFD}"), None);
    }
}
