import { test } from "node:test";
import assert from "node:assert/strict";
import { buildIndex, search } from "../src/search.js";

const lib = {
  mueller2020: { id: "mueller2020", title: "Overall survival with drug X in colorectal cancer", author: [{ family: "Müller", given: "Anna" }, { family: "Smith", given: "John" }], "container-title": "N Engl J Med", issued: { "date-parts": [[2020]] }, DOI: "10.1056/NEJMoa2001" },
  smith2019: { id: "smith2019", title: "Quality of life in metastatic disease", author: [{ family: "Smith", given: "John" }], "container-title": "Lancet Oncol", issued: { "date-parts": [[2019]] }, PMID: "31234567" },
  csr: { id: "csr", title: "Clinical study report XY-123", author: [{ literal: "Muster Pharma GmbH" }], issued: { "date-parts": [[2023]] } },
  schmidt2021: { id: "schmidt2021", title: "Mueller cells and the retina", author: [{ family: "Schmidt", given: "Eva" }], issued: { "date-parts": [[2021]] } },
};
const idx = buildIndex(lib);
const ids = (q) => search(idx, q).map((x) => x.id);

test("an empty query lists everything, newest first", () => {
  assert.deepEqual(ids(""), ["csr", "schmidt2021", "mueller2020", "smith2019"]);
});

test("every word must match; first authors rank above co-authors", () => {
  assert.deepEqual(ids("smith"), ["smith2019", "mueller2020"]);
  assert.deepEqual(ids("smith 2020"), ["mueller2020"]);
  assert.deepEqual(ids("smith lancet"), ["smith2019"]);
});

test("umlauts, prefixes and typos", () => {
  assert.equal(ids("müller")[0], "mueller2020");
  assert.equal(ids("muel")[0], "mueller2020");
  assert.equal(ids("mueler")[0], "mueller2020");
  assert.deepEqual(ids("colrectal"), ["mueller2020"]);
});

test("an author outranks the same word in a title", () => {
  assert.deepEqual(ids("mueller"), ["mueller2020", "schmidt2021"]);
});

test("keys, institutions, DOI and PMID", () => {
  assert.equal(ids("csr")[0], "csr");
  assert.deepEqual(ids("muster pharma"), ["csr"]);
  assert.deepEqual(ids("10.1056/nejmoa2001"), ["mueller2020"]);
  assert.deepEqual(ids("31234567"), ["smith2019"]);
  assert.deepEqual(ids("nothing like this"), []);
});
