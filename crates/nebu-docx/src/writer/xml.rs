//! Building WordprocessingML as text. The writer composes each block as a
//! string with the prefixes of [`RAW_PREFIXES`](crate::content::RAW_PREFIXES)
//! and parses it into the target tree as a fragment, which writes it with
//! the prefixes the target binds.

/// Escapes text for an element's content.
pub(crate) fn text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

/// Escapes text for an attribute value in double quotes.
pub(crate) fn attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            _ => out.push(c),
        }
    }
    out
}

/// A `w:t`, keeping its spaces.
pub(crate) fn t(s: &str) -> String {
    if s.starts_with(' ') || s.ends_with(' ') || s.contains("  ") {
        format!("<w:t xml:space=\"preserve\">{}</w:t>", text(s))
    } else {
        format!("<w:t>{}</w:t>", text(s))
    }
}

/// The inner content of a run for text with tabs and line breaks.
pub(crate) fn run_text(s: &str) -> String {
    let mut out = String::new();
    let mut part = String::new();
    for c in s.chars() {
        match c {
            '\t' | '\n' => {
                if !part.is_empty() {
                    out.push_str(&t(&part));
                    part.clear();
                }
                out.push_str(if c == '\t' { "<w:tab/>" } else { "<w:br/>" });
            }
            _ => part.push(c),
        }
    }
    if !part.is_empty() {
        out.push_str(&t(&part));
    }
    out
}

/// Points to twentieths of a point, rounded.
pub(crate) fn twips(pt: f64) -> i64 {
    (pt * 20.0).round() as i64
}

/// Points to English Metric Units.
pub(crate) fn emu(pt: f64) -> i64 {
    (pt * 12700.0).round() as i64
}

/// A border width in points to eighths of a point, within what Word draws.
pub(crate) fn eighths(pt: f64) -> i64 {
    ((pt * 8.0).round() as i64).clamp(2, 96)
}

/// The order of the children of `w:pPr` (ECMA-376 part 1, 17.3.1.26).
pub(crate) const PPR_ORDER: &[&str] = &[
    "pStyle",
    "keepNext",
    "keepLines",
    "pageBreakBefore",
    "framePr",
    "widowControl",
    "numPr",
    "suppressLineNumbers",
    "pBdr",
    "shd",
    "tabs",
    "suppressAutoHyphens",
    "kinsoku",
    "wordWrap",
    "overflowPunct",
    "topLinePunct",
    "autoSpaceDE",
    "autoSpaceDN",
    "bidi",
    "adjustRightInd",
    "snapToGrid",
    "spacing",
    "ind",
    "contextualSpacing",
    "mirrorIndents",
    "suppressOverlap",
    "jc",
    "textDirection",
    "textAlignment",
    "textboxTightWrap",
    "outlineLvl",
    "divId",
    "cnfStyle",
    "rPr",
    "sectPr",
    "pPrChange",
];

/// Paragraph properties from `(name, xml)` pairs, in schema order.
pub(crate) fn ppr(mut parts: Vec<(&'static str, String)>) -> String {
    if parts.is_empty() {
        return String::new();
    }
    parts.sort_by_key(|(n, _)| PPR_ORDER.iter().position(|o| o == n).unwrap_or(usize::MAX));
    let mut out = String::from("<w:pPr>");
    for (_, x) in parts {
        out.push_str(&x);
    }
    out.push_str("</w:pPr>");
    out
}

/// The order of the children of `w:settings` (ECMA-376 part 1, 17.15.1.78).
pub(crate) const SETTINGS_ORDER: &[&str] = &[
    "writeProtection",
    "view",
    "zoom",
    "removePersonalInformation",
    "removeDateAndTime",
    "doNotDisplayPageBoundaries",
    "displayBackgroundShape",
    "printPostScriptOverText",
    "printFractionalCharacterWidth",
    "printFormsData",
    "embedTrueTypeFonts",
    "embedSystemFonts",
    "saveSubsetFonts",
    "saveFormsData",
    "mirrorMargins",
    "alignBordersAndEdges",
    "bordersDoNotSurroundHeader",
    "bordersDoNotSurroundFooter",
    "gutterAtTop",
    "hideSpellingErrors",
    "hideGrammaticalErrors",
    "activeWritingStyle",
    "proofState",
    "formsDesign",
    "attachedTemplate",
    "linkStyles",
    "stylePaneFormatFilter",
    "stylePaneSortMethod",
    "documentType",
    "mailMerge",
    "revisionView",
    "trackRevisions",
    "doNotTrackMoves",
    "doNotTrackFormatting",
    "documentProtection",
    "autoFormatOverride",
    "styleLockTheme",
    "styleLockQFSet",
    "defaultTabStop",
    "autoHyphenation",
    "consecutiveHyphenLimit",
    "hyphenationZone",
    "doNotHyphenateCaps",
    "showEnvelope",
    "summaryLength",
    "clickAndTypeStyle",
    "defaultTableStyle",
    "evenAndOddHeaders",
    "bookFoldRevPrinting",
    "bookFoldPrinting",
    "bookFoldPrintingSheets",
    "drawingGridHorizontalSpacing",
    "drawingGridVerticalSpacing",
    "displayHorizontalDrawingGridEvery",
    "displayVerticalDrawingGridEvery",
    "doNotUseMarginsForDrawingGridOrigin",
    "drawingGridHorizontalOrigin",
    "drawingGridVerticalOrigin",
    "doNotShadeFormData",
    "noPunctuationKerning",
    "characterSpacingControl",
    "printTwoOnOne",
    "strictFirstAndLastChars",
    "noLineBreaksAfter",
    "noLineBreaksBefore",
    "savePreviewPicture",
    "doNotValidateAgainstSchema",
    "saveInvalidXml",
    "ignoreMixedContent",
    "alwaysShowPlaceholderText",
    "doNotDemarcateInvalidXml",
    "saveXmlDataOnly",
    "useXSLTWhenSaving",
    "saveThroughXslt",
    "showXMLTags",
    "alwaysMergeEmptyNamespace",
    "updateFields",
    "hdrShapeDefaults",
    "footnotePr",
    "endnotePr",
    "compat",
    "docVars",
    "rsids",
    "mathPr",
    "attachedSchema",
    "themeFontLang",
    "clrSchemeMapping",
    "doNotIncludeSubdocsInStats",
    "doNotAutoCompressPictures",
    "forceUpgrade",
    "captions",
    "readModeInkLockDown",
    "smartTagType",
    "schemaLibrary",
    "shapeDefaults",
    "doNotEmbedSmartTags",
    "decimalSymbol",
    "listSeparator",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_becomes_runs_with_tabs_and_breaks() {
        assert_eq!(
            run_text("a\tb\n c"),
            "<w:t>a</w:t><w:tab/><w:t>b</w:t><w:br/><w:t xml:space=\"preserve\"> c</w:t>"
        );
        assert_eq!(text("<&>"), "&lt;&amp;&gt;");
        assert_eq!(attr("\"a\"\n"), "&quot;a&quot;&#10;");
    }
}
