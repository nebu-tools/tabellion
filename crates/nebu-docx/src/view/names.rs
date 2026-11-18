//! The names of Word's built-in styles.
//!
//! Word stores a built-in style under its English name in lower or mixed
//! case (`heading 1`, `Title`) whatever the language of its interface, and
//! shows it under another name: `Heading 1` in English, `Überschrift 1` in
//! German, `Titre 1` in French. Each row gives the stored name, the English
//! name Word shows, the German name and the French name. DESIGN.md says where
//! the names come from.

/// Stored name, English, German, French.
pub(crate) const BUILT_IN: &[(&str, &str, &str, &str)] = &[
    ("Normal", "Normal", "Standard", "Normal"),
    ("heading 1", "Heading 1", "Überschrift 1", "Titre 1"),
    ("heading 2", "Heading 2", "Überschrift 2", "Titre 2"),
    ("heading 3", "Heading 3", "Überschrift 3", "Titre 3"),
    ("heading 4", "Heading 4", "Überschrift 4", "Titre 4"),
    ("heading 5", "Heading 5", "Überschrift 5", "Titre 5"),
    ("heading 6", "Heading 6", "Überschrift 6", "Titre 6"),
    ("heading 7", "Heading 7", "Überschrift 7", "Titre 7"),
    ("heading 8", "Heading 8", "Überschrift 8", "Titre 8"),
    ("heading 9", "Heading 9", "Überschrift 9", "Titre 9"),
    ("Title", "Title", "Titel", "Titre"),
    ("Subtitle", "Subtitle", "Untertitel", "Sous-titre"),
    ("caption", "Caption", "Beschriftung", "Légende"),
    (
        "footnote text",
        "Footnote Text",
        "Fußnotentext",
        "Note de bas de page",
    ),
    (
        "footnote reference",
        "Footnote Reference",
        "Fußnotenzeichen",
        "Appel note de bas de p.",
    ),
    (
        "endnote text",
        "Endnote Text",
        "Endnotentext",
        "Note de fin",
    ),
    (
        "endnote reference",
        "Endnote Reference",
        "Endnotenzeichen",
        "Appel de note de fin",
    ),
    (
        "annotation text",
        "Comment Text",
        "Kommentartext",
        "Commentaire",
    ),
    (
        "annotation reference",
        "Comment Reference",
        "Kommentarzeichen",
        "Marque de commentaire",
    ),
    (
        "annotation subject",
        "Comment Subject",
        "Kommentarthema",
        "Objet du commentaire",
    ),
    ("header", "Header", "Kopfzeile", "En-tête"),
    ("footer", "Footer", "Fußzeile", "Pied de page"),
    ("page number", "Page Number", "Seitenzahl", "Numéro de page"),
    (
        "line number",
        "Line Number",
        "Zeilennummer",
        "Numéro de ligne",
    ),
    ("toc 1", "TOC 1", "Verzeichnis 1", "TM 1"),
    ("toc 2", "TOC 2", "Verzeichnis 2", "TM 2"),
    ("toc 3", "TOC 3", "Verzeichnis 3", "TM 3"),
    ("toc 4", "TOC 4", "Verzeichnis 4", "TM 4"),
    ("toc 5", "TOC 5", "Verzeichnis 5", "TM 5"),
    ("toc 6", "TOC 6", "Verzeichnis 6", "TM 6"),
    ("toc 7", "TOC 7", "Verzeichnis 7", "TM 7"),
    ("toc 8", "TOC 8", "Verzeichnis 8", "TM 8"),
    ("toc 9", "TOC 9", "Verzeichnis 9", "TM 9"),
    (
        "TOC Heading",
        "TOC Heading",
        "Inhaltsverzeichnisüberschrift",
        "En-tête de table des matières",
    ),
    (
        "table of figures",
        "Table of Figures",
        "Abbildungsverzeichnis",
        "Table des illustrations",
    ),
    ("List", "List", "Liste", "Liste"),
    (
        "List Bullet",
        "List Bullet",
        "Aufzählungszeichen",
        "Liste à puces",
    ),
    (
        "List Bullet 2",
        "List Bullet 2",
        "Aufzählungszeichen 2",
        "Liste à puces 2",
    ),
    (
        "List Bullet 3",
        "List Bullet 3",
        "Aufzählungszeichen 3",
        "Liste à puces 3",
    ),
    (
        "List Number",
        "List Number",
        "Listennummer",
        "Liste à numéros",
    ),
    (
        "List Number 2",
        "List Number 2",
        "Listennummer 2",
        "Liste à numéros 2",
    ),
    (
        "List Number 3",
        "List Number 3",
        "Listennummer 3",
        "Liste à numéros 3",
    ),
    (
        "List Paragraph",
        "List Paragraph",
        "Listenabsatz",
        "Paragraphe de liste",
    ),
    ("Quote", "Quote", "Zitat", "Citation"),
    (
        "Intense Quote",
        "Intense Quote",
        "Intensives Zitat",
        "Citation intense",
    ),
    ("Body Text", "Body Text", "Textkörper", "Corps de texte"),
    (
        "Body Text 2",
        "Body Text 2",
        "Textkörper 2",
        "Corps de texte 2",
    ),
    (
        "Body Text 3",
        "Body Text 3",
        "Textkörper 3",
        "Corps de texte 3",
    ),
    (
        "No Spacing",
        "No Spacing",
        "Kein Leerraum",
        "Sans interligne",
    ),
    ("Hyperlink", "Hyperlink", "Hyperlink", "Lien hypertexte"),
    (
        "FollowedHyperlink",
        "FollowedHyperlink",
        "BesuchterLink",
        "Lien hypertexte visité",
    ),
    ("Strong", "Strong", "Fett", "Élevé"),
    ("Emphasis", "Emphasis", "Hervorhebung", "Accentuation"),
    (
        "Subtle Emphasis",
        "Subtle Emphasis",
        "Schwache Hervorhebung",
        "Accentuation légère",
    ),
    (
        "Intense Emphasis",
        "Intense Emphasis",
        "Intensive Hervorhebung",
        "Accentuation intense",
    ),
    (
        "Subtle Reference",
        "Subtle Reference",
        "Schwacher Verweis",
        "Référence légère",
    ),
    (
        "Intense Reference",
        "Intense Reference",
        "Intensiver Verweis",
        "Référence intense",
    ),
    ("Book Title", "Book Title", "Buchtitel", "Titre du livre"),
    (
        "Default Paragraph Font",
        "Default Paragraph Font",
        "Absatz-Standardschriftart",
        "Police par défaut",
    ),
    (
        "Normal Table",
        "Normal Table",
        "Normale Tabelle",
        "Tableau Normal",
    ),
    (
        "Table Grid",
        "Table Grid",
        "Tabellenraster",
        "Grille du tableau",
    ),
    ("No List", "No List", "Keine Liste", "Aucune liste"),
    (
        "Balloon Text",
        "Balloon Text",
        "Sprechblasentext",
        "Texte de bulles",
    ),
    ("Plain Text", "Plain Text", "Nur Text", "Texte brut"),
    (
        "Placeholder Text",
        "Placeholder Text",
        "Platzhaltertext",
        "Texte de l’espace réservé",
    ),
    (
        "Bibliography",
        "Bibliography",
        "Literaturverzeichnis",
        "Bibliographie",
    ),
    ("Revision", "Revision", "Überarbeitung", "Révision"),
    (
        "Normal (Web)",
        "Normal (Web)",
        "Standard (Web)",
        "Normal (Web)",
    ),
    ("Date", "Date", "Datum", "Date"),
    ("Signature", "Signature", "Unterschrift", "Signature"),
];

/// The row for a stored name, compared without case.
pub(crate) fn built_in(
    stored: &str,
) -> Option<&'static (&'static str, &'static str, &'static str, &'static str)> {
    BUILT_IN.iter().find(|r| r.0.eq_ignore_ascii_case(stored))
}

/// The languages of the table after English, as language tags.
pub(crate) const LANGUAGES: [&str; 2] = ["de", "fr"];
