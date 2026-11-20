//! Tracked and untracked text replacement inside one paragraph.

mod support;

use nebu_docx::content::Content;
use nebu_docx::revise::{replace_all, replace_in_paragraph};
use nebu_docx::xml::NodeId;
use nebu_docx::{Document, Place, RevisionKind, Revisions, TextOptions, Track, WriteOptions};
use serde_json::{Value, json};

fn insert(bytes: Vec<u8>, v: Value) -> (Document, Vec<u8>) {
    let content = Content::from_json(v.to_string().as_bytes()).unwrap();
    let mut doc = Document::from_bytes(bytes).unwrap();
    let opts = WriteOptions::for_content(&content, None);
    doc.insert(Place::End, &content.blocks, &opts).unwrap();
    let out = doc.to_bytes().unwrap();
    (Document::from_bytes(out.clone()).unwrap(), out)
}

/// The first paragraph whose text contains the needle.
fn paragraph_with(doc: &mut Document, needle: &str) -> NodeId {
    doc.views()
        .unwrap()
        .paragraphs()
        .iter()
        .find(|p| p.text.contains(needle))
        .unwrap_or_else(|| panic!("no paragraph holds {needle:?}"))
        .node
}

fn text_of(doc: &mut Document, node: NodeId) -> String {
    doc.views()
        .unwrap()
        .paragraphs()
        .iter()
        .find(|p| p.node == node)
        .unwrap()
        .text
        .clone()
}

#[test]
fn replaces_across_runs_and_keeps_formatting() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [
            {"type": "text", "text": "A wren "},
            {"type": "text", "text": "sang", "marks": {"bold": true}},
            {"type": "text", "text": " sweetly."},
        ]}]),
    );
    // A match inside one run keeps that run's formatting.
    let para = paragraph_with(&mut doc, "sang");
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "sang", "called", None).unwrap(),
        1
    );
    assert_eq!(text_of(&mut doc, para), "A wren called sweetly.");
    let bold: Vec<String> = doc
        .views()
        .unwrap()
        .paragraphs()
        .iter()
        .find(|p| p.node == para)
        .unwrap()
        .content
        .iter()
        .filter_map(|item| match item {
            nebu_docx::Inline::Text { text, toggles, .. }
                if toggles.has(nebu_docx::Toggle::Bold) =>
            {
                Some(text.clone())
            }
            _ => None,
        })
        .collect();
    assert_eq!(bold, vec!["called"], "{bold:?}");
}

#[test]
fn a_replacement_takes_the_first_runs_formatting() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [
            {"type": "text", "text": "A wren "},
            {"type": "text", "text": "sang", "marks": {"bold": true}},
            {"type": "text", "text": " sweetly."},
        ]}]),
    );
    let para = paragraph_with(&mut doc, "sang");
    assert_eq!(
        replace_in_paragraph(
            &mut doc,
            para,
            "wren sang sweetly",
            "thrush called loudly",
            None
        )
        .unwrap(),
        1
    );
    assert_eq!(text_of(&mut doc, para), "A thrush called loudly.");
    // The bold run was covered, so no bold text remains.
    let bold: Vec<String> = doc
        .views()
        .unwrap()
        .paragraphs()
        .iter()
        .find(|p| p.node == para)
        .unwrap()
        .content
        .iter()
        .filter_map(|item| match item {
            nebu_docx::Inline::Text { text, toggles, .. }
                if toggles.has(nebu_docx::Toggle::Bold) =>
            {
                Some(text.clone())
            }
            _ => None,
        })
        .collect();
    assert!(bold.is_empty(), "{bold:?}");
}

#[test]
fn replaces_every_occurrence_and_counts() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [{"type": "text", "text": "aa aaa aa"}] }]),
    );
    let para = paragraph_with(&mut doc, "aaa");
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "aa", "b", None).unwrap(),
        3
    );
    assert_eq!(text_of(&mut doc, para), "b ba b");
}

#[test]
fn a_match_is_left_alone_when_nothing_matches() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [{"type": "text", "text": "A wren sang."}] }]),
    );
    let para = paragraph_with(&mut doc, "wren");
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "nightingale", "thrush", None).unwrap(),
        0
    );
    assert_eq!(text_of(&mut doc, para), "A wren sang.");
}

#[test]
fn refuses_an_unknown_place_an_empty_find_and_a_non_paragraph() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [{"type": "text", "text": "A wren sang."}] }]),
    );
    let para = paragraph_with(&mut doc, "wren");
    let err = replace_in_paragraph(&mut doc, para, "", "thrush", None).unwrap_err();
    assert!(matches!(err, nebu_docx::Error::Invalid(_)), "{err:?}");
    let other = {
        let bytes = support::sample();
        let mut foreign = Document::from_bytes(bytes).unwrap();
        let content = Content::from_json(
            json!([{ "type": "paragraph", "content": [{"type": "text", "text": "elsewhere"}] }])
                .to_string()
                .as_bytes(),
        )
        .unwrap();
        let opts = WriteOptions::for_content(&content, None);
        foreign.insert(Place::End, &content.blocks, &opts).unwrap();
        foreign
            .views()
            .unwrap()
            .paragraphs()
            .iter()
            .find(|p| p.text.contains("elsewhere"))
            .unwrap()
            .node
    };
    let err = replace_in_paragraph(&mut doc, other, "x", "y", None).unwrap_err();
    assert!(
        err.to_string().contains("no story of this document"),
        "{err:?}"
    );
}

#[test]
fn refuses_anything_but_a_paragraph() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "table", "rows": [
            {"cells": [{"content": [{"type": "paragraph", "content": [{"type": "text", "text": "North Fen"}]}]}]},
        ]}]),
    );
    let table = doc
        .views()
        .unwrap()
        .blocks()
        .iter()
        .find(|b| b.kind == nebu_docx::BlockKind::Table)
        .expect("a table")
        .node;
    let err = replace_in_paragraph(&mut doc, table, "North", "South", None).unwrap_err();
    assert!(err.to_string().contains("not a paragraph"), "{err:?}");
}

#[test]
fn refuses_fields_hyperlinks_and_notes() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [
            {"type": "text", "text": "See "},
            {"type": "field", "code": "REF tab-sites \\h", "result": "Table 1"},
            {"type": "text", "text": " now."},
        ]}]),
    );
    let para = paragraph_with(&mut doc, "Table 1");
    // The field's result reads as text, so the match spans it and
    // refuses, naming the field; the document is unchanged.
    let before = doc.to_bytes().unwrap();
    let err = replace_in_paragraph(&mut doc, para, "See Table 1 now", "Look", None).unwrap_err();
    assert!(err.to_string().contains("a field"), "{err:?}");
    assert_eq!(doc.to_bytes().unwrap(), before);
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [
            {"type": "text", "text": "Visit "},
            {"type": "link", "target": {"url": "https://example.com/x"}, "content": [{"type": "text", "text": "the marsh"}]},
            {"type": "text", "text": " today."},
        ]}]),
    );
    let para = paragraph_with(&mut doc, "marsh");
    // The link text reads as text, so the visible sentence refuses,
    // naming the link, and the document is unchanged.
    let before = doc.to_bytes().unwrap();
    let err =
        replace_in_paragraph(&mut doc, para, "Visit the marsh today", "Go", None).unwrap_err();
    assert!(err.to_string().contains("a hyperlink"), "{err:?}");
    assert_eq!(doc.to_bytes().unwrap(), before);
    // The sentence with the link text left out is not there at all.
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "Visit  today", "Go", None).unwrap(),
        0
    );
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [
            {"type": "text", "text": "A wren"},
            {"type": "footnote", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "First seen here."}]}]},
            {"type": "text", "text": " sang."},
        ]}]),
    );
    let para = paragraph_with(&mut doc, "sang");
    let err = replace_in_paragraph(&mut doc, para, "wren sang", "thrush called", None).unwrap_err();
    assert!(err.to_string().contains("a footnote"), "{err:?}");
}

#[test]
fn refuses_a_match_wholly_inside_a_field_result() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [
            {"type": "text", "text": "See "},
            {"type": "field", "code": "REF tab-sites \\h", "result": "Table 1"},
            {"type": "text", "text": " now."},
        ]}]),
    );
    let para = paragraph_with(&mut doc, "Table 1");
    // The result alone is still a cached field value: replacing it
    // would edit what Word can overwrite on refresh.
    let before = doc.to_bytes().unwrap();
    let err = replace_in_paragraph(&mut doc, para, "Table 1", "Table 2", None).unwrap_err();
    assert!(err.to_string().contains("a field"), "{err:?}");
    assert_eq!(doc.to_bytes().unwrap(), before);
    // Text beside the field stays replaceable.
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "See ", "Look ", None).unwrap(),
        1
    );
    assert_eq!(text_of(&mut doc, para), "Look Table 1 now.");
}

#[test]
fn keeps_trailing_space_on_a_split_tail() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [{"type": "text", "text": "abc "}] }]),
    );
    let para = paragraph_with(&mut doc, "abc");
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "b", "B", None).unwrap(),
        1
    );
    assert_eq!(text_of(&mut doc, para), "aBc ");
    // The tail ends with a space, so the file must keep it preserved:
    // check the serialized attribute, not just the tree's text.
    let main = doc.main_part().to_string();
    let tree = doc.xml(&main).unwrap();
    let mut kept = false;
    for node in tree.descendants(para).unwrap() {
        if tree.is(node, Some(nebu_docx::ns::W), "t").unwrap_or(false)
            && tree.text_content(node).unwrap_or_default() == "c "
        {
            kept = tree
                .attribute(node, Some(nebu_docx::xml::XML_NS), "space")
                .unwrap()
                .is_some_and(|v| v == "preserve");
        }
    }
    assert!(kept, "the tail holding \"c \" keeps xml:space=\"preserve\"");
}

#[test]
fn replacement_text_escapes_markup() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [{"type": "text", "text": "A wren sang."}] }]),
    );
    let para = paragraph_with(&mut doc, "wren");
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "wren", "<wren> & thrush", None).unwrap(),
        1
    );
    assert_eq!(text_of(&mut doc, para), "A <wren> & thrush sang.");
    // The file parses again with the same text.
    let bytes = doc.to_bytes().unwrap();
    let mut reopened = Document::from_bytes(bytes).unwrap();
    let para = paragraph_with(&mut reopened, "<wren>");
    assert_eq!(text_of(&mut reopened, para), "A <wren> & thrush sang.");
}

#[test]
fn refuses_carriage_returns_like_other_line_ends() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [{"type": "text", "text": "A wren sang."}] }]),
    );
    let para = paragraph_with(&mut doc, "wren");
    for (find, with) in [("wr\ren", "x"), ("wren", "x\ry"), ("a\rb", "x")] {
        let err = replace_in_paragraph(&mut doc, para, find, with, None).unwrap_err();
        assert!(matches!(err, nebu_docx::Error::Invalid(_)), "{err:?}");
    }
    assert_eq!(text_of(&mut doc, para), "A wren sang.");
}

#[test]
fn replaces_inside_table_cells() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "table", "rows": [
            {"cells": [{"content": [{"type": "paragraph", "content": [{"type": "text", "text": "site"}]}]}]},
            {"cells": [{"content": [{"type": "paragraph", "content": [{"type": "text", "text": "North Fen"}]}]}]},
        ]}]),
    );
    let para = paragraph_with(&mut doc, "North");
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "North", "South", None).unwrap(),
        1
    );
    assert_eq!(text_of(&mut doc, para), "South Fen");
}

#[test]
fn keeps_spaces_at_the_edges() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [{"type": "text", "text": "a b c"}] }]),
    );
    let para = paragraph_with(&mut doc, "a b");
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "b", " X ", None).unwrap(),
        1
    );
    assert_eq!(text_of(&mut doc, para), "a  X  c");
}

/// Replaces a paragraph's runs with parsed XML: detaches its runs and
/// appends the fragment. Tests build structures the content model
/// cannot write, such as fields across paragraphs.
fn set_runs(doc: &mut Document, para: NodeId, xml: &str) {
    let main = doc.main_part().to_string();
    let tree = doc.xml(&main).unwrap();
    let runs: Vec<NodeId> = tree
        .elements(para)
        .unwrap()
        .filter(|&n| tree.is(n, Some(nebu_docx::ns::W), "r").unwrap_or(false))
        .collect();
    for r in runs {
        tree.detach(r).unwrap();
    }
    for n in tree
        .parse_fragment(
            xml,
            &[
                ("w", nebu_docx::ns::W),
                (
                    "mc",
                    "http://schemas.openxmlformats.org/markup-compatibility/2006",
                ),
            ],
        )
        .unwrap()
    {
        tree.append(para, n).unwrap();
    }
}

/// The first `w:t` of a paragraph's runs is found inline where needed.

#[test]
fn refuses_field_results_continued_from_an_earlier_paragraph() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([
            { "type": "paragraph", "content": [{"type": "text", "text": "Alpha one"}] },
            { "type": "paragraph", "content": [{"type": "text", "text": "two Beta"}] },
        ]),
    );
    let first = paragraph_with(&mut doc, "Alpha");
    let second = paragraph_with(&mut doc, "two Beta");
    // A field that begins in the first paragraph and ends in the
    // second: begin, code and separator at the end of the first, its
    // result on both sides, the end marker and plain text in the second.
    {
        let main = doc.main_part().to_string();
        let tree = doc.xml(&main).unwrap();
        let t = {
            let mut found = None;
            for run in tree.elements(first).unwrap() {
                if !tree.is(run, Some(nebu_docx::ns::W), "r").unwrap_or(false) {
                    continue;
                }
                for child in tree.elements(run).unwrap() {
                    if tree.is(child, Some(nebu_docx::ns::W), "t").unwrap_or(false) {
                        found = Some(child);
                        break;
                    }
                }
            }
            found.expect("no text node")
        };
        tree.set_text(t, "Alpha ").unwrap();
        for n in tree
            .parse_fragment(
                concat!(
                    "<w:r><w:fldChar w:fldCharType=\"begin\"/></w:r>",
                    "<w:r><w:instrText xml:space=\"preserve\"> REF x \\h </w:instrText></w:r>",
                    "<w:r><w:fldChar w:fldCharType=\"separate\"/></w:r>",
                    "<w:r><w:t>one</w:t></w:r>",
                ),
                &[("w", nebu_docx::ns::W)],
            )
            .unwrap()
        {
            tree.append(first, n).unwrap();
        }
        let t = {
            let mut found = None;
            for run in tree.elements(second).unwrap() {
                if !tree.is(run, Some(nebu_docx::ns::W), "r").unwrap_or(false) {
                    continue;
                }
                for child in tree.elements(run).unwrap() {
                    if tree.is(child, Some(nebu_docx::ns::W), "t").unwrap_or(false) {
                        found = Some(child);
                        break;
                    }
                }
            }
            found.expect("no text node")
        };
        tree.set_text(t, "two").unwrap();
        for n in tree
            .parse_fragment(
                concat!(
                    "<w:r><w:fldChar w:fldCharType=\"end\"/></w:r>",
                    "<w:r><w:t xml:space=\"preserve\"> Beta</w:t></w:r>",
                ),
                &[("w", nebu_docx::ns::W)],
            )
            .unwrap()
        {
            tree.append(second, n).unwrap();
        }
    }
    assert_eq!(text_of(&mut doc, first), "Alpha one");
    assert_eq!(text_of(&mut doc, second), "two Beta");
    // The result in the second paragraph still belongs to the field.
    let before = doc.to_bytes().unwrap();
    let err = replace_in_paragraph(&mut doc, second, "two", "2", None).unwrap_err();
    assert!(err.to_string().contains("a field"), "{err:?}");
    assert_eq!(doc.to_bytes().unwrap(), before);
    // Text after the closing marker stays replaceable.
    assert_eq!(
        replace_in_paragraph(&mut doc, second, "Beta", "Gamma", None).unwrap(),
        1
    );
    assert_eq!(text_of(&mut doc, second), "two Gamma");
}

#[test]
fn refuses_a_match_crossing_a_wrapper_boundary() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [{"type": "text", "text": "placeholder"}] }]),
    );
    let para = paragraph_with(&mut doc, "placeholder");
    set_runs(
        &mut doc,
        para,
        concat!(
            "<w:customXml><w:r><w:t>river</w:t></w:r></w:customXml>",
            "<w:r><w:t xml:space=\"preserve\"> bank</w:t></w:r>",
        ),
    );
    assert_eq!(text_of(&mut doc, para), "river bank");
    // The match starts inside the wrapper and ends outside it.
    let before = doc.to_bytes().unwrap();
    let err = replace_in_paragraph(&mut doc, para, "river bank", "stream shore", None).unwrap_err();
    assert!(err.to_string().contains("a custom XML element"), "{err:?}");
    assert_eq!(doc.to_bytes().unwrap(), before);
    // Wholly inside the wrapper stays replaceable.
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "river", "stream", None).unwrap(),
        1
    );
    assert_eq!(text_of(&mut doc, para), "stream bank");
}

#[test]
fn projects_only_the_chosen_alternate_content_branch() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [{"type": "text", "text": "placeholder"}] }]),
    );
    let para = paragraph_with(&mut doc, "placeholder");
    set_runs(
        &mut doc,
        para,
        concat!(
            "<w:r><w:t xml:space=\"preserve\">A </w:t></w:r>",
            "<mc:AlternateContent>",
            "<mc:Choice Requires=\"w\"><w:r><w:t>seen</w:t></w:r></mc:Choice>",
            "<mc:Fallback><w:r><w:t>hidden</w:t></w:r></mc:Fallback>",
            "</mc:AlternateContent>",
            "<w:r><w:t xml:space=\"preserve\"> sang.</w:t></w:r>",
        ),
    );
    assert_eq!(text_of(&mut doc, para), "A seen sang.");
    // The fallback text is not there to replace.
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "hidden", "shown", None).unwrap(),
        0
    );
    // A match across the block refuses, naming it.
    let before = doc.to_bytes().unwrap();
    let err = replace_in_paragraph(&mut doc, para, "A seen sang", "x", None).unwrap_err();
    assert!(
        err.to_string().contains("an alternate content block"),
        "{err:?}"
    );
    assert_eq!(doc.to_bytes().unwrap(), before);
}

#[test]
fn refuses_a_match_leaving_the_shown_fallback_branch() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [{"type": "text", "text": "placeholder"}] }]),
    );
    let para = paragraph_with(&mut doc, "placeholder");
    // The choice asks for nothing Word understands, so Word shows the
    // fallback; a plain run follows the block.
    set_runs(
        &mut doc,
        para,
        concat!(
            "<mc:AlternateContent>",
            "<mc:Choice Requires=\"u\" xmlns:u=\"https://example.com/unsupported\"><w:r><w:t>elsewhere</w:t></w:r></mc:Choice>",
            "<mc:Fallback><w:r><w:t>seen</w:t></w:r></mc:Fallback>",
            "</mc:AlternateContent>",
            "<w:r><w:t xml:space=\"preserve\"> sang.</w:t></w:r>",
        ),
    );
    assert_eq!(text_of(&mut doc, para), "seen sang.");
    // The match starts in the shown branch and ends outside it.
    let before = doc.to_bytes().unwrap();
    let err = replace_in_paragraph(&mut doc, para, "seen sang", "heard call", None).unwrap_err();
    assert!(
        err.to_string().contains("an alternate content block"),
        "{err:?}"
    );
    assert_eq!(doc.to_bytes().unwrap(), before);
    // Wholly inside the shown branch stays replaceable.
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "seen", "heard", None).unwrap(),
        1
    );
    assert_eq!(text_of(&mut doc, para), "heard sang.");
}

#[test]
fn leaves_ruby_annotations_alone() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [{"type": "text", "text": "placeholder"}] }]),
    );
    let para = paragraph_with(&mut doc, "placeholder");
    set_runs(
        &mut doc,
        para,
        concat!(
            "<w:ruby>",
            "<w:rt><w:r><w:t>anno</w:t></w:r></w:rt>",
            "<w:rubyBase><w:r><w:t>base</w:t></w:r></w:rubyBase>",
            "</w:ruby>",
            "<w:r><w:t xml:space=\"preserve\"> plain</w:t></w:r>",
        ),
    );
    assert_eq!(text_of(&mut doc, para), "base plain");
    // The annotation is not the paragraph's text.
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "anno", "note", None).unwrap(),
        0
    );
    // A match from the base across the ruby refuses, naming it.
    let before = doc.to_bytes().unwrap();
    let err = replace_in_paragraph(&mut doc, para, "base plain", "x", None).unwrap_err();
    assert!(err.to_string().contains("a ruby"), "{err:?}");
    assert_eq!(doc.to_bytes().unwrap(), before);
    // The base alone stays replaceable.
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "base", "stem", None).unwrap(),
        1
    );
    assert_eq!(text_of(&mut doc, para), "stem plain");
}

/// A tracking author with a fixed stamp, so the XML stays comparable.
fn tracking() -> Track {
    Track {
        author: "Max".into(),
        date: "2026-11-20T12:00:00Z".into(),
    }
}

/// The text of the whole document with revisions rejected.
fn rejected_text(doc: &mut Document) -> String {
    doc.views().unwrap().text(&TextOptions {
        revisions: Revisions::Rejected,
        ..Default::default()
    })
}

/// Wraps a paragraph's runs in an insertion by an author: detaches the
/// runs and appends an `w:ins` holding them.
fn wrap_runs(doc: &mut Document, para: NodeId, author: &str, id: u64) {
    let main = doc.main_part().to_string();
    let tree = doc.xml(&main).unwrap();
    let runs: Vec<NodeId> = tree
        .elements(para)
        .unwrap()
        .filter(|&n| tree.is(n, Some(nebu_docx::ns::W), "r").unwrap_or(false))
        .collect();
    let ins = tree
        .parse_fragment(
            &format!(
                "<w:ins w:id=\"{id}\" w:author=\"{author}\" w:date=\"2026-11-20T12:00:00Z\"/>"
            ),
            &[("w", nebu_docx::ns::W)],
        )
        .unwrap()
        .pop()
        .expect("an insertion");
    tree.insert_before(runs[0], ins).unwrap();
    for r in runs {
        tree.append(ins, r).unwrap();
    }
}

#[test]
fn tracks_a_replacement_as_a_deletion_and_an_insertion() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [{"type": "text", "text": "A wren sang."}] }]),
    );
    let para = paragraph_with(&mut doc, "wren");
    let rev = tracking();
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "wren", "thrush", Some(&rev)).unwrap(),
        1
    );
    assert_eq!(text_of(&mut doc, para), "A thrush sang.");
    assert!(rejected_text(&mut doc).contains("A wren sang."));
    // One deletion of the old text and one insertion of the new text,
    // both by the tracking author.
    let mine: Vec<_> = doc
        .views()
        .unwrap()
        .revisions()
        .iter()
        .filter(|r| r.author == "Max")
        .cloned()
        .collect();
    assert!(
        mine.iter()
            .any(|r| r.kind == RevisionKind::Deletion && r.text.contains("wren")),
        "{mine:?}"
    );
    assert!(
        mine.iter()
            .any(|r| r.kind == RevisionKind::Insertion && r.text.contains("thrush")),
        "{mine:?}"
    );
    // The file names the old text as deleted text inside a deletion and
    // the new text inside an insertion, both stamped.
    let xml = String::from_utf8(support::entry(
        &doc.to_bytes().unwrap(),
        "word/document.xml",
    ))
    .unwrap();
    assert!(xml.contains("<w:del "), "{xml}");
    assert!(xml.contains("<w:delText>wren</w:delText>"), "{xml}");
    assert!(xml.contains("<w:ins "), "{xml}");
    assert!(xml.contains("w:author=\"Max\""), "{xml}");
    assert!(xml.contains("<w:t>thrush</w:t>"), "{xml}");
}

#[test]
fn tracks_a_replacement_inside_its_own_insertion() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [{"type": "text", "text": "A wren sang."}] }]),
    );
    let para = paragraph_with(&mut doc, "wren");
    wrap_runs(&mut doc, para, "Max", 7);
    let rev = tracking();
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "wren", "thrush", Some(&rev)).unwrap(),
        1
    );
    assert_eq!(text_of(&mut doc, para), "A thrush sang.");
    // The whole paragraph is the author's pending insertion, so
    // rejecting drops it entirely.
    assert!(!rejected_text(&mut doc).contains("sang."));
    // The deletion nests inside the author's own insertion, where the
    // runs stood, rather than moving text across structures.
    let xml = String::from_utf8(support::entry(
        &doc.to_bytes().unwrap(),
        "word/document.xml",
    ))
    .unwrap();
    let ins = xml.find("<w:ins ").expect("an insertion");
    let del = xml.find("<w:del ").expect("a deletion");
    assert!(ins < del, "{xml}");
}

#[test]
fn keeps_bookmark_order_across_a_tracked_replacement() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [{"type": "text", "text": "placeholder"}] }]),
    );
    let para = paragraph_with(&mut doc, "placeholder");
    // The bookmark starts between the runs the match covers.
    set_runs(
        &mut doc,
        para,
        concat!(
            "<w:r><w:t>a</w:t></w:r>",
            "<w:bookmarkStart w:id=\"20\" w:name=\"bkm\"/>",
            "<w:r><w:t>b</w:t></w:r>",
            "<w:bookmarkEnd w:id=\"20\"/>",
        ),
    );
    let rev = tracking();
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "ab", "x", Some(&rev)).unwrap(),
        1
    );
    // Two deletions, one per side of the marker: rejecting either puts
    // its run back exactly where it stood.
    let xml = String::from_utf8(support::entry(
        &doc.to_bytes().unwrap(),
        "word/document.xml",
    ))
    .unwrap();
    assert_eq!(xml.matches("<w:del ").count(), 2, "{xml}");
    let mut doc = Document::from_bytes(doc.to_bytes().unwrap()).unwrap();
    let para = paragraph_with(&mut doc, "x");
    // Two deletions and one insertion settle.
    assert_eq!(nebu_docx::revise::settle(&mut doc, false, None).unwrap(), 3);
    let xml = String::from_utf8(support::entry(
        &doc.to_bytes().unwrap(),
        "word/document.xml",
    ))
    .unwrap();
    let start = xml
        .find("<w:bookmarkStart w:id=\"20\"")
        .expect("a bookmark start");
    let run = xml.find("<w:t>b</w:t>").expect("the restored run");
    let end = xml
        .find("<w:bookmarkEnd w:id=\"20\"")
        .expect("a bookmark end");
    assert!(start < run && run < end, "{xml}");
    assert_eq!(text_of(&mut doc, para), "ab");
}

#[test]
fn tracks_a_replacement_over_its_own_property_change() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [{"type": "text", "text": "placeholder"}] }]),
    );
    let para = paragraph_with(&mut doc, "placeholder");
    set_runs(
        &mut doc,
        para,
        concat!(
            "<w:r><w:rPr>",
            "<w:rPrChange w:id=\"9\" w:author=\"Max\" w:date=\"2026-11-20T12:00:00Z\"><w:rPr/></w:rPrChange>",
            "<w:b/></w:rPr><w:t>bold draft</w:t></w:r>",
        ),
    );
    let rev = tracking();
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "draft", "final", Some(&rev)).unwrap(),
        1
    );
    // Every revision id in the file is distinct: the split and the new
    // run record the same property change under fresh ids, and none
    // reuses the fixture's change or the sample's bookmark ids.
    // (Bookmark starts and ends legitimately share one id.)
    let xml = String::from_utf8(support::entry(
        &doc.to_bytes().unwrap(),
        "word/document.xml",
    ))
    .unwrap();
    let mut ids: Vec<&str> = vec![];
    for prefix in ["<w:del w:id=\"", "<w:ins w:id=\"", "<w:rPrChange w:id=\""] {
        let mut rest = xml.as_str();
        while let Some(at) = rest.find(prefix) {
            rest = &rest[at + prefix.len()..];
            ids.push(rest.split('"').next().unwrap_or(""));
        }
    }
    assert!(ids.len() >= 4, "del, ins and two copied marks: {ids:?}");
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(ids.len(), sorted.len(), "duplicate revision ids: {ids:?}");
    // Both settlement outcomes keep the bold text.
    let mut accepted = Document::from_bytes(doc.to_bytes().unwrap()).unwrap();
    let para = paragraph_with(&mut accepted, "final");
    assert_eq!(
        nebu_docx::revise::settle(&mut accepted, true, None).unwrap(),
        2
    );
    assert_eq!(text_of(&mut accepted, para), "bold final");
    assert!(accepted.views().unwrap().paragraphs().iter().any(|p| p.node == para && p.content.iter().any(|item| matches!(item, nebu_docx::Inline::Text { toggles, .. } if toggles.has(nebu_docx::Toggle::Bold)))));
    let mut rejected = Document::from_bytes(doc.to_bytes().unwrap()).unwrap();
    let para = paragraph_with(&mut rejected, "final");
    assert_eq!(
        nebu_docx::revise::settle(&mut rejected, false, None).unwrap(),
        2
    );
    assert_eq!(text_of(&mut rejected, para), "bold draft");
}

#[test]
fn settle_leaves_a_nested_foreign_deletion_pending() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [{"type": "text", "text": "placeholder"}] }]),
    );
    let para = paragraph_with(&mut doc, "placeholder");
    set_runs(
        &mut doc,
        para,
        concat!(
            "<w:r><w:t xml:space=\"preserve\">A </w:t></w:r>",
            "<w:del w:id=\"1\" w:author=\"Max\" w:date=\"2026-11-20T12:00:00Z\">",
            "<w:r><w:t xml:space=\"preserve\">old </w:t></w:r>",
            "<w:del w:id=\"2\" w:author=\"Pip\" w:date=\"2026-11-20T12:00:00Z\">",
            "<w:r><w:delText>ancient</w:delText></w:r>",
            "</w:del></w:del>",
            "<w:r><w:t xml:space=\"preserve\"> text</w:t></w:r>",
        ),
    );
    // Rejecting Max's deletion unwraps it but leaves Pip's alone: the
    // nested deleted text stays deleted text.
    assert_eq!(
        nebu_docx::revise::settle(&mut doc, false, Some("Max")).unwrap(),
        1
    );
    let xml = String::from_utf8(support::entry(
        &doc.to_bytes().unwrap(),
        "word/document.xml",
    ))
    .unwrap();
    assert!(!xml.contains("<w:del w:id=\"1\""), "{xml}");
    assert!(xml.contains("<w:del w:id=\"2\""), "{xml}");
    assert!(xml.contains("<w:delText>ancient</w:delText>"), "{xml}");
    // Rejecting Max's deletion restores his text; Pip's stays deleted.
    assert_eq!(text_of(&mut doc, para), "A old  text");
}

#[test]
fn tracks_a_match_spanning_plain_text_and_its_own_insertion() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [{"type": "text", "text": "placeholder"}] }]),
    );
    let para = paragraph_with(&mut doc, "placeholder");
    set_runs(
        &mut doc,
        para,
        concat!(
            "<w:r><w:t xml:space=\"preserve\">A </w:t></w:r>",
            "<w:ins w:id=\"7\" w:author=\"Max\" w:date=\"2026-11-20T12:00:00Z\">",
            "<w:r><w:t xml:space=\"preserve\">wren </w:t></w:r></w:ins>",
            "<w:r><w:t>sang.</w:t></w:r>",
        ),
    );
    assert_eq!(text_of(&mut doc, para), "A wren sang.");
    let rev = tracking();
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "A wren sang", "A thrush called", Some(&rev)).unwrap(),
        1
    );
    assert_eq!(text_of(&mut doc, para), "A thrush called.");
    // Rejecting drops the author's own insertion wholesale, with the
    // nested deletion inside it.
    let mut doc = Document::from_bytes(doc.to_bytes().unwrap()).unwrap();
    let para = paragraph_with(&mut doc, "thrush");
    assert_eq!(nebu_docx::revise::settle(&mut doc, false, None).unwrap(), 4);
    assert_eq!(text_of(&mut doc, para), "A sang.");
}

#[test]
fn refuses_a_foreign_insertion() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [{"type": "text", "text": "A wren sang."}] }]),
    );
    let para = paragraph_with(&mut doc, "wren");
    wrap_runs(&mut doc, para, "Pip", 7);
    let before = doc.to_bytes().unwrap();
    let rev = tracking();
    let err = replace_in_paragraph(&mut doc, para, "wren", "thrush", Some(&rev)).unwrap_err();
    match err {
        nebu_docx::Error::ForeignRevision { authors } => assert_eq!(authors, ["Pip"]),
        other => panic!("a foreign revision, not {other:?}"),
    }
    assert_eq!(doc.to_bytes().unwrap(), before);
}

#[test]
fn refuses_a_foreign_move_even_by_the_same_author() {
    // Moves always belong to someone else: the engine never rewrites
    // moved text, tracked or not.
    for author in ["Max", "Pip"] {
        let (mut doc, _) = insert(
            support::sample(),
            json!([{ "type": "paragraph", "content": [{"type": "text", "text": "placeholder"}] }]),
        );
        let para = paragraph_with(&mut doc, "placeholder");
        set_runs(
            &mut doc,
            para,
            &format!(
                concat!(
                    "<w:moveTo w:id=\"8\" w:author=\"{a}\" w:date=\"2026-11-20T12:00:00Z\">",
                    "<w:r><w:t>river</w:t></w:r></w:moveTo>",
                    "<w:r><w:t xml:space=\"preserve\"> bank</w:t></w:r>",
                ),
                a = author,
            ),
        );
        assert_eq!(text_of(&mut doc, para), "river bank");
        let before = doc.to_bytes().unwrap();
        let rev = tracking();
        let err = replace_in_paragraph(&mut doc, para, "river bank", "stream shore", Some(&rev))
            .unwrap_err();
        match err {
            nebu_docx::Error::ForeignRevision { authors } => assert_eq!(authors, [author]),
            other => panic!("a foreign revision, not {other:?}"),
        }
        assert_eq!(doc.to_bytes().unwrap(), before);
    }
}

#[test]
fn refuses_a_foreign_property_change_and_ignores_its_own() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [{"type": "text", "text": "placeholder"}] }]),
    );
    let para = paragraph_with(&mut doc, "placeholder");
    let run_with_mark = |author: &str| {
        format!(
            concat!(
                "<w:r><w:rPr>",
                "<w:rPrChange w:id=\"9\" w:author=\"{a}\" w:date=\"2026-11-20T12:00:00Z\"><w:rPr/></w:rPrChange>",
                "<w:b/></w:rPr><w:t>bold draft</w:t></w:r>",
            ),
            a = author,
        )
    };
    set_runs(&mut doc, para, &run_with_mark("Pip"));
    let rev = tracking();
    let err = replace_in_paragraph(&mut doc, para, "draft", "final", Some(&rev)).unwrap_err();
    match err {
        nebu_docx::Error::ForeignRevision { authors } => assert_eq!(authors, ["Pip"]),
        other => panic!("a foreign revision, not {other:?}"),
    }
    set_runs(&mut doc, para, &run_with_mark("Max"));
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "draft", "final", Some(&rev)).unwrap(),
        1
    );
    assert_eq!(text_of(&mut doc, para), "bold final");
}

#[test]
fn settle_accepts_a_tracked_replacement() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [{"type": "text", "text": "A wren sang."}] }]),
    );
    let para = paragraph_with(&mut doc, "wren");
    let rev = tracking();
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "wren", "thrush", Some(&rev)).unwrap(),
        1
    );
    assert_eq!(nebu_docx::revise::settle(&mut doc, true, None).unwrap(), 2);
    assert_eq!(text_of(&mut doc, para), "A thrush sang.");
    assert!(rejected_text(&mut doc).contains("A thrush sang."));
    assert!(
        doc.views().unwrap().revisions().is_empty(),
        "no records left"
    );
}

#[test]
fn settle_rejects_a_tracked_replacement() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [{"type": "text", "text": "A wren sang."}] }]),
    );
    let para = paragraph_with(&mut doc, "wren");
    let rev = tracking();
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "wren", "thrush", Some(&rev)).unwrap(),
        1
    );
    assert_eq!(nebu_docx::revise::settle(&mut doc, false, None).unwrap(), 2);
    assert_eq!(text_of(&mut doc, para), "A wren sang.");
    assert!(
        doc.views().unwrap().revisions().is_empty(),
        "no records left"
    );
}

#[test]
fn settle_only_touches_the_named_author() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([
            { "type": "paragraph", "content": [{"type": "text", "text": "A wren sang."}] },
            { "type": "paragraph", "content": [{"type": "text", "text": "A pipit called."}] },
        ]),
    );
    let first = paragraph_with(&mut doc, "wren");
    let second = paragraph_with(&mut doc, "pipit");
    let max = tracking();
    let pip = Track {
        author: "Pip".into(),
        date: "2026-11-20T12:00:00Z".into(),
    };
    assert_eq!(
        replace_in_paragraph(&mut doc, first, "wren", "thrush", Some(&max)).unwrap(),
        1
    );
    assert_eq!(
        replace_in_paragraph(&mut doc, second, "pipit", "lark", Some(&pip)).unwrap(),
        1
    );
    assert_eq!(
        nebu_docx::revise::settle(&mut doc, true, Some("Max")).unwrap(),
        2
    );
    assert_eq!(text_of(&mut doc, first), "A thrush sang.");
    assert_eq!(text_of(&mut doc, second), "A lark called.");
    let rest: Vec<_> = doc
        .views()
        .unwrap()
        .revisions()
        .iter()
        .map(|r| r.author.clone())
        .collect();
    assert!(rest.iter().all(|a| a == "Pip"), "{rest:?}");
    assert!(!rest.is_empty(), "Pip's records stay");
}

#[test]
fn settle_leaves_moves_and_property_marks() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [{"type": "text", "text": "placeholder"}] }]),
    );
    let para = paragraph_with(&mut doc, "placeholder");
    set_runs(
        &mut doc,
        para,
        concat!(
            "<w:moveTo w:id=\"8\" w:author=\"Mover\" w:date=\"2026-11-20T12:00:00Z\">",
            "<w:r><w:t xml:space=\"preserve\">M </w:t></w:r></w:moveTo>",
            "<w:r><w:t>draft</w:t></w:r>",
            "<w:r><w:rPr>",
            "<w:rPrChange w:id=\"9\" w:author=\"Pip\" w:date=\"2026-11-20T12:00:00Z\"><w:rPr/></w:rPrChange>",
            "</w:rPr><w:t xml:space=\"preserve\"> here</w:t></w:r>",
        ),
    );
    let rev = tracking();
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "draft", "final", Some(&rev)).unwrap(),
        1
    );
    assert_eq!(nebu_docx::revise::settle(&mut doc, true, None).unwrap(), 2);
    assert_eq!(text_of(&mut doc, para), "M final here");
    let kinds: Vec<_> = doc
        .views()
        .unwrap()
        .revisions()
        .iter()
        .map(|r| r.kind)
        .collect();
    assert!(kinds.contains(&RevisionKind::MoveTo), "{kinds:?}");
    assert!(kinds.contains(&RevisionKind::Formatting), "{kinds:?}");
    assert_eq!(kinds.len(), 2, "{kinds:?}");
}

/// A move pair between two paragraphs, with range markers: the source
/// holds the moveFrom record, the destination the moveTo record. The
/// ids are realistic: each range has its own marker id shared by its
/// start and end, the wrappers carry separate ids, and the two ranges
/// pair by name. The destination paragraph also carries a paragraph-mark
/// move in its natural properties, inside body-level range markers.
fn moved_pair(doc: &mut Document, source: NodeId, dest: NodeId, author: &str) {
    set_runs(
        doc,
        source,
        &format!(
            concat!(
                "<w:moveFromRangeStart w:id=\"10\" w:name=\"m1\" w:author=\"{a}\" w:date=\"2026-11-20T12:00:00Z\"/>",
                "<w:moveFrom w:id=\"11\" w:author=\"{a}\" w:date=\"2026-11-20T12:00:00Z\" w:name=\"m1\">",
                "<w:r><w:t>rover</w:t></w:r></w:moveFrom>",
                "<w:moveFromRangeEnd w:id=\"10\"/>",
                "<w:r><w:t xml:space=\"preserve\"> stays</w:t></w:r>",
            ),
            a = author,
        ),
    );
    set_runs(
        doc,
        dest,
        &format!(
            concat!(
                "<w:r><w:t xml:space=\"preserve\">goes </w:t></w:r>",
                "<w:moveTo w:id=\"21\" w:author=\"{a}\" w:date=\"2026-11-20T12:00:00Z\" w:name=\"m1\">",
                "<w:r><w:t>rover</w:t></w:r></w:moveTo>",
            ),
            a = author,
        ),
    );
    let main = doc.main_part().to_string();
    let tree = doc.xml(&main).unwrap();
    // The paragraph-mark move lives in the destination's own properties.
    let props = tree
        .elements(dest)
        .unwrap()
        .find(|&n| tree.is(n, Some(nebu_docx::ns::W), "pPr").unwrap_or(false))
        .expect("destination properties");
    for n in tree
        .parse_fragment(
            &format!(
                "<w:rPr><w:moveTo w:id=\"22\" w:author=\"{a}\" w:date=\"2026-11-20T12:00:00Z\"/></w:rPr>",
                a = author,
            ),
            &[("w", nebu_docx::ns::W)],
        )
        .unwrap()
    {
        tree.append(props, n).unwrap();
    }
    // The destination range markers wrap the whole paragraph. Starts
    // carry the name with the author and date; ends carry only the id.
    for (xml, place) in [
        (
            format!(
                "<w:moveToRangeStart w:id=\"20\" w:name=\"m1\" w:author=\"{author}\" w:date=\"2026-11-20T12:00:00Z\"/>"
            ),
            true,
        ),
        ("<w:moveToRangeEnd w:id=\"20\"/>".to_string(), false),
    ] {
        let mut marker = None;
        for n in tree
            .parse_fragment(xml.as_str(), &[("w", nebu_docx::ns::W)])
            .unwrap()
        {
            marker = Some(n);
        }
        let marker = marker.expect("a range marker");
        if place {
            tree.insert_before(dest, marker).unwrap();
        } else {
            tree.insert_after(dest, marker).unwrap();
        }
    }
}

/// The start tag of the first element with this local name: the
/// preserved markers keep the shape Word writes, with the author and
/// date on starts and nothing but the id on ends.
fn marker_tag<'a>(xml: &'a str, local: &str) -> &'a str {
    let at = xml.find(local).unwrap_or_else(|| panic!("no {local}"));
    let end = xml[at..].find('>').expect("a tag end");
    &xml[at..at + end]
}

#[test]
fn settle_accepts_a_move_pair() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([
            { "type": "paragraph", "content": [{"type": "text", "text": "source here"}] },
            { "type": "paragraph", "content": [{"type": "text", "text": "dest here"}] },
        ]),
    );
    let source = paragraph_with(&mut doc, "source");
    let dest = paragraph_with(&mut doc, "dest");
    moved_pair(&mut doc, source, dest, "Lark");
    assert_eq!(nebu_docx::revise::settle(&mut doc, true, None).unwrap(), 2);
    assert_eq!(text_of(&mut doc, source), " stays");
    assert_eq!(text_of(&mut doc, dest), "goes rover");
    // The two text records settle; the paragraph-mark revision stays
    // pending. Views never surface property-nested moves, so
    // preservation shows in the XML.
    let revisions = doc.views().unwrap().revisions();
    assert_eq!(revisions.len(), 1, "{revisions:?}");
    assert_eq!(revisions[0].kind, RevisionKind::MoveTo, "{revisions:?}");
    assert_eq!(revisions[0].author, "Lark", "{revisions:?}");
    let xml = String::from_utf8(support::entry(
        &doc.to_bytes().unwrap(),
        "word/document.xml",
    ))
    .unwrap();
    // The text wrappers are gone, the paragraph-mark record stays, and
    // both ranges stay with it: id 22 remains inside its destination
    // range with the matching source range.
    assert!(!xml.contains("w:moveFrom w:id"), "{xml}");
    assert!(!xml.contains("w:moveTo w:id=\"21\""), "{xml}");
    assert!(xml.contains("w:moveFromRangeStart"), "{xml}");
    assert!(xml.contains("w:moveFromRangeEnd"), "{xml}");
    let start = xml.find("w:moveToRangeStart").expect("a destination start");
    let record = xml.find("w:moveTo w:id=\"22\"").expect("id 22");
    let end = xml.find("w:moveToRangeEnd").expect("a destination end");
    assert!(start < record && record < end, "{xml}");
    // The starts keep the author and date; the ends carry only the id.
    for (local, id) in [("w:moveFromRangeStart", "10"), ("w:moveToRangeStart", "20")] {
        let tag = marker_tag(&xml, local);
        assert!(tag.contains(&format!("w:id=\"{id}\"")), "{tag}");
        assert!(tag.contains("w:name=\"m1\""), "{tag}");
        assert!(tag.contains("w:author=\"Lark\""), "{tag}");
        assert!(tag.contains("w:date=\"2026-11-20T12:00:00Z\""), "{tag}");
    }
    for (local, id) in [("w:moveFromRangeEnd", "10"), ("w:moveToRangeEnd", "20")] {
        let tag = marker_tag(&xml, local);
        assert!(tag.contains(&format!("w:id=\"{id}\"")), "{tag}");
        assert!(!tag.contains("w:name"), "{tag}");
    }
}

#[test]
fn settle_rejects_a_move_pair() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([
            { "type": "paragraph", "content": [{"type": "text", "text": "source here"}] },
            { "type": "paragraph", "content": [{"type": "text", "text": "dest here"}] },
        ]),
    );
    let source = paragraph_with(&mut doc, "source");
    let dest = paragraph_with(&mut doc, "dest");
    moved_pair(&mut doc, source, dest, "Lark");
    assert_eq!(nebu_docx::revise::settle(&mut doc, false, None).unwrap(), 2);
    assert_eq!(text_of(&mut doc, source), "rover stays");
    assert_eq!(text_of(&mut doc, dest), "goes ");
    // The two text records settle; the paragraph-mark revision stays
    // pending. Views never surface property-nested moves, so
    // preservation shows in the XML.
    let revisions = doc.views().unwrap().revisions();
    assert_eq!(revisions.len(), 1, "{revisions:?}");
    assert_eq!(revisions[0].kind, RevisionKind::MoveTo, "{revisions:?}");
    assert_eq!(revisions[0].author, "Lark", "{revisions:?}");
    let xml = String::from_utf8(support::entry(
        &doc.to_bytes().unwrap(),
        "word/document.xml",
    ))
    .unwrap();
    // The text wrappers are gone, the paragraph-mark record stays, and
    // both ranges stay with it: id 22 remains inside its destination
    // range with the matching source range.
    assert!(!xml.contains("w:moveFrom w:id"), "{xml}");
    assert!(!xml.contains("w:moveTo w:id=\"21\""), "{xml}");
    assert!(xml.contains("w:moveFromRangeStart"), "{xml}");
    assert!(xml.contains("w:moveFromRangeEnd"), "{xml}");
    let start = xml.find("w:moveToRangeStart").expect("a destination start");
    let record = xml.find("w:moveTo w:id=\"22\"").expect("id 22");
    let end = xml.find("w:moveToRangeEnd").expect("a destination end");
    assert!(start < record && record < end, "{xml}");
    // The starts keep the author and date; the ends carry only the id.
    for (local, id) in [("w:moveFromRangeStart", "10"), ("w:moveToRangeStart", "20")] {
        let tag = marker_tag(&xml, local);
        assert!(tag.contains(&format!("w:id=\"{id}\"")), "{tag}");
        assert!(tag.contains("w:name=\"m1\""), "{tag}");
        assert!(tag.contains("w:author=\"Lark\""), "{tag}");
        assert!(tag.contains("w:date=\"2026-11-20T12:00:00Z\""), "{tag}");
    }
    for (local, id) in [("w:moveFromRangeEnd", "10"), ("w:moveToRangeEnd", "20")] {
        let tag = marker_tag(&xml, local);
        assert!(tag.contains(&format!("w:id=\"{id}\"")), "{tag}");
        assert!(!tag.contains("w:name"), "{tag}");
    }
}

#[test]
fn settle_moves_only_touch_the_named_author() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([
            { "type": "paragraph", "content": [{"type": "text", "text": "source here"}] },
            { "type": "paragraph", "content": [{"type": "text", "text": "dest here"}] },
        ]),
    );
    let source = paragraph_with(&mut doc, "source");
    let dest = paragraph_with(&mut doc, "dest");
    moved_pair(&mut doc, source, dest, "Lark");
    // Another author's move stays whole: nothing settles. The accepted
    // text already reads through the pending move; the rejected text
    // still holds the source side.
    assert_eq!(
        nebu_docx::revise::settle(&mut doc, true, Some("Wren")).unwrap(),
        0
    );
    assert_eq!(text_of(&mut doc, source), " stays");
    assert_eq!(text_of(&mut doc, dest), "goes rover");
    assert!(rejected_text(&mut doc).contains("rover stays"));
    let pre: Vec<_> = doc
        .views()
        .unwrap()
        .revisions()
        .iter()
        .map(|r| (r.kind, r.author.clone(), r.text.clone()))
        .collect();
    assert_eq!(pre.len(), 2, "{pre:?}");
    assert_eq!(
        nebu_docx::revise::settle(&mut doc, true, Some("Lark")).unwrap(),
        2
    );
    assert_eq!(text_of(&mut doc, source), " stays");
    assert_eq!(text_of(&mut doc, dest), "goes rover");
}

#[test]
fn replace_survives_the_node_limit() {
    // Budget the main part tightly. The template paragraph below holds
    // one plain run whose text needs a head split to match, so a
    // tracked replace proceeds through text splits, run splits, a
    // deletion with moved and renamed runs, and an insertion before the
    // node budget runs out. Rolling back must restore the original
    // nodes — not copies — and their serialized form, and free the
    // arena for another edit. Room for nine nodes lets the tracked edit
    // allocate eight of them before failing; the untracked follow-up
    // needs eight of its own, so it only fits on reclaimed ground.
    let bytes = support::sample_with(&[("word/styles.xml", None), ("word/numbering.xml", None)]);
    let arena = {
        let mut probe = Document::from_bytes(bytes.clone()).unwrap();
        let main = probe.main_part().to_string();
        let tree = probe.xml(&main).unwrap();
        tree.descendants(tree.document()).unwrap().count() + 1
    };
    let mut doc = Document::from_bytes_with_limits(
        bytes,
        nebu_docx::Limits {
            max_xml_nodes: arena + 9,
            ..Default::default()
        },
    )
    .unwrap();
    let para = paragraph_with(&mut doc, "summary");
    // The head text node and its run, to check identity after rollback.
    let (head, run) = {
        let main = doc.main_part().to_string();
        let tree = doc.xml(&main).unwrap();
        let mut found = None;
        for r in tree.elements(para).unwrap() {
            if !tree.is(r, Some(nebu_docx::ns::W), "r").unwrap_or(false) {
                continue;
            }
            for child in tree.elements(r).unwrap() {
                if tree.is(child, Some(nebu_docx::ns::W), "t").unwrap_or(false) {
                    found = Some((child, r));
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        found.expect("a head text node")
    };
    let rev = tracking();
    let before = doc.to_bytes().unwrap();
    let err = replace_in_paragraph(&mut doc, para, "summary", "findings", Some(&rev)).unwrap_err();
    assert!(matches!(err, nebu_docx::Error::Limit { .. }), "{err:?}");
    // The document is byte-identical, with no pending revisions, and
    // the original head text node is back where it stood.
    assert_eq!(doc.to_bytes().unwrap(), before);
    assert!(doc.views().unwrap().revisions().is_empty());
    {
        let main = doc.main_part().to_string();
        let tree = doc.xml(&main).unwrap();
        assert!(
            tree.children(run).unwrap().any(|k| k == head),
            "the head text node"
        );
        assert_eq!(
            tree.text_content(head).unwrap().as_str(),
            "The summary goes here."
        );
    }
    // The rollback reclaimed its arena share.
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "summary", "findings", None).unwrap(),
        1
    );
    assert_eq!(text_of(&mut doc, para), "The findings goes here.");
}

#[test]
fn replace_rollback_keeps_the_original_bytes() {
    // The node-limit test above compares saved bytes, but the fixture's
    // own compression happens to match what the writer produces — so a
    // rollback that restores the nodes yet leaves the tree dirty still
    // passes. Repacking at another deflate level makes a rewrite
    // observable: the failed edit must leave the archive byte-identical.
    let bytes = support::sample_with(&[("word/styles.xml", None), ("word/numbering.xml", None)]);
    let files = support::entries(&bytes);
    let specs: Vec<support::zipraw::Spec> = files
        .iter()
        .map(|(n, d)| support::zipraw::Spec::new(n, d))
        .collect();
    let bytes = support::zipraw::build(&specs, &support::zipraw::Layout::default());
    let arena = {
        let mut probe = Document::from_bytes(bytes.clone()).unwrap();
        let main = probe.main_part().to_string();
        let tree = probe.xml(&main).unwrap();
        tree.descendants(tree.document()).unwrap().count() + 1
    };
    let mut doc = Document::from_bytes_with_limits(
        bytes,
        nebu_docx::Limits {
            max_xml_nodes: arena + 9,
            ..Default::default()
        },
    )
    .unwrap();
    let para = paragraph_with(&mut doc, "summary");
    let before = doc.to_bytes().unwrap();
    let err =
        replace_in_paragraph(&mut doc, para, "summary", "findings", Some(&tracking())).unwrap_err();
    assert!(matches!(err, nebu_docx::Error::Limit { .. }), "{err:?}");
    assert_eq!(doc.to_bytes().unwrap(), before);
}

#[test]
fn validate_names_a_bad_tracking_stamp() {
    let bad = Track {
        author: "Max\x01".into(),
        date: "2026-11-20T12:00:00Z".into(),
    };
    let err = nebu_docx::revise::validate("wren", "thrush", Some(&bad)).unwrap_err();
    assert!(err.to_string().contains("tracking author"), "{err:?}");
    let bad = Track {
        author: "Max".into(),
        date: "2026-11-20\x0100:00:00Z".into(),
    };
    let err = nebu_docx::revise::validate("wren", "thrush", Some(&bad)).unwrap_err();
    assert!(err.to_string().contains("tracking date"), "{err:?}");
    assert!(nebu_docx::revise::validate("wren", "thrush", Some(&tracking())).is_ok());
}

#[test]
fn batch_reports_each_paragraph_and_keeps_going() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([
            { "type": "paragraph", "content": [{"type": "text", "text": "plain draft here"}] },
            { "type": "table", "rows": [{"cells": [
                {"content": [{"type": "paragraph", "content": [
                    {"type": "text", "text": "See "},
                    {"type": "field", "code": "REF a \\h", "result": "draft"},
                    {"type": "text", "text": " A"},
                ]}]},
                {"content": [{"type": "paragraph", "content": [
                    {"type": "text", "text": "See "},
                    {"type": "field", "code": "REF b \\h", "result": "draft"},
                    {"type": "text", "text": " B"},
                ]}]},
            ]}]},
            { "type": "paragraph", "content": [{"type": "text", "text": "plain draft there"}] },
        ]),
    );
    let paras: Vec<NodeId> = doc
        .views()
        .unwrap()
        .paragraphs()
        .iter()
        .map(|p| p.node)
        .collect();
    let report = replace_all(&mut doc, &paras, "draft", "final", None).unwrap();
    // Both plain paragraphs change; both field results refuse.
    assert_eq!(report.replaced.iter().map(|(_, n)| n).sum::<usize>(), 2);
    assert_eq!(report.refused.len(), 2);
    for (_, error) in &report.refused {
        assert!(error.to_string().contains("a field"), "{error:?}");
    }
    let text = rejected_text(&mut doc);
    assert!(text.contains("plain final here"), "{text}");
    assert!(text.contains("plain final there"), "{text}");
}

#[test]
fn keeps_non_element_siblings_in_place_across_a_tracked_replacement() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [{"type": "text", "text": "placeholder"}] }]),
    );
    let para = paragraph_with(&mut doc, "placeholder");
    // A comment and a processing instruction stand between the covered
    // runs. They are not elements the flat scan names, but they still
    // hold the two runs apart.
    set_runs(
        &mut doc,
        para,
        concat!(
            "<w:r><w:t>a</w:t></w:r>",
            "<!--keep-->",
            "<?tgt data?>",
            "<w:r><w:t>b</w:t></w:r>",
        ),
    );
    let rev = tracking();
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "ab", "x", Some(&rev)).unwrap(),
        1
    );
    let xml = String::from_utf8(support::entry(
        &doc.to_bytes().unwrap(),
        "word/document.xml",
    ))
    .unwrap();
    assert_eq!(xml.matches("<w:del ").count(), 2, "{xml}");
    let mut doc = Document::from_bytes(doc.to_bytes().unwrap()).unwrap();
    let para = paragraph_with(&mut doc, "x");
    assert_eq!(nebu_docx::revise::settle(&mut doc, false, None).unwrap(), 3);
    let xml = String::from_utf8(support::entry(
        &doc.to_bytes().unwrap(),
        "word/document.xml",
    ))
    .unwrap();
    let a = xml.find("<w:t>a</w:t>").expect("the first run");
    let comment = xml.find("keep").expect("the comment");
    let pi = xml.find("<?tgt").expect("the processing instruction");
    let b = xml.find("<w:t>b</w:t>").expect("the second run");
    assert!(a < comment && comment < pi && pi < b, "{xml}");
    assert_eq!(text_of(&mut doc, para), "ab");
}

#[test]
fn batch_refuses_a_paragraph_in_a_detached_container() {
    let (mut doc, _) = insert(
        support::sample(),
        json!([{ "type": "paragraph", "content": [{"type": "text", "text": "A wren sang."}] }]),
    );
    let para = paragraph_with(&mut doc, "wren");
    // Pull the paragraph out of the body into a text box that belongs
    // to no part: the seed walk would still find it, but the edit could
    // never serialize.
    let main = doc.main_part().to_string();
    let tree = doc.xml(&main).unwrap();
    let boxy = tree
        .parse_fragment("<w:txbxContent/>", &[("w", nebu_docx::ns::W)])
        .unwrap()
        .pop()
        .expect("a text box");
    tree.append(boxy, para).unwrap();
    let report = replace_all(&mut doc, &[para], "wren", "thrush", None).unwrap();
    assert!(report.replaced.is_empty(), "{report:?}");
    assert_eq!(report.refused.len(), 1);
    assert!(
        report.refused[0].1.to_string().contains("not attached"),
        "{report:?}"
    );
}

#[test]
fn replace_keeps_a_foreign_prefix_on_rollback() {
    // The matched text uses another prefix bound to the same
    // namespace. Rolling back must restore that spelling, not the
    // canonical one: `set_name` marks elements renamed, after which
    // the serializer no longer reuses the original tag. Style parts
    // are stripped so the node budget binds the main part.
    let stripped = support::sample_with(&[("word/styles.xml", None), ("word/numbering.xml", None)]);
    let (mut doc, _) = insert(
        stripped,
        json!([{ "type": "paragraph", "content": [{"type": "text", "text": "placeholder"}] }]),
    );
    let para = paragraph_with(&mut doc, "placeholder");
    {
        let main = doc.main_part().to_string();
        let tree = doc.xml(&main).unwrap();
        let runs: Vec<NodeId> = tree
            .elements(para)
            .unwrap()
            .filter(|&n| tree.is(n, Some(nebu_docx::ns::W), "r").unwrap_or(false))
            .collect();
        for r in runs {
            tree.detach(r).unwrap();
        }
        for n in tree
            .parse_fragment(
                "<w:r><x:t xmlns:x=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">wren</x:t></w:r>",
                &[("w", nebu_docx::ns::W)],
            )
            .unwrap()
        {
            tree.append(para, n).unwrap();
        }
    }
    let arena = {
        let mut probe = Document::from_bytes(doc.to_bytes().unwrap()).unwrap();
        let main = probe.main_part().to_string();
        let tree = probe.xml(&main).unwrap();
        tree.descendants(tree.document()).unwrap().count() + 1
    };
    // Room for three new nodes: the deletion, the insertion and its
    // run all fit, so the replacement text fails to parse and the
    // rollback must restore the original qualified name.
    let bytes = doc.to_bytes().unwrap();
    let mut doc = Document::from_bytes_with_limits(
        bytes.clone(),
        nebu_docx::Limits {
            max_xml_nodes: arena + 3,
            ..Default::default()
        },
    )
    .unwrap();
    let para = paragraph_with(&mut doc, "wren");
    let rev = tracking();
    let before = doc.to_bytes().unwrap();
    let before_xml = String::from_utf8(support::entry(&before, "word/document.xml")).unwrap();
    assert!(before_xml.contains("<x:t"), "{before_xml}");
    let err = replace_in_paragraph(&mut doc, para, "wren", "thrush", Some(&rev)).unwrap_err();
    assert!(matches!(err, nebu_docx::Error::Limit { .. }), "{err:?}");
    // The rollback restored the foreign spelling, byte for byte — and
    // in memory too: the passthrough serializer reuses the original
    // tag while the node looks untouched, so bytes alone would miss a
    // rename left behind.
    assert_eq!(doc.to_bytes().unwrap(), before);
    {
        let main = doc.main_part().to_string();
        let tree = doc.xml(&main).unwrap();
        let mut text = None;
        for r in tree.elements(para).unwrap() {
            if !tree.is(r, Some(nebu_docx::ns::W), "r").unwrap_or(false) {
                continue;
            }
            for child in tree.elements(r).unwrap() {
                if tree.is(child, Some(nebu_docx::ns::W), "t").unwrap_or(false) {
                    text = Some(child);
                }
            }
        }
        let text = text.expect("the restored text");
        let got = tree.name(text).unwrap().expect("an element name");
        assert_eq!(got.namespace, Some(nebu_docx::ns::W));
        assert_eq!(got.prefix, "x");
        assert_eq!(got.local, "t");
    }
}
