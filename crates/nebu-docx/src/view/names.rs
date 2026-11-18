//! The names Word shows for its built-in styles.
//!
//! Word stores a built-in style under an English name in lower or mixed
//! case (`heading 1`, `annotation text`) whatever the language of its
//! interface, and shows it under another: `Heading 1` and `Comment Text` in
//! English. The tables come from Word: `tools/word/style-names.ps1` asks a
//! Word for the name it shows (`Style.NameLocal`) of every built-in style
//! and writes `names/<language>.tsv` for the language of its interface.
//! Each line of a table is the stored name, a tab and the name Word shows.
//! The English table gives the display name; any other table gives the
//! names in [`super::LocalName`].

use std::collections::HashMap;
use std::sync::OnceLock;

use super::LocalName;

/// The tables by language tag, English first.
pub(crate) const TABLES: &[(&str, &str)] = &[("en", include_str!("names/en.tsv"))];

/// A built-in style's names.
#[derive(Debug)]
pub(crate) struct Row {
    /// The name Word shows in English.
    pub display: &'static str,
    /// The names in the other languages, in the order of [`TABLES`].
    locals: Vec<LocalName>,
}

impl Row {
    pub fn local_names(&self) -> Vec<LocalName> {
        self.locals.clone()
    }
}

/// The lines of a table: stored name and shown name. Lines that start
/// with `#` are comments.
pub(crate) fn lines(table: &'static str) -> impl Iterator<Item = (&'static str, &'static str)> {
    table
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('\t'))
}

fn rows() -> &'static HashMap<String, Row> {
    static ROWS: OnceLock<HashMap<String, Row>> = OnceLock::new();
    ROWS.get_or_init(|| {
        let mut out: HashMap<String, Row> = HashMap::new();
        for (k, (language, table)) in TABLES.iter().enumerate() {
            for (stored, shown) in lines(table) {
                let key = stored.to_lowercase();
                if k == 0 {
                    out.entry(key).or_insert(Row {
                        display: shown,
                        locals: vec![],
                    });
                } else if let Some(row) = out.get_mut(&key) {
                    row.locals.push(LocalName {
                        language,
                        name: shown,
                    });
                }
            }
        }
        out
    })
}

/// The row for a stored name, compared without case.
pub(crate) fn built_in(stored: &str) -> Option<&'static Row> {
    rows().get(&stored.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table the engine uses is the file the script wrote, line for
    /// line: every line gives the names of one built-in style.
    #[test]
    fn the_table_is_the_generated_file() {
        for (language, table) in TABLES {
            assert!(
                table.starts_with(
                    "# The names Word shows for its built-in styles in display language "
                ),
                "{language}: not written by tools/word/style-names.ps1"
            );
            let mut stored = std::collections::HashSet::new();
            let mut count = 0;
            for line in table.lines().filter(|l| !l.starts_with('#')) {
                let (s, shown) = line.split_once('\t').expect("a tab on every line");
                assert!(
                    !s.is_empty() && !shown.is_empty() && !shown.contains('\t'),
                    "{line:?}"
                );
                assert!(stored.insert(s.to_lowercase()), "{s} twice");
                if *language == "en" {
                    assert_eq!(built_in(s).map(|r| r.display), Some(shown), "{s}");
                }
                count += 1;
            }
            assert_eq!(lines(table).count(), count);
            // Word 2021 has 376 built-in styles.
            assert!(count >= 370, "{language}: only {count} styles");
        }
    }
}
