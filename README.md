<img src="assets/icon.svg" width="64" height="64" alt="">

# G-BA-Dossiers in Typst und R

Mit diesem Werkzeug schreibst du Dossier-Module in Typst und befüllst sie aus R. Es baut daraus eine `.docx` auf Basis der Original-Dokumentvorlage des G-BA. Das Dossier-Team öffnet die Datei in Word und arbeitet dort wie mit einer ausgefüllten Vorlage weiter.

English version: [README.en.md](README.en.md)

Word ist für den Build nicht nötig. Der Build läuft auf Linux.

## Einrichten

Du brauchst `rv`, `typst`, `curl` und `zip`.

```sh
rv sync
rv run gba.R fetch
```

`fetch` lädt die Vorlagen vom G-BA nach `templates/` und prüft ihre Prüfsummen. Dieses Repository enthält keine Vorlagen. Es verweist in [templates.csv](templates.csv) nur auf ihre Download-Adressen.

## Modul erstellen

1. Erzeuge ein Typst-Projekt:

   ```sh
   rv run gba.R import modul4 dossier/m4
   ```

2. Bearbeite die erzeugte `.typ`-Datei in `dossier/m4/`.
3. Baue die docx:

   ```sh
   rv run gba.R build dossier/m4/2025-11-18_Anl2_6_Modul4.typ
   ```

Die docx liegt danach als `<name>_gefuellt.docx` neben der Quelle. Der Build nennt die Zahl offener Platzhalter und warnt, wenn eine Fragestellung, Erläuterung oder Überschrift der Vorlage fehlt.

Für eine Lesevorschau beim Schreiben führst du `typst watch` auf der `.typ`-Datei aus. Die Vorschau zeigt den Inhalt, aber nicht das Word-Layout.

Das Beispiel [examples/modul4/demo.R](examples/modul4/demo.R) füllt Modul 4 mit öffentlichen Daten aus `survival::colon`. Es zeigt R-Tabellen, Kaplan-Meier-Plots und einen je Endpunkt wiederholten Abschnitt.

Verfügbare Module: `modul1`, `modul1-checkliste`, `modul2`, `modul3`, `modul4`, `k1`, `k1-checkliste`, `k2`, `k3`, `k4`.

## Seitenzahlen in Word aktualisieren

Der Build schreibt Tabellen- und Abbildungsnummern, Querverweise und die Texte aller Verzeichnisse selbst in die docx. Seitenzahlen kann er nicht berechnen, weil dafür das Word-Layout nötig ist. Neue Einträge in den Verzeichnissen tragen deshalb eine geschätzte Seite.

Bevor du das Einreichungs-PDF erzeugst, drücke in Word `Strg+A` und dann `F9`. Wähle bei jedem Verzeichnis **Gesamtes Verzeichnis aktualisieren**.

Wenn deine Word-Oberfläche nicht deutsch ist, verliert das Inhaltsverzeichnis dabei den Eintrag für Kapitel 1. Der Feldcode nennt die Formatvorlage "Überschrift 1" wörtlich. Dasselbe passiert bei einer von Hand ausgefüllten Vorlage.

## Inhalte in der .typ-Datei ändern

Jeder Block trägt die Kennung seines Gegenstücks in der Vorlage, zum Beispiel `"b0141"`. Lösche keine Fragestellungen und Erläuterungen, denn die G-BA-Vorgaben verlangen sie im Dokument.

Deckblatt, Kopf- und Fußzeilen füllst du über `felder`:

```typst
#show: dossier.with(vorlage: "...", felder: (
	wirkstoff: "...", handelsname: "...", unternehmer: "...",
	kodierung: "A", anwendungsgebiet: "...", stand: "01.12.2026",
))
```

Eine Antwort ersetzt den Platzhalter `<< Angaben des pharmazeutischen Unternehmers >>`. Sie darf Text, Listen, Fußnoten, Tabellen und Abbildungen enthalten:

```typst
#antwort("b0141")[
	Text mit Fußnote#footnote[Quelle].

	- Listenpunkt
]
```

Daten aus R ersetzen alle Zeilen unterhalb des Tabellenkopfs einer Vorlagentabelle. In R schreibst du sie mit `gba_json()`:

```typst
#tabelle("b0430", daten: json("ergebnisse.json").at(0).zeilen)
```

Weicht die Spaltenzahl von der Vorlage ab, übergib `spalten:` mit relativen Breiten.

Neue Tabellen und Abbildungen bekommen eine Beschriftung im Stil der Vorlage:

```typst
#neue-tabelle(beschriftung: [Titel], kopf: ([A], [B]), zeilen: rows)
#abbildung(image("km.png", width: 14cm), beschriftung: [Titel])
```

Einen Abschnitt wiederholst du mit einer Funktion und `mit`. `mit` ersetzt Platzhalter wie `<Endpunkt xxx>` in allen Blöcken des Abschnitts:

```typst
#let abschnitt(e) = mit(("<Endpunkt xxx>": e.name), [
	#ueberschrift(5, "b0381")[\<Endpunkt xxx\> – RCT]
])
#for e in endpunkte { abschnitt(e) }
```

Ein Kontrollkästchen setzt du mit `#kreuz(0, true)`.

`#feld(n)[...]` steht für ein Objekt der Vorlage, etwa einen Querverweis. Der Text in den Klammern ist der gespeicherte Wert. Eine Änderung an diesem Text ändert das Feld nicht.

Für R-Ergebnisse gibt es `gba_zeilen()`, `gba_json()`, `gba_zahl()`, `gba_ki()` und `gba_p()`.

## Warum die Word-Vorlage die Quelle bleibt

Die G-BA-Vorgaben verlangen die Vorlagen, und die Vorlagen enthalten Dinge, die ein Konverter nicht nachbaut: Formularfelder, Rahmen, Abschnitte und Feldstrukturen. Darum erzeugt das Werkzeug keine neue Datei, sondern ändert die Vorlage.

`gba.R import` zerlegt die Vorlage in Blöcke und schreibt sie als Typst. `gba.R build` vergleicht jeden Block mit der Vorlage. Unveränderte Blöcke übernimmt der Build mit ihren Originalbytes. Geänderte Blöcke setzt er mit Formatvorlage und Lesezeichen der Vorlage neu. Eine unveränderte `.typ`-Datei ergibt deshalb eine docx, deren Dateien bytegleich zur Vorlage sind.

## Grenzen

- Nummerierte Listen aus Typst laufen im Dokument weiter und beginnen nicht neu.
- Abbildungen müssen PNG oder JPEG sein.
- Der Build überträgt Formeln als Text mit Hoch- und Tiefstellung.
- Typst-Elemente ohne Word-Gegenstück, etwa `#rect` oder `context`, brechen den Build mit einer Meldung ab.
- Eine `.typ`-Datei gehört zu genau einer Vorlagenversion. Bei einer neuen Vorlage stimmen die Blockkennungen nicht mehr.

## Testen

```sh
rv run tests/testthat.R
```

Die Tests laufen auch in der CI. Ein wöchentlicher Lauf meldet, wenn die G-BA-Seite eine Vorlage aus `templates.csv` nicht mehr verlinkt.

`tools/word_check.R` vergleicht die Feldergebnisse des Builds mit denen von Word. Das Skript braucht Word unter Windows mit WSL und läuft deshalb nicht in der CI.

## Lizenz

[AGPL-3.0](LICENSE). Die Vorlagen des G-BA sind nicht Teil dieses Repositorys und fallen nicht unter diese Lizenz.
