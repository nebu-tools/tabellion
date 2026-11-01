// gba.typ: Autoren-API für G-BA-Dossiers.
//
// Jede Funktion erzeugt zwei Dinge:
//  1. Ein metadata-Element mit Label <gba>. Daraus baut die R-Pipeline die docx.
//  2. Eine Typst-Darstellung für die PDF-Vorschau.
// Die Vorschau ist eine Lesehilfe. Maßgeblich ist die aus der Word-Vorlage
// erzeugte docx.

#let _grau = luma(235)
#let _dunkel = luma(170)

// `vorschau` wird nur in der PDF-Vorschau gerendert und nie exportiert.
#let _reg(wert, vorschau: none, ..pos) = {
  let v = if pos.pos().len() > 0 { pos.pos().first() } else { vorschau }
  [#metadata(wert + (vorschau: v)) <gba>]
}

// Inline-Objekt der Vorlage (Feld, Querverweis, Fußnote, Tabulator ...).
// Der Inhalt ist der in der Vorlage zwischengespeicherte Text.
#let feld(key, gecached) = metadata((gba-atom: key, cached: gecached))

// Formularfeld-Kontrollkästchen der Vorlage.
#let kreuz(key, an) = metadata((
  gba-atom: key,
  on: an,
  cached: if an { [☒] } else { [☐] },
))

// Blockweite Platzhalter-Ersetzung, z. B. für wiederholte Endpunkt-Abschnitte.
#let _ersetze(paare, inhalt) = {
  if paare.len() == 0 {
    inhalt
  } else {
    show paare.first().at(0): paare.first().at(1)
    _ersetze(paare.slice(1), inhalt)
  }
}

#let mit(ersetzungen, inhalt) = {
  _reg((kind: "mit-start", ersetzungen: ersetzungen))
  _ersetze(ersetzungen.pairs(), inhalt)
  _reg((kind: "mit-ende"))
}

#let absatz(id, inhalt) = _reg((kind: "block", id: id, body: inhalt), par(inhalt))

#let ueberschrift(ebene, id, inhalt) = _reg(
  (kind: "block", id: id, body: inhalt),
  heading(level: ebene, inhalt),
)

#let ueberschrift-frei(ebene, id, inhalt) = _reg(
  (kind: "block", id: id, body: inhalt),
  heading(level: ebene, numbering: none, inhalt),
)

#let frage(id, inhalt) = _reg((kind: "block", id: id, body: inhalt), par(emph(inhalt)))

#let erlaeuterung(id, inhalt) = _reg(
  (kind: "block", id: id, body: inhalt),
  block(fill: _grau, stroke: 0.5pt + _dunkel, inset: 5pt, width: 100%, text(size: 9.5pt, inhalt)),
)

#let beschriftung(id, inhalt) = _reg(
  (kind: "block", id: id, body: inhalt),
  block(sticky: true, text(weight: "bold", size: 10pt, inhalt)),
)

// Ohne Inhalt bleibt der Platzhalter << Angaben ... >> der Vorlage stehen.
#let antwort(id, ..pos) = {
  let body = if pos.pos().len() > 0 { pos.pos().first() } else { none }
  _reg(
    (kind: "antwort", id: id, body: body),
    if body == none { box(fill: _dunkel, inset: 2pt)[\<\< Angaben des pharmazeutischen Unternehmers \>\>] } else { body },
  )
}

#let _spalten(zeilen) = calc.max(1, ..zeilen.map(z => z.len()))

// Vollstaendige Tabelle: jede Zeile entspricht einer Vorlagenzeile.
// Mit `daten:` werden alle Zeilen unterhalb des Kopfes ersetzt; die erste
// Datenzeile der Vorlage liefert die Formatierung, `zeile:` wählt eine andere.
// `spalten:` gibt relative Spaltenbreiten vor, wenn sich die Spaltenzahl ändert.
#let tabelle(id, kopf: auto, daten: none, zeile: auto, spalten: none, ..zeilen) = {
  let rows = zeilen.pos()
  let alle = if daten == none { rows } else { rows + daten }
  _reg(
    (
      kind: "tabelle", id: id, rows: rows, daten: daten,
      kopf: if kopf == auto { none } else { kopf },
      zeile: if zeile == auto { none } else { zeile },
      spalten: spalten,
    ),
    {
      set text(size: 9pt)
      if alle.len() > 0 {
        let n = _spalten(alle)
        table(columns: n, ..alle.map(z => z + ([],) * (n - z.len())).flatten())
      }
    },
  )
}

// Neue Tabelle mit Beschriftung im Stil der Vorlage (Beschriftung + SEQ-Feld).
#let neue-tabelle(beschriftung: none, kopf: (), zeilen: (), vorlage: none, spalten: none) = _reg(
  (kind: "neue-tabelle", titel: beschriftung, kopf: kopf, zeilen: zeilen,
   vorlage: vorlage, spalten: spalten),
  {
    block(sticky: true, text(weight: "bold", size: 10pt)[Tabelle: #beschriftung])
    set text(size: 9pt)
    table(columns: _spalten((kopf,) + zeilen), table.header(..kopf), ..zeilen.flatten())
  },
)

// Neue Abbildung, z. B. ein Kaplan-Meier- oder Forest-Plot aus R.
#let abbildung(bild, beschriftung: none) = _reg(
  (kind: "abbildung", bild: bild, titel: beschriftung),
  figure(bild, caption: beschriftung),
)

// Verzeichnisse aktualisiert Word; die Vorschau zeigt nur einen Hinweis.
#let verzeichnis(von, bis) = _reg(
  (kind: "bereich", von: von, bis: bis),
  [_(Verzeichnis, wird von Word aktualisiert)_],
)

#let dossier(
  vorlage: none,
  felder: (:),
  kapitel: 1,
  praefix: "",
  inhalt,
) = {
  show metadata: it => {
    if type(it.value) != dictionary {
      it
    } else if "gba-atom" in it.value {
      it.value.cached
    } else if "vorschau" in it.value {
      it.value.vorschau
    } else {
      it
    }
  }
  set document(title: felder.at("wirkstoff", default: "Dossier"))
  set text(lang: "de", font: ("Arial", "Liberation Sans"), size: 11pt)
  set par(justify: false)
  set page(paper: "a4", margin: (x: 2.5cm, y: 3cm), numbering: "1")
  set heading(numbering: (..n) => praefix + n.pos().map(str).join("."))
  show heading: set text(size: 12pt)
  counter(heading).update(kapitel - 1)
  _reg((kind: "konfig", vorlage: vorlage, felder: felder))
  inhalt
}
