//! `tabellion inspect`: an outline of a document, or one of its views.

use std::path::Path;

use std::collections::HashMap;

use nebu_docx::{Block, BlockId, BlockKind, DrawingKind, Inline, NoteKind, Views};
use serde::Serialize;

use crate::View;
use crate::layout::{columns, emit, one_line};

pub(crate) fn run(file: &Path, view: Option<View>, json: bool) -> Result<(), String> {
    let mut doc = crate::open(file)?;
    let fail = |e: nebu_docx::Error| format!("{}: {e}", file.display());
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let sha = doc.sha256();
    let views = doc.views().map_err(fail)?;
    let Some(view) = view else {
        emit(&outline(views, &name));
        return Ok(());
    };
    macro_rules! show {
        ($key:literal, $value:expr, $text:expr) => {{
            let v = $value;
            if json {
                emit(&to_json($key, &sha, &v));
            } else {
                emit(&$text(&v));
            }
        }};
    }
    let v = views;
    match view {
        View::Blocks => show!("blocks", v.blocks(), blocks_text),
        View::Styles => show!("styles", v.styles().iter().collect::<Vec<_>>(), styles_text),
        View::Tables => show!("tables", v.tables(), tables_text),
        View::Fields => show!("fields", v.fields(), fields_text),
        View::Comments => show!("comments", v.comments(), comments_text),
        View::Revisions => show!("revisions", v.revisions(), revisions_text),
        View::Images => show!("images", v.images(), images_text),
        View::Bookmarks => show!("bookmarks", v.bookmarks(), bookmarks_text),
        View::Notes => show!("notes", v.notes(), notes_text),
        View::Sections => show!("sections", v.sections(), sections_text),
        View::Numbering => show!("numbering", v.numbering(), numbering_text),
        View::ContentControls => show!("content_controls", v.content_controls(), controls_text),
    }
    Ok(())
}

/// `{"sha256": …, "<view>": …}`, the shape the schema gives, with fields
/// in the order the views define them.
fn to_json<T: Serialize>(key: &str, sha: &str, value: &T) -> String {
    struct Wrapped<'a, T> {
        key: &'a str,
        sha: &'a str,
        value: &'a T,
    }
    impl<T: Serialize> Serialize for Wrapped<'_, T> {
        fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeMap;
            let mut m = s.serialize_map(Some(2))?;
            m.serialize_entry("sha256", self.sha)?;
            m.serialize_entry(self.key, self.value)?;
            m.end()
        }
    }
    let mut s = serde_json::to_string_pretty(&Wrapped { key, sha, value }).unwrap_or_default();
    s.push('\n');
    s
}

fn opt(s: &Option<String>) -> String {
    s.clone().unwrap_or_default()
}

fn id(b: &Option<BlockId>) -> String {
    b.map(|b| b.to_string()).unwrap_or_default()
}

fn is_caption(b: &Block) -> bool {
    b.kind == BlockKind::Paragraph
        && (b.style_name.as_deref() == Some("Caption")
            || b.content.iter().any(|i| {
                matches!(i, Inline::FieldStart { code, .. }
                    if code.split_whitespace().next().is_some_and(|w| w.eq_ignore_ascii_case("SEQ")))
            }))
}

fn is_figure(b: &Block) -> bool {
    b.content.iter().any(|i| {
        matches!(
            i,
            Inline::Drawing {
                kind: DrawingKind::Picture
                    | DrawingKind::Chart
                    | DrawingKind::Diagram
                    | DrawingKind::Canvas
                    | DrawingKind::Group,
                ..
            }
        )
    })
}

/// The caption next to block `i`: before it first for a table, after it
/// first for a figure.
fn caption(blocks: &[Block], i: usize, before_first: bool) -> Option<String> {
    let prev = i.checked_sub(1).map(|j| &blocks[j]);
    let next = blocks.get(i + 1);
    let order = if before_first {
        [prev, next]
    } else {
        [next, prev]
    };
    order
        .into_iter()
        .flatten()
        .find(|b| is_caption(b))
        .map(|b| b.text.clone())
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn outline(v: &Views, name: &str) -> String {
    let blocks = v.blocks();
    let tables = v.tables();
    let by_id: HashMap<BlockId, &nebu_docx::Table> = tables
        .iter()
        .filter_map(|t| t.id.map(|id| (id, t)))
        .collect();
    let mut rows = vec![];
    for (i, b) in blocks.iter().enumerate() {
        if b.style_name.as_deref() == Some("Title") {
            rows.push(vec!["title".into(), b.id.to_string(), b.text.clone()]);
        } else if let Some(l) = b.level {
            let indent = "  ".repeat(usize::from(l.saturating_sub(1)));
            rows.push(vec![
                format!("heading {l}"),
                b.id.to_string(),
                format!("{indent}{}", b.text),
            ]);
        } else if b.kind == BlockKind::Table {
            let size = by_id
                .get(&b.id)
                .map(|t| format!(" ({} × {})", t.row_count, t.column_count))
                .unwrap_or_default();
            let cap = caption(blocks, i, true).unwrap_or_else(|| "(no caption)".into());
            rows.push(vec![
                "table".into(),
                b.id.to_string(),
                format!("{cap}{size}"),
            ]);
        } else if is_figure(b) {
            let cap = caption(blocks, i, false)
                .or_else(|| is_caption(b).then(|| b.text.clone()))
                .unwrap_or_else(|| "(no caption)".into());
            rows.push(vec!["figure".into(), b.id.to_string(), cap]);
        }
    }
    let mut out = format!(
        "{name}: {} in {}\n\n",
        plural(blocks.len(), "block", "blocks"),
        plural(v.sections().len(), "section", "sections")
    );
    if rows.is_empty() {
        out.push_str("No headings, tables or figures.\n");
    } else {
        out.push_str(&columns(&["KIND", "ID", "TEXT"], &rows));
    }
    let notes = v.notes();
    let footnotes = notes
        .iter()
        .filter(|n| n.kind == NoteKind::Footnote)
        .count();
    let endnotes = notes.len() - footnotes;
    let counts = [
        plural(tables.len(), "table", "tables"),
        plural(v.images().len(), "image", "images"),
        plural(v.fields().len(), "field", "fields"),
        plural(v.revisions().len(), "revision", "revisions"),
        plural(v.comments().len(), "comment", "comments"),
        plural(footnotes, "footnote", "footnotes"),
        plural(endnotes, "endnote", "endnotes"),
        plural(v.bookmarks().len(), "bookmark", "bookmarks"),
    ];
    out.push('\n');
    out.push_str(&counts.join(", "));
    out.push('\n');
    // Blocks that share a paragraph id have numbered ids, which hold only
    // for this file.
    let shared: Vec<String> = blocks
        .iter()
        .filter(|b| b.shared_para_id)
        .map(|b| b.id.to_string())
        .collect();
    if !shared.is_empty() {
        out.push_str(&format!(
            "\n{} share a paragraph id with another block and are numbered instead: {}\n",
            plural(shared.len(), "block", "blocks"),
            shared.join(", ")
        ));
    }
    out
}

fn blocks_text(blocks: &[Block]) -> String {
    let rows: Vec<Vec<String>> = blocks
        .iter()
        .map(|b| {
            vec![
                b.index.to_string(),
                b.id.to_string(),
                kind_name(b.kind).into(),
                opt(&b.style_name),
                b.level.map(|l| l.to_string()).unwrap_or_default(),
                match b.kind {
                    BlockKind::Other | BlockKind::Bookmark if b.text.is_empty() => opt(&b.element),
                    _ => b.text.clone(),
                },
            ]
        })
        .collect();
    columns(&["#", "ID", "KIND", "STYLE", "LEVEL", "TEXT"], &rows)
}

fn kind_name(k: BlockKind) -> &'static str {
    match k {
        BlockKind::Paragraph => "paragraph",
        BlockKind::Table => "table",
        BlockKind::ContentControl => "control",
        BlockKind::Section => "section",
        BlockKind::Bookmark => "bookmark",
        BlockKind::Other => "other",
    }
}

fn styles_text(styles: &[&nebu_docx::Style]) -> String {
    let rows: Vec<Vec<String>> = styles
        .iter()
        .map(|s| {
            let kind = match s.kind {
                nebu_docx::StyleKind::Paragraph => "paragraph",
                nebu_docx::StyleKind::Character => "character",
                nebu_docx::StyleKind::Table => "table",
                nebu_docx::StyleKind::Numbering => "numbering",
            };
            let local = s
                .local_names
                .iter()
                .map(|l| format!("{}: {}", l.language, l.name))
                .collect::<Vec<_>>()
                .join(", ");
            vec![
                s.id.clone(),
                kind.into(),
                s.display_name.clone(),
                if s.built_in { "yes" } else { "" }.into(),
                opt(&s.based_on),
                local,
            ]
        })
        .collect();
    columns(
        &["ID", "KIND", "NAME", "BUILT-IN", "BASED ON", "OTHER NAMES"],
        &rows,
    )
}

fn tables_text(tables: &[nebu_docx::Table]) -> String {
    let mut out = String::new();
    for t in tables {
        let place = match (&t.id, &t.parent) {
            (Some(id), _) => id.to_string(),
            (None, Some(p)) => format!(
                "in table {}, row {}, column {}",
                p.table + 1,
                p.row + 1,
                p.column + 1
            ),
            (None, None) => format!("in a {:?} story", t.location.story).to_lowercase(),
        };
        out.push_str(&format!(
            "Table {}: {place}, {} rows, {} columns{}\n",
            t.index + 1,
            t.row_count,
            t.column_count,
            t.style
                .as_ref()
                .map(|s| format!(", style {s}"))
                .unwrap_or_default()
        ));
        out.push_str(&grid(&t.rows));
        out.push('\n');
    }
    if tables.is_empty() {
        out.push_str("No tables.\n");
    }
    out
}

/// A table's grid with aligned columns. A merged cell's text stands at its
/// top left; the positions it covers are blank.
fn grid(rows: &[Vec<Option<String>>]) -> String {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let cell = |c: &Option<String>| crate::layout::cut(&one_line(c.as_deref().unwrap_or("")), 24);
    let mut w = vec![0; cols];
    for r in rows {
        for (i, c) in r.iter().enumerate() {
            w[i] = w[i].max(cell(c).chars().count());
        }
    }
    let mut out = String::new();
    for r in rows {
        let mut line = String::from("  ");
        for (i, c) in r.iter().enumerate() {
            let t = cell(c);
            line.push_str(&t);
            line.push_str(&" ".repeat(w[i] - t.chars().count() + 2));
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

fn fields_text(fields: &[nebu_docx::Field]) -> String {
    let rows: Vec<Vec<String>> = fields
        .iter()
        .map(|f| {
            vec![
                f.index.to_string(),
                f.kind.clone(),
                f.code.clone(),
                f.result.clone(),
            ]
        })
        .collect();
    columns(&["#", "KIND", "CODE", "RESULT"], &rows)
}

fn comments_text(comments: &[nebu_docx::Comment]) -> String {
    let rows: Vec<Vec<String>> = comments
        .iter()
        .map(|c| {
            vec![
                c.id.clone(),
                c.author.clone(),
                c.parent
                    .as_ref()
                    .map(|p| format!("reply to {p}"))
                    .unwrap_or_default(),
                opt(&c.anchor),
                c.text.clone(),
            ]
        })
        .collect();
    columns(&["ID", "AUTHOR", "REPLY", "ABOUT", "TEXT"], &rows)
}

fn revisions_text(revs: &[nebu_docx::Revision]) -> String {
    let rows: Vec<Vec<String>> = revs
        .iter()
        .map(|r| {
            let v = serde_json::to_value((r.kind, r.target)).unwrap_or_default();
            let word = |i: usize| v[i].as_str().unwrap_or_default().replace('_', " ");
            vec![
                r.index.to_string(),
                word(0),
                word(1),
                r.author.clone(),
                opt(&r.date),
                r.text.clone(),
            ]
        })
        .collect();
    columns(&["#", "KIND", "OF", "AUTHOR", "DATE", "TEXT"], &rows)
}

fn images_text(images: &[nebu_docx::Image]) -> String {
    let rows: Vec<Vec<String>> = images
        .iter()
        .map(|i| {
            vec![
                i.index.to_string(),
                match i.placement {
                    nebu_docx::Placement::Inline => "inline",
                    nebu_docx::Placement::Floating => "floating",
                }
                .into(),
                match (i.width, i.height) {
                    (Some(w), Some(h)) => format!("{w} × {h} pt"),
                    _ => String::new(),
                },
                i.part.clone().or(i.link.clone()).unwrap_or_default(),
                opt(&i.alt),
            ]
        })
        .collect();
    columns(&["#", "PLACEMENT", "SIZE", "PART", "ALT TEXT"], &rows)
}

fn bookmarks_text(marks: &[nebu_docx::Bookmark]) -> String {
    let rows: Vec<Vec<String>> = marks
        .iter()
        .map(|b| {
            vec![
                b.name.clone(),
                if b.hidden { "hidden" } else { "" }.into(),
                id(&b.location.block),
                b.text.clone(),
            ]
        })
        .collect();
    columns(&["NAME", "", "BLOCK", "TEXT"], &rows)
}

fn notes_text(notes: &[nebu_docx::Note]) -> String {
    let rows: Vec<Vec<String>> = notes
        .iter()
        .map(|n| {
            vec![
                match n.kind {
                    NoteKind::Footnote => "footnote",
                    NoteKind::Endnote => "endnote",
                }
                .into(),
                n.number.map(|v| v.to_string()).unwrap_or_default(),
                id(&n.location.as_ref().and_then(|l| l.block)),
                n.text.clone(),
            ]
        })
        .collect();
    columns(&["KIND", "NUMBER", "BLOCK", "TEXT"], &rows)
}

fn sections_text(sections: &[nebu_docx::Section]) -> String {
    let mut out = String::new();
    for s in sections {
        let size = match (s.page_width, s.page_height) {
            (Some(w), Some(h)) => format!("{w} × {h} pt {}", s.orientation),
            _ => s.orientation.clone(),
        };
        out.push_str(&format!(
            "Section {}: blocks {} to {}, starts on {}, {size}, text width {} pt, {}\n",
            s.index + 1,
            id(&s.first_block),
            id(&s.last_block),
            s.start.replace('_', " "),
            s.text_width.map(|w| w.to_string()).unwrap_or_default(),
            plural(s.columns as usize, "column", "columns"),
        ));
        for (what, list) in [("header", &s.headers), ("footer", &s.footers)] {
            for h in list {
                out.push_str(&format!(
                    "  {} {what} ({}{}): {}\n",
                    h.kind,
                    h.part,
                    if h.inherited { ", from before" } else { "" },
                    one_line(&h.text)
                ));
            }
        }
    }
    out
}

fn numbering_text(n: &nebu_docx::Numbering) -> String {
    let mut out = String::new();
    for d in &n.definitions {
        let levels: Vec<String> = d
            .levels
            .iter()
            .take(3)
            .map(|l| format!("{} {}", opt(&l.format), opt(&l.text)))
            .collect();
        out.push_str(&format!(
            "definition {}{}: {}\n",
            d.id,
            d.style
                .as_ref()
                .map(|s| format!(" (style {s})"))
                .unwrap_or_default(),
            levels.join("; ")
        ));
    }
    for i in &n.instances {
        out.push_str(&format!(
            "list {} uses definition {}{}\n",
            i.id,
            opt(&i.definition),
            if i.overrides.is_empty() {
                String::new()
            } else {
                format!(", {} level overrides", i.overrides.len())
            }
        ));
    }
    if out.is_empty() {
        out.push_str("No numbering.\n");
    }
    out
}

fn controls_text(controls: &[nebu_docx::Control]) -> String {
    let rows: Vec<Vec<String>> = controls
        .iter()
        .map(|c| {
            let kind = serde_json::to_value(c.control.kind).unwrap_or_default();
            vec![
                kind.as_str().unwrap_or_default().replace('_', " "),
                opt(&c.control.tag),
                opt(&c.control.title),
                id(&c.location.block),
                if c.control.placeholder {
                    format!("{} (placeholder)", c.text)
                } else {
                    c.text.clone()
                },
            ]
        })
        .collect();
    columns(&["KIND", "TAG", "TITLE", "BLOCK", "TEXT"], &rows)
}
