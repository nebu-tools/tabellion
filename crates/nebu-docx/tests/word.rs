//! The views against what Word reports.
//!
//! Each document in `corpus/` has a reading by Word next to it
//! (`NAME.word.json`, made by `corpus/build.sh read`). These tests compare
//! Tabellion's views with those readings, so they run without Word.
//!
//! Where Tabellion reports a document differently from Word on purpose, the
//! normalisation below names the difference and the reason.

use std::path::{Path, PathBuf};

use nebu_docx::{
    BlockKind, ContentControlKind, Document, Inline, NoteKind, Placement, RevisionKind,
    RevisionTarget, Revisions, StoryKind, TextOptions,
};
use serde_json::Value;

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus")
}

/// Every corpus document with its reading.
fn documents() -> Vec<(String, Document, Value)> {
    let mut out = vec![];
    let mut names: Vec<PathBuf> = std::fs::read_dir(corpus())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "docx"))
        .collect();
    names.sort();
    for p in names {
        let reading = p.with_extension("word.json");
        let reading: Value =
            serde_json::from_slice(&std::fs::read(&reading).unwrap_or_else(|_| {
                panic!(
                    "{} has no Word reading; run corpus/build.sh read",
                    p.display()
                )
            }))
            .unwrap();
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        out.push((name, Document::open(&p).unwrap(), reading));
    }
    assert!(out.len() >= 5, "the corpus is incomplete");
    out
}

/// Word's text with its control characters made plain.
///
/// Word writes a paragraph end as `\r`, the end of a cell or row as
/// `\r\x07`, a line break as `\x0B`, a page or section break as `\x0C`, a
/// note reference as `\x02`, a comment mark as `\x05`, a non-breaking
/// hyphen as `\x1E`, an optional hyphen as `\x1F` and a legacy picture
/// or object as `\x01`. Tabellion writes line ends, tabs, U+2011 and
/// U+00AD, and no character for a note or comment mark or a picture.
///
/// Word gives the letters of an equation in mathematical italic
/// (U+1D44E for `a`), as Cambria Math draws them; Tabellion gives the
/// letters as the file stores them.
fn plain(word: &str) -> String {
    word.chars()
        .filter_map(|c| match c {
            '\r' | '\u{07}' | '\u{0B}' | '\u{0C}' => Some('\n'),
            '\u{01}' | '\u{02}' | '\u{05}' => None,
            '\u{210E}' => Some('h'),
            '\u{1D434}'..='\u{1D44D}' => char::from_u32(c as u32 - 0x1D434 + 'A' as u32),
            '\u{1D44E}'..='\u{1D467}' => char::from_u32(c as u32 - 0x1D44E + 'a' as u32),
            '\u{1E}' => Some('\u{2011}'),
            '\u{1F}' => Some('\u{AD}'),
            c => Some(c),
        })
        .collect()
}

/// Text as lines without empty ones, cells and paragraphs alike, so that
/// Word's cell marks and Tabellion's tab-separated rows compare.
///
/// Word gives an inline picture as the character `/`, where Tabellion's
/// text has no character for a picture: a `/` that stands alone is left
/// out.
///
/// Word gives a symbol (`w:sym`) as `(`, where Tabellion gives the code the
/// symbol font has for it, often in the private use area from U+F000.
fn lines(s: &str) -> Vec<String> {
    let s: String = s
        .chars()
        .map(|c| {
            if ('\u{F000}'..='\u{F0FF}').contains(&c) {
                '('
            } else {
                c
            }
        })
        .collect();
    s.split(['\n', '\t'])
        .map(|l| {
            let l = l.trim();
            let l = l.strip_suffix(" /").unwrap_or(l);
            l.to_string()
        })
        .filter(|l| !l.is_empty() && l != "/")
        .collect()
}

/// The words of a text, after [`lines`].
fn tokens(s: &str) -> Vec<String> {
    lines(s)
        .iter()
        .flat_map(|l| l.split_whitespace().map(str::to_string).collect::<Vec<_>>())
        .collect()
}

fn diff(what: &str, word: &[String], ours: &[String]) -> Option<String> {
    if word == ours {
        return None;
    }
    let at = word.iter().zip(ours).take_while(|(a, b)| a == b).count();
    Some(format!(
        "{what}: differs at line {at}\n  Word:      {:?}\n  Tabellion: {:?}",
        &word[at.min(word.len())..(at + 3).min(word.len())],
        &ours[at.min(ours.len())..(at + 3).min(ours.len())]
    ))
}

fn check(failures: Vec<String>) {
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

fn story(r: &Value, key: &str, kind: &str) -> Vec<String> {
    r[key][kind]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|v| v.as_str().unwrap_or("").to_string())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn main_text_matches_word() {
    let mut failures = vec![];
    for (name, mut doc, r) in documents() {
        for (view, key) in [
            (Revisions::Current, "stories"),
            (Revisions::Accepted, "accepted"),
            (Revisions::Rejected, "rejected"),
        ] {
            let word = story(&r, key, "1");
            // Word saves the stories it changed when accepting; a document
            // without revisions has only the first reading.
            let word = if word.is_empty() {
                story(&r, "stories", "1")
            } else {
                word
            };
            let ours = doc
                .text(&TextOptions {
                    revisions: view,
                    text_boxes: false,
                    ..TextOptions::default()
                })
                .unwrap();
            // Tabellion puts the cells of a nested table into the text of
            // the outer cell, separated by spaces, so that a row stays one
            // line. The comparison is therefore word by word.
            let w = tokens(&plain(&word.concat()));
            if let Some(d) = diff(&format!("{name}, {key}"), &w, &tokens(&ours)) {
                failures.push(d);
            }
        }
    }
    check(failures);
}

/// Word's cells of one table: row, the cell's position in its row (Word's
/// `ColumnIndex`), text, nested tables.
fn word_cells(t: &Value) -> Vec<(usize, usize, String, Vec<Value>)> {
    t["cells"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["row"].as_u64().unwrap() as usize,
                c["column"].as_u64().unwrap() as usize,
                c["text"].as_str().unwrap_or("").to_string(),
                c["tables"].as_array().cloned().unwrap_or_default(),
            )
        })
        .collect()
}

fn compare_table(
    name: &str,
    word: &Value,
    tables: &[nebu_docx::Table],
    i: usize,
    failures: &mut Vec<String>,
) {
    let t = &tables[i];
    for (row, column, text, nested) in word_cells(word) {
        // Word numbers the cells of a row by position, counting a cell that
        // a vertical merge carries down from above.
        let mut in_row: Vec<&nebu_docx::Cell> = t
            .cells
            .iter()
            .filter(|c| c.row < row && c.row + c.row_span >= row)
            .collect();
        in_row.sort_by_key(|c| c.column);
        let Some(cell) = in_row.get(column - 1) else {
            failures.push(format!(
                "{name}, table {i}: no cell at row {row}, position {column}"
            ));
            continue;
        };
        if cell.row + 1 != row {
            // A continuation of a vertical merge; Word gives it as empty.
            if !plain(&text).trim().is_empty() {
                failures.push(format!(
                    "{name}, table {i}: row {row} cell {column} should be empty"
                ));
            }
            continue;
        }
        if nested.is_empty() {
            let w = plain(&text).trim_end_matches('\n').to_string();
            let w = w.strip_suffix("\n").unwrap_or(&w).to_string();
            if lines(&w) != lines(&cell.text) {
                failures.push(format!(
                    "{name}, table {i}, row {row}, cell {column}: Word {w:?}, Tabellion {:?}",
                    cell.text
                ));
            }
        }
        for (k, n) in nested.iter().enumerate() {
            match cell.tables.get(k) {
                Some(&j) => compare_table(name, n, tables, j, failures),
                None => failures.push(format!("{name}, table {i}: a nested table is missing")),
            }
        }
    }
}

#[test]
fn tables_match_word_with_revisions_accepted() {
    let mut failures = vec![];
    for (name, mut doc, r) in documents() {
        let tables = doc.tables().unwrap();
        let top: Vec<usize> = tables
            .iter()
            .filter(|t| t.location.story == StoryKind::Main && t.parent.is_none())
            .map(|t| t.index)
            .collect();
        let key = if r["accepted_tables"].is_array() {
            "accepted_tables"
        } else {
            "tables"
        };
        let word = r[key].as_array().cloned().unwrap_or_default();
        if word.len() != top.len() {
            failures.push(format!(
                "{name}: Word has {} tables, Tabellion {}",
                word.len(),
                top.len()
            ));
            continue;
        }
        for (w, &i) in word.iter().zip(&top) {
            compare_table(&name, w, &tables, i, &mut failures);
        }
    }
    check(failures);
}

/// A field code as Word gives it: nested fields between `\x13` and `\x15`,
/// which Tabellion writes as `{` and `}`.
fn word_code(s: &str) -> String {
    s.replace('\u{13}', "{")
        .replace('\u{15}', "}")
        .trim()
        .to_string()
}

#[test]
fn fields_match_word() {
    let mut failures = vec![];
    for (name, mut doc, r) in documents() {
        let fields = doc.fields().unwrap();
        for (story, kinds) in [
            (1, &[StoryKind::Main][..]),
            (2, &[StoryKind::Footnote]),
            (3, &[StoryKind::Endnote]),
        ] {
            let word: Vec<(String, Vec<String>)> = r["fields"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|f| f["story"] == story)
                .map(|f| {
                    (
                        word_code(f["code"].as_str().unwrap_or("")),
                        lines(&plain(f["result"].as_str().unwrap_or(""))),
                    )
                })
                .collect();
            let ours: Vec<(String, Vec<String>)> = fields
                .iter()
                .filter(|f| kinds.contains(&f.location.story))
                .map(|f| (f.code.clone(), lines(&f.result)))
                .collect();
            // Word lists a hyperlink element (w:hyperlink) among its fields
            // as a HYPERLINK field. Tabellion reports it as a hyperlink in
            // the paragraph's content, not as a field.
            let word: Vec<(String, Vec<String>)> = word
                .into_iter()
                .filter(|(code, _)| {
                    !code.starts_with("HYPERLINK") || ours.iter().any(|(c, _)| c == code)
                })
                .collect();
            if word != ours {
                failures.push(format!(
                    "{name}, story {story}:\n  Word:      {word:?}\n  Tabellion: {ours:?}"
                ));
            }
        }
        // Headers and footers: Word lists a header's fields once per
        // section that shows it; compare the codes as a set.
        let mut word: Vec<String> = r["fields"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| (6..=11).contains(&f["story"].as_i64().unwrap_or(0)))
            .map(|f| word_code(f["code"].as_str().unwrap_or("")))
            .collect();
        let mut ours: Vec<String> = fields
            .iter()
            .filter(|f| matches!(f.location.story, StoryKind::Header | StoryKind::Footer))
            .map(|f| f.code.clone())
            .collect();
        word.sort();
        word.dedup();
        ours.sort();
        ours.dedup();
        if word != ours {
            failures.push(format!(
                "{name}, headers and footers:\n  Word:      {word:?}\n  Tabellion: {ours:?}"
            ));
        }
    }
    check(failures);
}

/// Word's `WdRevisionType` for a revision.
fn word_type(kind: RevisionKind, target: RevisionTarget) -> i64 {
    match (kind, target) {
        (RevisionKind::Insertion, _) => 1,
        (RevisionKind::Deletion, _) => 2,
        (RevisionKind::MoveFrom, _) => 14,
        (RevisionKind::MoveTo, _) => 15,
        (RevisionKind::Formatting, RevisionTarget::Run) => 3,
        (RevisionKind::Formatting, RevisionTarget::Paragraph) => 10,
        (RevisionKind::Formatting, RevisionTarget::Section) => 12,
        (RevisionKind::Formatting, _) => 11,
    }
}

fn words(s: &str) -> String {
    lines(s).join(" ")
}

#[test]
fn revisions_match_word() {
    let mut failures = vec![];
    for (name, mut doc, r) in documents() {
        let word: Vec<(i64, String, String)> = r["revisions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["story"] == 1)
            .map(|v| {
                (
                    v["type"].as_i64().unwrap(),
                    v["author"].as_str().unwrap().to_string(),
                    words(&plain(v["text"].as_str().unwrap_or(""))),
                )
            })
            .collect();
        let ours: Vec<(i64, String, String)> = doc
            .revisions()
            .unwrap()
            .iter()
            .filter(|v| v.location.story == StoryKind::Main)
            .map(|v| {
                (
                    word_type(v.kind, v.target),
                    v.author.clone(),
                    words(&v.text),
                )
            })
            .collect();
        if word != ours {
            failures.push(format!(
                "{name}:\n  Word:      {word:#?}\n  Tabellion: {ours:#?}"
            ));
        }
    }
    check(failures);
}

#[test]
fn comments_match_word() {
    let mut failures = vec![];
    for (name, mut doc, r) in documents() {
        let comments = doc.comments().unwrap();
        let word: Vec<(String, String, String, String, Option<usize>)> = r["comments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                (
                    c["author"].as_str().unwrap().to_string(),
                    c["initials"].as_str().unwrap_or("").to_string(),
                    words(&plain(c["text"].as_str().unwrap_or(""))),
                    words(&plain(c["scope"].as_str().unwrap_or(""))),
                    c["parent"].as_u64().map(|p| p as usize - 1),
                )
            })
            .collect();
        let ours: Vec<(String, String, String, String, Option<usize>)> = comments
            .iter()
            .map(|c| {
                (
                    c.author.clone(),
                    c.initials.clone().unwrap_or_default(),
                    words(&c.text),
                    words(c.anchor.as_deref().unwrap_or("")),
                    c.parent
                        .as_ref()
                        .and_then(|p| comments.iter().position(|x| x.id == *p)),
                )
            })
            .collect();
        if word != ours {
            failures.push(format!(
                "{name}:\n  Word:      {word:?}\n  Tabellion: {ours:?}"
            ));
        }
    }
    check(failures);
}

#[test]
fn bookmarks_match_word() {
    let mut failures = vec![];
    for (name, mut doc, r) in documents() {
        let mut word: Vec<(String, String)> = r["bookmarks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| {
                (
                    b["name"].as_str().unwrap().to_string(),
                    words(&plain(b["text"].as_str().unwrap_or(""))),
                )
            })
            .collect();
        let mut ours: Vec<(String, String)> = doc
            .bookmarks()
            .unwrap()
            .iter()
            .map(|b| (b.name.clone(), words(&b.text)))
            .collect();
        word.sort();
        ours.sort();
        if word != ours {
            failures.push(format!(
                "{name}:\n  Word:      {word:?}\n  Tabellion: {ours:?}"
            ));
        }
    }
    check(failures);
}

#[test]
fn notes_match_word() {
    let mut failures = vec![];
    for (name, mut doc, r) in documents() {
        let notes = doc.notes().unwrap();
        for (key, kind) in [
            ("footnotes", NoteKind::Footnote),
            ("endnotes", NoteKind::Endnote),
        ] {
            let word: Vec<(usize, String)> = r[key]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|n| {
                            (
                                n["index"].as_u64().unwrap() as usize,
                                words(&plain(n["text"].as_str().unwrap_or(""))),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            // Word's index counts every note, one with its own mark too;
            // Tabellion's number is the one Word shows, which such a note
            // has not. Both lists are in the order of the references.
            let ours: Vec<(usize, String)> = notes
                .iter()
                .filter(|n| n.kind == kind && n.location.is_some())
                .enumerate()
                .map(|(i, n)| (i + 1, words(&n.text)))
                .collect();
            if word != ours {
                failures.push(format!(
                    "{name}, {key}:\n  Word:      {word:?}\n  Tabellion: {ours:?}"
                ));
            }
        }
    }
    check(failures);
}

#[test]
fn headers_and_footers_match_word() {
    let mut failures = vec![];
    for (name, mut doc, r) in documents() {
        let sections = doc.sections().unwrap();
        let word = r["sections"].as_array().cloned().unwrap_or_default();
        if word.len() != sections.len() {
            failures.push(format!(
                "{name}: Word has {} sections, Tabellion {}",
                word.len(),
                sections.len()
            ));
            continue;
        }
        for (w, s) in word.iter().zip(&sections) {
            for (key, list) in [("headers", &s.headers), ("footers", &s.footers)] {
                for h in w[key].as_array().unwrap() {
                    let kind = match h["kind"].as_i64() {
                        Some(1) => "default",
                        Some(2) => "first",
                        _ => "even",
                    };
                    let exists = h["exists"].as_bool().unwrap_or(false);
                    let linked = h["linked"].as_bool().unwrap_or(false);
                    let ours = list.iter().find(|x| x.kind == kind);
                    let text = words(&plain(h["text"].as_str().unwrap_or("")));
                    match ours {
                        Some(x) if exists || linked => {
                            if words(&x.text) != text || (exists && x.inherited != linked) {
                                failures.push(format!(
                                    "{name}, section {}, {kind} {key}: Word {text:?} (linked {linked}), Tabellion {:?} (inherited {})",
                                    s.index, x.text, x.inherited
                                ));
                            }
                        }
                        // Word reports an empty default header for a section
                        // that has none.
                        None if exists && !text.is_empty() => failures.push(format!(
                            "{name}, section {}: no {kind} {key}, Word has {text:?}",
                            s.index
                        )),
                        _ => {}
                    }
                }
            }
        }
    }
    check(failures);
}

#[test]
fn text_boxes_match_word() {
    let mut failures = vec![];
    for (name, mut doc, r) in documents() {
        let word: Vec<String> = story(&r, "stories", "5")
            .iter()
            .map(|t| words(&plain(t)))
            .collect();
        let ours: Vec<String> = doc
            .blocks()
            .unwrap()
            .iter()
            .flat_map(|b| b.content.clone())
            .filter_map(|i| match i {
                Inline::Drawing { text: Some(t), .. } => Some(words(&t)),
                _ => None,
            })
            .collect();
        if word != ours {
            failures.push(format!(
                "{name}:\n  Word:      {word:?}\n  Tabellion: {ours:?}"
            ));
        }
    }
    check(failures);
}

#[test]
fn images_match_word() {
    let mut failures = vec![];
    for (name, mut doc, r) in documents() {
        let images = doc.images().unwrap();
        let shape = |s: &Value| {
            (
                s["alt"].as_str().unwrap_or("").to_string(),
                s["title"].as_str().unwrap_or("").to_string(),
                s["width"].as_f64().unwrap_or(0.0).round() as i64,
                s["height"].as_f64().unwrap_or(0.0).round() as i64,
            )
        };
        let ours = |p: Placement| -> Vec<(String, String, i64, i64)> {
            images
                .iter()
                .filter(|i| i.placement == p && i.location.story == StoryKind::Main)
                .map(|i| {
                    (
                        i.alt.clone().unwrap_or_default(),
                        i.title.clone().unwrap_or_default(),
                        i.width.unwrap_or(0.0).round() as i64,
                        i.height.unwrap_or(0.0).round() as i64,
                    )
                })
                .collect()
        };
        // Inline pictures are Word's inline shapes of type 3; floating ones
        // its shapes of type 13.
        let inline: Vec<_> = r["inline_shapes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["type"] == 3)
            .map(shape)
            .collect();
        let floating: Vec<_> = r["shapes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["type"] == 13)
            .map(shape)
            .collect();
        for (what, w, o) in [
            ("inline", inline, ours(Placement::Inline)),
            ("floating", floating, ours(Placement::Floating)),
        ] {
            if w != o {
                failures.push(format!(
                    "{name}, {what}:\n  Word:      {w:?}\n  Tabellion: {o:?}"
                ));
            }
        }
    }
    check(failures);
}

/// Word's `WdContentControlType`.
fn control_type(k: ContentControlKind) -> i64 {
    match k {
        ContentControlKind::RichText => 0,
        ContentControlKind::PlainText => 1,
        ContentControlKind::Picture => 2,
        ContentControlKind::ComboBox => 3,
        ContentControlKind::DropDownList => 4,
        ContentControlKind::BuildingBlock => 5,
        ContentControlKind::Date => 6,
        ContentControlKind::Group => 7,
        ContentControlKind::CheckBox => 8,
        ContentControlKind::RepeatingSection => 9,
        _ => -1,
    }
}

#[test]
fn content_controls_match_word() {
    let mut failures = vec![];
    for (name, mut doc, r) in documents() {
        let word: Vec<(i64, String, String, bool)> = r["content_controls"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                (
                    c["type"].as_i64().unwrap(),
                    c["tag"].as_str().unwrap_or("").to_string(),
                    c["title"].as_str().unwrap_or("").to_string(),
                    c["placeholder"].as_bool().unwrap_or(false),
                )
            })
            .collect();
        let mut ours: Vec<(i64, String, String, bool)> = doc
            .content_controls()
            .unwrap()
            .iter()
            .filter(|c| c.location.story == StoryKind::Main)
            .map(|c| {
                let c = &c.control;
                (
                    control_type(c.kind),
                    c.tag.clone().unwrap_or_default(),
                    c.title.clone().unwrap_or_default(),
                    c.placeholder,
                )
            })
            .collect();
        // A repeating section holds items that Word does not list as
        // controls of their own.
        ours.retain(|c| c.0 != -1);
        if word != ours {
            failures.push(format!(
                "{name}:\n  Word:      {word:?}\n  Tabellion: {ours:?}"
            ));
        }
    }
    check(failures);
}

#[test]
fn built_in_style_names_match_word() {
    let mut failures = vec![];
    for (name, mut doc, r) in documents() {
        let word: Vec<String> = r["styles"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["builtin"].as_bool().unwrap_or(false))
            .map(|s| s["name"].as_str().unwrap().to_string())
            .collect();
        let styles = doc.styles().unwrap();
        // Word lists the styles in use. A built-in style it lists under a
        // name that differs only in case is a wrong display name.
        for s in styles.iter().filter(|s| s.built_in) {
            if let Some(w) = word
                .iter()
                .find(|w| w.eq_ignore_ascii_case(&s.display_name))
                && *w != s.display_name
            {
                failures.push(format!(
                    "{name}: Word shows {w:?}, Tabellion {:?}",
                    s.display_name
                ));
            }
        }
        // Every style a paragraph uses is one Word lists, by the same name.
        for b in doc.blocks().unwrap() {
            if let Some(n) = &b.style_name
                && styles.find(n).is_some_and(|s| s.built_in)
                && !word.contains(n)
            {
                failures.push(format!(
                    "{name}: block {} has style {n:?}, which Word does not list",
                    b.id
                ));
            }
        }
    }
    check(failures);
}

#[test]
fn every_block_has_an_id_and_text_matches_its_content() {
    for (name, mut doc, _) in documents() {
        let blocks = doc.blocks().unwrap();
        let mut ids: Vec<&str> = blocks.iter().map(|b| b.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), blocks.len(), "{name}: block ids repeat");
        for b in blocks.iter().filter(|b| b.kind == BlockKind::Paragraph) {
            // Paragraph text is the visible text of its content plus
            // field results, which content holds as text too.
            let from_content: String = b
                .content
                .iter()
                .filter_map(|i| match i {
                    Inline::Text {
                        text,
                        revision,
                        hidden: false,
                        ..
                    } if !revision.as_ref().is_some_and(|r| {
                        matches!(r.kind, RevisionKind::Deletion | RevisionKind::MoveFrom)
                    }) =>
                    {
                        Some(text.as_str())
                    }
                    _ => None,
                })
                .collect();
            if !b
                .content
                .iter()
                .any(|i| matches!(i, Inline::Math { .. } | Inline::FieldStart { .. }))
            {
                assert_eq!(
                    b.text.trim_end_matches('\n'),
                    from_content.trim_end_matches('\n'),
                    "{name}: block {}",
                    b.id
                );
            }
        }
    }
}
