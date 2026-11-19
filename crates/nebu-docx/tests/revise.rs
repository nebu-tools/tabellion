//! Tracked and untracked text replacement inside one paragraph.

mod support;

use nebu_docx::content::Content;
use nebu_docx::revise::replace_in_paragraph;
use nebu_docx::xml::NodeId;
use nebu_docx::{Document, Place, WriteOptions};
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
        replace_in_paragraph(&mut doc, para, "sang", "called").unwrap(),
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
        replace_in_paragraph(&mut doc, para, "wren sang sweetly", "thrush called loudly").unwrap(),
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
    assert_eq!(replace_in_paragraph(&mut doc, para, "aa", "b").unwrap(), 3);
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
        replace_in_paragraph(&mut doc, para, "nightingale", "thrush").unwrap(),
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
    let err = replace_in_paragraph(&mut doc, para, "", "thrush").unwrap_err();
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
    let err = replace_in_paragraph(&mut doc, other, "x", "y").unwrap_err();
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
    let err = replace_in_paragraph(&mut doc, table, "North", "South").unwrap_err();
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
    let err = replace_in_paragraph(&mut doc, para, "See Table 1 now", "Look").unwrap_err();
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
    let err = replace_in_paragraph(&mut doc, para, "Visit the marsh today", "Go").unwrap_err();
    assert!(err.to_string().contains("a hyperlink"), "{err:?}");
    assert_eq!(doc.to_bytes().unwrap(), before);
    // The sentence with the link text left out is not there at all.
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "Visit  today", "Go").unwrap(),
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
    let err = replace_in_paragraph(&mut doc, para, "wren sang", "thrush called").unwrap_err();
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
    let err = replace_in_paragraph(&mut doc, para, "Table 1", "Table 2").unwrap_err();
    assert!(err.to_string().contains("a field"), "{err:?}");
    assert_eq!(doc.to_bytes().unwrap(), before);
    // Text beside the field stays replaceable.
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "See ", "Look ").unwrap(),
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
    assert_eq!(replace_in_paragraph(&mut doc, para, "b", "B").unwrap(), 1);
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
        replace_in_paragraph(&mut doc, para, "wren", "<wren> & thrush").unwrap(),
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
        let err = replace_in_paragraph(&mut doc, para, find, with).unwrap_err();
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
        replace_in_paragraph(&mut doc, para, "North", "South").unwrap(),
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
    assert_eq!(replace_in_paragraph(&mut doc, para, "b", " X ").unwrap(), 1);
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
    let err = replace_in_paragraph(&mut doc, second, "two", "2").unwrap_err();
    assert!(err.to_string().contains("a field"), "{err:?}");
    assert_eq!(doc.to_bytes().unwrap(), before);
    // Text after the closing marker stays replaceable.
    assert_eq!(
        replace_in_paragraph(&mut doc, second, "Beta", "Gamma").unwrap(),
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
    let err = replace_in_paragraph(&mut doc, para, "river bank", "stream shore").unwrap_err();
    assert!(err.to_string().contains("a custom XML element"), "{err:?}");
    assert_eq!(doc.to_bytes().unwrap(), before);
    // Wholly inside the wrapper stays replaceable.
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "river", "stream").unwrap(),
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
        replace_in_paragraph(&mut doc, para, "hidden", "shown").unwrap(),
        0
    );
    // A match across the block refuses, naming it.
    let before = doc.to_bytes().unwrap();
    let err = replace_in_paragraph(&mut doc, para, "A seen sang", "x").unwrap_err();
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
    let err = replace_in_paragraph(&mut doc, para, "seen sang", "heard call").unwrap_err();
    assert!(
        err.to_string().contains("an alternate content block"),
        "{err:?}"
    );
    assert_eq!(doc.to_bytes().unwrap(), before);
    // Wholly inside the shown branch stays replaceable.
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "seen", "heard").unwrap(),
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
        replace_in_paragraph(&mut doc, para, "anno", "note").unwrap(),
        0
    );
    // A match from the base across the ruby refuses, naming it.
    let before = doc.to_bytes().unwrap();
    let err = replace_in_paragraph(&mut doc, para, "base plain", "x").unwrap_err();
    assert!(err.to_string().contains("a ruby"), "{err:?}");
    assert_eq!(doc.to_bytes().unwrap(), before);
    // The base alone stays replaceable.
    assert_eq!(
        replace_in_paragraph(&mut doc, para, "base", "stem").unwrap(),
        1
    );
    assert_eq!(text_of(&mut doc, para), "stem plain");
}
