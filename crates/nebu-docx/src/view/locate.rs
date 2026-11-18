//! Block ids and locators: how a person or a program names a place in the
//! body.

use std::fmt;

use serde::Serialize;

use crate::Error;

use super::props::parse_para_id;

/// The id of a top-level block of the body.
///
/// A paragraph with a `w14:paraId` is `p:` and the id in eight capital
/// hexadecimal digits, a table whose first paragraph has one `t:` and that
/// id; these survive a save by Word. Any other block is `#` and its index,
/// which holds only for the file it was read from. Paragraph ids compare as
/// Word compares them, as numbers: `p:4956fe` and `p:004956FE` are one id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BlockId {
    /// A paragraph by its `w14:paraId`.
    Paragraph(u32),
    /// A table by the `w14:paraId` of its first paragraph.
    Table(u32),
    /// A block by its position among the blocks, from 0.
    Index(usize),
}

impl fmt::Display for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BlockId::Paragraph(n) => write!(f, "p:{n:08X}"),
            BlockId::Table(n) => write!(f, "t:{n:08X}"),
            BlockId::Index(n) => write!(f, "#{n}"),
        }
    }
}

impl Serialize for BlockId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl std::str::FromStr for BlockId {
    type Err = Error;
    /// Reads `p:HEX`, `t:HEX` or `#INDEX`.
    fn from_str(s: &str) -> Result<BlockId, Error> {
        let bad = || {
            Error::Invalid(format!(
                "{s:?} is not a block id; write p:ID or t:ID with up to eight hexadecimal digits below 80000000, or #INDEX"
            ))
        };
        if let Some(n) = s.strip_prefix('#') {
            if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
                return Err(bad());
            }
            return n.parse().map(BlockId::Index).map_err(|_| bad());
        }
        let (kind, hex) = s.split_once(':').ok_or_else(bad)?;
        let n = parse_para_id(hex).ok_or_else(bad)?;
        match kind {
            "p" | "P" => Ok(BlockId::Paragraph(n)),
            "t" | "T" => Ok(BlockId::Table(n)),
            _ => Err(bad()),
        }
    }
}

/// A place in the body, as a person names it.
///
/// A locator is made by parsing (`"heading:Results".parse()`) or by one of
/// the constructors, which refuse an empty or malformed name, so every
/// locator names something that can be looked for. [`Views::locate`]
/// gives the one block it names, or says that it names none or several.
///
/// [`Views::locate`]: super::Views::locate
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locator(pub(crate) Place);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Place {
    Block(BlockId),
    Bookmark(String),
    Heading(String),
    Placeholder(String),
    Text(String),
}

fn checked(what: &str, s: &str) -> Result<String, Error> {
    if s.trim().is_empty() {
        return Err(Error::Invalid(format!("a {what} locator needs a {what}")));
    }
    if s.chars().any(|c| c.is_control() && c != '\t') {
        return Err(Error::Invalid(format!(
            "{s:?} holds a control character, which no {what} has"
        )));
    }
    Ok(s.to_string())
}

impl Locator {
    /// The block with this id.
    pub fn block(id: BlockId) -> Locator {
        Locator(Place::Block(id))
    }

    /// The block where the bookmark of this name starts, or the block after
    /// it for a bookmark between blocks. Word compares bookmark names
    /// without case.
    pub fn bookmark(name: &str) -> Result<Locator, Error> {
        let n = checked("bookmark", name)?;
        if n.chars().any(char::is_whitespace) {
            return Err(Error::Invalid(format!(
                "{name:?} is not a bookmark name; bookmark names have no spaces"
            )));
        }
        Ok(Locator(Place::Bookmark(n)))
    }

    /// The heading whose text, without surrounding white space, is this.
    pub fn heading(text: &str) -> Result<Locator, Error> {
        Ok(Locator(Place::Heading(
            checked("heading", text)?.trim().to_string(),
        )))
    }

    /// The paragraph that holds `{{name}}`.
    pub fn placeholder(name: &str) -> Result<Locator, Error> {
        let n = checked("placeholder", name)?;
        if n.contains(['{', '}']) {
            return Err(Error::Invalid(format!(
                "{name:?} is not a placeholder name; write it without braces"
            )));
        }
        Ok(Locator(Place::Placeholder(n)))
    }

    /// The one block whose text contains this.
    pub fn text(text: &str) -> Result<Locator, Error> {
        Ok(Locator(Place::Text(checked("text", text)?)))
    }
}

impl std::str::FromStr for Locator {
    type Err = Error;
    /// Reads `p:…`, `t:…`, `#…`, `bookmark:…`, `heading:…`,
    /// `placeholder:…` or `text:…`.
    fn from_str(s: &str) -> Result<Locator, Error> {
        if s.starts_with("p:") || s.starts_with("t:") || s.starts_with('#') {
            return Ok(Locator::block(s.parse()?));
        }
        let Some((k, v)) = s.split_once(':') else {
            return Err(Error::Invalid(format!(
                "{s:?} names no place; write p:ID, t:ID, #INDEX, bookmark:NAME, heading:TEXT, placeholder:NAME or text:TEXT"
            )));
        };
        match k {
            "bookmark" => Locator::bookmark(v),
            "heading" => Locator::heading(v),
            "placeholder" => Locator::placeholder(v),
            "text" => Locator::text(v),
            "block" => Ok(Locator::block(v.parse()?)),
            _ => Err(Error::Invalid(format!(
                "{k:?} is not a kind of place; use bookmark, heading, placeholder or text"
            ))),
        }
    }
}

impl fmt::Display for Locator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Place::Block(id) => write!(f, "the block id {id}"),
            Place::Bookmark(n) => write!(f, "the bookmark {n:?}"),
            Place::Heading(t) => write!(f, "the heading {t:?}"),
            Place::Placeholder(n) => write!(f, "the placeholder {{{{{n}}}}}"),
            Place::Text(t) => write!(f, "the text {t:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_ids_compare_as_numbers() {
        let a: BlockId = "p:4956fe".parse().unwrap();
        let b: BlockId = "p:004956FE".parse().unwrap();
        assert_eq!(a, b);
        assert_eq!(a.to_string(), "p:004956FE");
        assert_eq!("#12".parse::<BlockId>().unwrap(), BlockId::Index(12));
        for bad in [
            "p:",
            "p:80000000",
            "p:12345678A",
            "p:xyz",
            "#",
            "#-1",
            "q:1",
        ] {
            assert!(bad.parse::<BlockId>().is_err(), "{bad}");
        }
    }

    #[test]
    fn locators_refuse_empty_and_malformed_names() {
        for bad in [
            "heading:",
            "heading:  ",
            "bookmark:",
            "bookmark:two words",
            "placeholder:{{x}}",
            "text:",
            "text:a\u{7}b",
            "p:",
            "#x",
            "street:1",
            "nothing",
        ] {
            assert!(bad.parse::<Locator>().is_err(), "{bad}");
        }
        assert!(Locator::heading("").is_err());
        assert_eq!(
            "heading: Results ".parse::<Locator>().unwrap(),
            Locator::heading("Results").unwrap()
        );
    }
}
