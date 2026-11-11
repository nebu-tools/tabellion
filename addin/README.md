# Zitate: citations in Word

A Word add-in for citing sources in dossiers, as a replacement for EndNote's Cite While You Write. It works like citations in LaTeX: one library, a search box, and numbers and reference lists that keep themselves current.

## Use

Open **Zitate** on the Home tab.

- **Cite.** Type any part of an author, year, title, journal, key, DOI, or PMID. The search tolerates typos and umlauts (`mueller`, `müller`, `mueler`). Enter cites the highlighted work at the cursor. Ctrl+Enter marks a work, and Enter then cites all marked works together, for example `[2,5]`. **Seite** adds a page to a single work.
- **Add to a citation.** Put the cursor in a citation and pick a work. It joins that citation.
- **Type it.** Write `[@mueller2020]` or `[@a; @b, S. 12]` in the text, then click **Aktualisieren**. The markers become citations. A marker with an unknown key stays as text and shows up in the pane. The R pipeline can write markers too.
- **Reference lists.** Select the `<< Angaben des pharmazeutischen Unternehmers >>` placeholder under a "Referenzliste" heading and click **Literaturverzeichnis einfügen**. Each list holds the citations since the previous list and counts from 1, as Modul 3 needs for its six lists. Citations after the last list, for example in appendices, belong to it.
- **Sources.** Under **Quellen hinzufügen**, import a `.bib` file, a `.ris` file (EndNote: File › Export, style "RefMan (RIS) Export"), or CSL JSON (Zotero), or fetch one work by DOI or PMID. The library is stored in the document, so everyone who opens the file has it. A second import updates entries. A known work keeps its key, matched by DOI, PMID, or title and year.

Numbers and lists update after every citation and after **Aktualisieren**. They also update by themselves when a citation is deleted or pasted.

## Track changes

A new citation follows the document's track-changes setting. Renumbering and the lists are written without tracking, so one early citation does not send every later number to review. A citation deleted with tracking on stays in the text until the deletion is accepted. Until then it has no number and is not in the list.

## In the docx

| Part | Form |
| --- | --- |
| Citation | Inline content control, tag `zitat:<id>`, title `Zitat: key1; key2` |
| Reference list | Block content control, tag `literaturverzeichnis` |
| Library and citations | Custom XML part in namespace `urn:newga:zitate:1`, holding JSON `{"quellen": {key: CSL-JSON}, "zitate": {id: [{"id": key, "locator"?}]}}` |

Without the add-in, Word shows the last text. Colleagues without it can still read and edit the document. newga's `outline` lists the citations and lists, and `apply` refuses to replace text that holds a citation unless the change sets `"zitate_loeschen": true`.

## Style

NLM/Vancouver, numbered in order of appearance, in brackets, with German terms. This meets the G-BA's "Vancouver oder Harvard". The CSL files in `csl/` come from the [Citation Style Language](https://citationstyles.org) project under CC BY-SA 3.0. For another style, replace the `.csl` file and its import in `src/taskpane.js`.

## Development

```sh
npm ci
npm test                                       # unit tests, no Word needed
npm run build -- --base https://host/zitate    # dist/ for that address
npm run e2e                                    # real Word, see below
```

To try it locally on Windows:

```sh
npm run build -- --base http://localhost:3000
npm run serve
```

Then sideload `dist/manifest.xml`. For Word for Windows, add a string value under `HKCU\Software\Microsoft\Office\16.0\WEF\Developer` that holds the manifest's path, and restart Word. Word for Windows accepts `http://localhost`. Every other address needs HTTPS.

`npm run e2e` needs WSL on a Windows machine with Word, and the templates from `rv run gba.R fetch`. It sideloads the add-in for one Word window at a time, which opens three times. A test page in the pane uses the real pane like a user would, and the checks run in three steps:

1. An empty document: search, cite, markers, lists, deleting, pasting, track changes, and a DOI lookup.
2. The G-BA Modul 3 template: reference lists for sections 3.1 and 3.2.
3. `gba.R apply` rewrites the result, and Word opens it again. The first update must find nothing to change.

## Rollout

1. Host `dist/` on an HTTPS server the team can reach, for example an internal web server or the planned web app.
2. Build with that address: `npm run build -- --base https://…`.
3. A Microsoft 365 admin deploys `dist/manifest.xml` under Integrated apps. The **Zitate** button then appears for the chosen users in Word for Windows, Mac, and the web. Without an admin, a shared folder works as a trusted add-in catalog (Trust Center › Trusted Add-in Catalogs).

## Limits

- Citations in footnotes, endnotes, and headers are not found.
- Existing EndNote citations stay as they are. The add-in does not read or convert them. A converter needs a real dossier with EndNote fields to build and test against.
- There is one style.
- If two people edit the same document at the same time, one person's library changes can overwrite the other's.
- A citation pasted into another document keeps its keys only if they fit into the title, which holds 64 characters. The other document's library also needs those works.
