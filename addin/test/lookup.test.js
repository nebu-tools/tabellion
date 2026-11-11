import { test } from "node:test";
import assert from "node:assert/strict";
import { lookup, parseIdent, cleanCsl, fromEsummary } from "../src/lookup.js";

// A real esummary record, shortened.
const SUMMARY = {
  uid: "25265492", pubdate: "2014 Nov 13", source: "N Engl J Med", title: "Combined BRAF and MEK inhibition versus BRAF inhibition alone in melanoma.",
  volume: "371", issue: "20", pages: "1877-88",
  authors: [{ name: "Long GV", authtype: "Author" }, { name: "Stroyakovskiy D", authtype: "Author" }, { name: "COMBI-d Study Group", authtype: "CollectiveName" }],
  articleids: [{ idtype: "pubmed", value: "25265492" }, { idtype: "doi", value: "10.1056/NEJMoa1406037" }],
};

test("DOIs and PMIDs in their usual spellings", () => {
  assert.deepEqual(parseIdent("https://doi.org/10.1056/NEJMoa1406498."), { doi: "10.1056/NEJMoa1406498" });
  assert.deepEqual(parseIdent("doi:10.1000/xyz(1)"), { doi: "10.1000/xyz(1)" });
  assert.deepEqual(parseIdent("PMID: 25265492"), { pmid: "25265492" });
  assert.equal(parseIdent("Müller 2020"), null);
});

test("PubMed summaries become CSL", () => {
  assert.deepEqual(fromEsummary(SUMMARY), {
    type: "article-journal", PMID: "25265492", title: "Combined BRAF and MEK inhibition versus BRAF inhibition alone in melanoma",
    "container-title": "N Engl J Med", volume: "371", issue: "20", page: "1877-88", DOI: "10.1056/NEJMoa1406037",
    author: [{ family: "Long", given: "GV" }, { family: "Stroyakovskiy", given: "D" }, { literal: "COMBI-d Study Group" }],
    issued: { "date-parts": [[2014, 11, 13]] },
  });
});

test("Crossref CSL loses what CSL does not know", () => {
  assert.equal(cleanCsl({ type: "journal-article" }).type, "article-journal");
  const c = cleanCsl({ type: "article", title: ["T"], author: [{ family: "A", given: "B", affiliation: [], sequence: "first" }], reference: [{}], "container-title": "J", issued: { "date-parts": [[2020, 1]], timestamp: 1 } });
  assert.deepEqual(c, { type: "article-journal", title: "T", author: [{ family: "A", given: "B" }], "container-title": "J", issued: { "date-parts": [[2020, 1]] } });
});

test("lookup asks doi.org for CSL and E-utilities for PMIDs", async () => {
  const calls = [];
  const fake = async (url, opts) => {
    calls.push([url, opts?.headers?.Accept]);
    if (url.startsWith("https://doi.org/")) return { ok: true, json: async () => ({ type: "article-journal", title: "T", DOI: "10.1056/x" }) };
    return { ok: true, json: async () => ({ result: { 25265492: SUMMARY, 1: { uid: "1", error: "cannot get document summary" } } }) };
  };
  assert.equal((await lookup("10.1056/x", fake)).title, "T");
  assert.equal((await lookup("25265492", fake)).PMID, "25265492");
  await assert.rejects(lookup("1", fake), /PMID nicht gefunden/);
  assert.deepEqual(calls[0], ["https://doi.org/10.1056/x", "application/vnd.citationstyles.csl+json"]);
  assert.match(calls[1][0], /^https:\/\/eutils\.ncbi\.nlm\.nih\.gov\/entrez\/eutils\/esummary\.fcgi\?db=pubmed&retmode=json&id=25265492$/);
  assert.equal(calls[1][1], undefined);
});
