import { test } from "node:test";
import assert from "node:assert/strict";
import { parseLibrary, parseBibtex, parseRis, latexToText, bibNames, mergeItems, makeKey, fold } from "../src/library.js";

test("LaTeX accents, symbols and markup become text", () => {
  assert.equal(latexToText("M{\\\"u}ller and Stra\\ss{}e"), "Müller and Straße");
  assert.equal(latexToText("\\'{e}t\\'e -- {DNA} \\textit{in vitro}"), "été – DNA <i>in vitro</i>");
  assert.equal(latexToText("10~mg \\& more"), "10 mg & more");
});

test("BibTeX names: last-first, first-last, particles, institutions, others", () => {
  assert.deepEqual(bibNames("M{\\\"u}ller, Anna and John B. Smith and van der Berg, Jan and {World Health Organization} and others"), [
    { family: "Müller", given: "Anna" },
    { family: "Smith", given: "John B." },
    { family: "van der Berg", given: "Jan" },
    { literal: "World Health Organization" },
  ]);
  assert.deepEqual(bibNames("Ludwig van Beethoven"), [{ family: "van Beethoven", given: "Ludwig" }]);
});

test("BibTeX entries with macros, comments and quoted values", () => {
  const items = parseBibtex(`
    @string{nejm = "N Engl J Med"}
    @comment{ignored {nested}}
    % a line comment
    @article{mueller2020,
      author = {M{\\"u}ller, Anna and Smith, John},
      title = {Overall survival with {drug X}},
      journal = nejm,
      year = 2020, month = may,
      volume = "382", number = {4}, pages = {301--310},
      doi = {https://doi.org/10.1056/NEJMoa1},
    }
    @techreport{csr, title={Clinical study report}, institution={Muster Pharma}, number={XY-123}, year={2023}}
  `);
  assert.equal(items.length, 2);
  assert.deepEqual(items[0], {
    id: "mueller2020", type: "article-journal", title: "Overall survival with drug X",
    author: [{ family: "Müller", given: "Anna" }, { family: "Smith", given: "John" }],
    "container-title": "N Engl J Med", volume: "382", issue: "4", page: "301-310", DOI: "10.1056/NEJMoa1",
    issued: { "date-parts": [[2020, 5]] },
  });
  assert.equal(items[1].type, "report");
  assert.equal(items[1].number, "XY-123");
  assert.equal(items[1].issue, undefined);
  assert.equal(items[1].publisher, "Muster Pharma");
});

test("RIS records, including EndNote's export", () => {
  const items = parseRis([
    "TY  - JOUR", "AU  - Müller, Anna", "AU  - Smith, John B.", "TI  - Overall survival", "  with drug X",
    "T2  - N Engl J Med", "PY  - 2020/05/01", "VL  - 382", "IS  - 4", "SP  - 301", "EP  - 310",
    "DO  - 10.1056/NEJMoa1", "AN  - 31234567", "ER  - ", "", "TY  - ELEC", "AU  - Muster Pharma GmbH",
    "TI  - Fachinformation Demozil", "UR  - https://www.fachinfo.de/x", "PY  - 2024", "Y2  - 2026/11/02", "ER  - ",
  ].join("\r\n"));
  assert.equal(items.length, 2);
  assert.equal(items[0].title, "Overall survival with drug X");
  assert.equal(items[0].page, "301-310");
  assert.equal(items[0].PMID, "31234567");
  assert.deepEqual(items[0].issued, { "date-parts": [[2020, 5, 1]] });
  assert.equal(items[1].type, "webpage");
  assert.deepEqual(items[1].author, [{ literal: "Muster Pharma GmbH" }]);
  assert.deepEqual(items[1].accessed, { "date-parts": [[2026, 11, 2]] });
});

test("parseLibrary picks the format from name or content", () => {
  assert.equal(parseLibrary('[{"id":"a","title":"T"}]', "x.json")[0].id, "a");
  assert.equal(parseLibrary("TY  - JOUR\nTI  - T\nER  - \n", "export.txt")[0].title, "T");
  assert.equal(parseLibrary("@book{b, title={T}}", "lib.bib")[0].type, "book");
});

test("keys from first author and year, unique with letters", () => {
  const taken = new Set(["mueller2020"]);
  const it = { author: [{ family: "Müller" }], issued: { "date-parts": [[2020]] } };
  assert.equal(makeKey(it, taken), "mueller2020a");
  assert.equal(makeKey({ author: [{ literal: "The World Health Organization" }] }, new Set()), "world");
  assert.equal(fold("Señor Øresund Straße"), "senor øresund strasse");
});

test("merging keeps keys of known works and updates them", () => {
  const lib = { m20: { id: "m20", title: "Overall survival with drug X", DOI: "10.1/A", issued: { "date-parts": [[2020]] } } };
  const r = mergeItems(lib, [
    { title: "Overall survival with drug X (corrected)", DOI: "10.1/a" },
    { id: "m20", title: "Another work with the same key", DOI: "10.1/B", author: [{ family: "Lee" }], issued: { "date-parts": [[2019]] } },
    { title: "No key at all", author: [{ family: "Smith" }], issued: { "date-parts": [[2021]] } },
    { id: "bad key!", title: "Weird key", author: [{ family: "Smith" }], issued: { "date-parts": [[2021]] } },
  ]);
  assert.equal(r.library.m20.title, "Overall survival with drug X (corrected)");
  assert.equal(r.library.lee2019.DOI, "10.1/B");
  assert.equal(r.updated, 1);
  assert.equal(r.added, 3);
  assert.ok(r.library.smith2021);
  assert.ok(r.library.smith2021a);
  // Without ids to compare, an entry under a known key is a correction.
  const fix = mergeItems({ k: { id: "k", title: "Typo" } }, [{ id: "k", title: "Fixed" }]);
  assert.deepEqual(fix.library, { k: { id: "k", title: "Fixed" } });
  for (const [k, v] of Object.entries(r.library)) assert.equal(v.id, k);
});
