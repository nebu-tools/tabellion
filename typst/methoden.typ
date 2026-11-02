// Textbausteine für Methodenabschnitte. Jeder Baustein nimmt ein Dictionary mit
// Werten aus der Auswertung. Ein fehlender Wert bricht den Lauf ab.
// Die Texte sind Platzhalter, bis das Dossierteam echte Formulierungen liefert.

#let bausteine = (
  ereigniszeit: w => [
    Lorem ipsum dolor sit amet, *consectetur* adipiscing elit. Analysiert wurden #w.n Personen aus #w.studie.

    Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua:

    + Ut enim ad minim veniam
    + Quis nostrud exercitation
  ],
  subgruppen: w => [
    Duis aute irure dolor in reprehenderit, $p < #w.alpha$, in voluptate velit esse.#footnote[Excepteur sint occaecat.]
  ],
)
