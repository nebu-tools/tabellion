//! Styles by their own identity: `w:styleId`, `w:name`, and the names Word
//! shows for built-in styles in other languages.

use std::collections::HashMap;

use crate::xml::Tree;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StyleKind {
    Paragraph,
    Character,
    Table,
    Numbering,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Style {
    pub id: String,
    /// As stored. Built-in styles store their English name ("heading 1").
    pub name: String,
    pub kind: StyleKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub based_on: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline_level: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_id: Option<String>,
    pub custom: bool,
    pub default: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Styles {
    pub list: Vec<Style>,
    by_id: HashMap<String, usize>,
}

/// Word's built-in names in other UI languages. Word writes the English name
/// into `w:name` for built-in styles, but people know them by the name their
/// Word shows. Extend per language as needed.
const LOCALISED: &[(&str, &[&str])] = &[
    ("normal", &["Standard", "Normal"]),
    ("heading 1", &["Überschrift 1", "Titre 1", "Titolo 1", "Título 1", "Kop 1"]),
    ("heading 2", &["Überschrift 2", "Titre 2", "Titolo 2", "Título 2", "Kop 2"]),
    ("heading 3", &["Überschrift 3", "Titre 3", "Titolo 3", "Título 3", "Kop 3"]),
    ("heading 4", &["Überschrift 4", "Titre 4", "Titolo 4", "Título 4", "Kop 4"]),
    ("caption", &["Beschriftung", "Légende", "Didascalia", "Epígrafe", "Bijschrift"]),
    ("body text", &["Textkörper", "Corps de texte", "Corpo testo", "Texto independiente", "Plattetekst"]),
    ("list bullet", &["Aufzählungszeichen", "Liste à puces", "Puntato", "Listavieta", "Lijstopsommingteken"]),
    ("list number", &["Listennummer", "Liste numéros", "Numero elenco", "Listanúmero", "Lijstnummering"]),
    ("footnote text", &["Fußnotentext", "Note de bas de page", "Testo nota a piè di pagina"]),
    ("footnote reference", &["Fußnotenzeichen", "Appel note de bas de p.", "Rimando nota a piè di pagina"]),
    ("table grid", &["Tabellenraster", "Grille du tableau", "Griglia tabella"]),
    ("hyperlink", &["Hyperlink", "Lien hypertexte", "Collegamento ipertestuale"]),
    ("toc 1", &["Verzeichnis 1", "TM 1", "Sommario 1"]),
    ("toc 2", &["Verzeichnis 2", "TM 2", "Sommario 2"]),
    ("table of figures", &["Abbildungsverzeichnis", "Table des illustrations", "Indice delle figure"]),
];

impl Styles {
    pub fn parse(t: &Tree) -> Styles {
        let root = t.document_element();
        let mut list = vec![];
        for s in t.elements(root).filter(|&c| t.is(c, "w:style")) {
            let kind = match t.attr(s, "w:type").as_deref() {
                Some("character") => StyleKind::Character,
                Some("table") => StyleKind::Table,
                Some("numbering") => StyleKind::Numbering,
                _ => StyleKind::Paragraph,
            };
            let val = |name: &str| t.child(s, name).and_then(|n| t.attr(n, "w:val"));
            let ppr = t.child(s, "w:pPr");
            let outline_level =
                ppr.and_then(|p| t.child(p, "w:outlineLvl")).and_then(|n| t.attr(n, "w:val")).and_then(|v| v.parse().ok());
            let num_id = ppr
                .and_then(|p| t.child(p, "w:numPr"))
                .and_then(|n| t.child(n, "w:numId"))
                .and_then(|n| t.attr(n, "w:val"));
            let id = t.attr(s, "w:styleId").unwrap_or_default();
            list.push(Style {
                name: val("w:name").unwrap_or_else(|| id.clone()),
                id,
                kind,
                based_on: val("w:basedOn"),
                outline_level,
                num_id,
                custom: matches!(t.attr(s, "w:customStyle").as_deref(), Some("1") | Some("true")),
                default: matches!(t.attr(s, "w:default").as_deref(), Some("1") | Some("true")),
            });
        }
        let by_id = list.iter().enumerate().map(|(i, s)| (s.id.clone(), i)).collect();
        Styles { list, by_id }
    }

    pub fn get(&self, id: &str) -> Option<&Style> {
        self.by_id.get(id).map(|&i| &self.list[i])
    }

    /// Finds a style by id, by stored name, or by a localised built-in name,
    /// ignoring case. `kind` narrows the search.
    pub fn resolve(&self, key: &str, kind: Option<StyleKind>) -> Option<&Style> {
        let ok = |s: &&Style| kind.is_none_or(|k| s.kind == k);
        if let Some(s) = self.get(key).filter(ok) {
            return Some(s);
        }
        let lower = key.to_lowercase();
        if let Some(s) = self.list.iter().filter(ok).find(|s| s.name.to_lowercase() == lower) {
            return Some(s);
        }
        for (builtin, names) in LOCALISED {
            if names.iter().any(|n| n.to_lowercase() == lower) {
                if let Some(s) = self.list.iter().filter(ok).find(|s| s.name.eq_ignore_ascii_case(builtin)) {
                    return Some(s);
                }
            }
        }
        None
    }

    /// Outline level + 1 for headings, through `basedOn`.
    pub fn heading_level(&self, id: &str) -> Option<u8> {
        let mut cur = self.get(id);
        let mut guard = 0;
        while let Some(s) = cur {
            if let Some(l) = s.outline_level {
                return (l < 9).then_some(l + 1);
            }
            if let Some(n) = s.name.to_lowercase().strip_prefix("heading ") {
                return n.parse().ok();
            }
            cur = s.based_on.as_deref().and_then(|b| self.get(b));
            guard += 1;
            if guard > 32 {
                break;
            }
        }
        None
    }

    /// The numbering instance a paragraph style pulls in, through `basedOn`.
    pub fn num_id(&self, id: &str) -> Option<String> {
        let mut cur = self.get(id);
        for _ in 0..32 {
            let s = cur?;
            if s.num_id.is_some() {
                return s.num_id.clone();
            }
            cur = s.based_on.as_deref().and_then(|b| self.get(b));
        }
        None
    }

    pub fn of_kind(&self, kind: StyleKind) -> impl Iterator<Item = &Style> {
        self.list.iter().filter(move |s| s.kind == kind)
    }
}
