import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { createRenderer, htmlToRuns, runsText } from "../src/render.js";
import { plan, parseMarker, storeToXml, storeFromXml, citeTitle, keysFromTitle, BIB_EMPTY, CITE_NEW } from "../src/model.js";

const csl = (f) => fs.readFileSync(new URL("../csl/" + f, import.meta.url), "utf8");
const renderer = createRenderer({
  style: csl("nlm-citation-sequence-brackets.csl"),
  locales: { "de-DE": csl("locales-de-DE.xml"), "en-US": csl("locales-en-US.xml") },
});

const quellen = {
  a: { id: "a", type: "article-journal", title: "Overall survival with <i>drug X</i>", author: [{ family: "Müller", given: "Anna" }], "container-title": "N Engl J Med", issued: { "date-parts": [[2020]] }, volume: "382", page: "301-310" },
  b: { id: "b", type: "report", title: "Clinical study report XY-123", author: [{ literal: "Muster Pharma GmbH" }], issued: { "date-parts": [[2023]] } },
  c: { id: "c", type: "book", title: "Statistics", author: [{ family: "Lee", given: "K" }], issued: { "date-parts": [[2010]] }, publisher: "Springer", "publisher-place": "Berlin" },
};
const store = (zitate) => ({ quellen, zitate });
const cite = (id, current = "[?]", extra = {}) => ({ kind: "cite", tag: "zitat:" + id, title: "", current, original: current, ...extra });
const bib = (current = "") => ({ kind: "bib", current });

test("htmlToRuns keeps italics, superscript and entities", () => {
  assert.deepEqual(htmlToRuns("Overall <i>drug &#38; X</i><sup>2</sup> &#60;3"), [
    { text: "Overall " }, { text: "drug & X", italic: true }, { text: "2", superscript: true }, { text: " <3" },
  ]);
});

test("each reference list numbers its own section from 1", () => {
  const controls = [cite("1"), cite("2"), bib(), cite("3"), cite("4"), bib()];
  const p = plan(controls, store({ 1: [{ id: "b" }], 2: [{ id: "a" }, { id: "b" }], 3: [{ id: "c" }], 4: [{ id: "a" }] }), renderer);
  assert.deepEqual(p.citations.map((c) => [c.index, runsText(c.runs)]), [[0, "[1]"], [1, "[1,2]"], [3, "[1]"], [4, "[2]"]]);
  assert.equal(p.bibs.length, 2);
  const lists = p.bibs.map((b) => b.entries.map(runsText));
  assert.match(lists[0][0], /^1\.\tMuster Pharma GmbH\. Clinical study report XY-123/);
  assert.match(lists[0][1], /^2\.\tMüller A\. Overall survival with drug X\. N Engl J Med\. 2020;382:301–10\.$/);
  assert.deepEqual(p.bibs[0].entries[1].find((r) => r.italic), { text: "drug X", italic: true });
  assert.match(lists[1][0], /^1\.\tLee K\. Statistics\. Berlin: Springer; 2010/);
  assert.equal(p.sections, 2);
  assert.equal(p.cited, 4);
});

test("citations after the last list join it; without a list all are numbered", () => {
  const p = plan([cite("1"), bib(), cite("2")], store({ 1: [{ id: "a" }], 2: [{ id: "b" }] }), renderer);
  assert.deepEqual(p.citations.map((c) => runsText(c.runs)), ["[1]", "[2]"]);
  assert.equal(p.bibs[0].entries.length, 2);
  const q = plan([cite("1"), cite("2")], store({ 1: [{ id: "a" }], 2: [{ id: "a" }] }), renderer);
  assert.deepEqual(q.citations.map((c) => runsText(c.runs)), ["[1]", "[1]"]);
  assert.deepEqual(q.bibs, []);
});

test("nothing to write when the document is current", () => {
  const first = plan([cite("1"), bib()], store({ 1: [{ id: "c" }] }), renderer);
  const text = first.bibs[0].entries.map(runsText).join("\r");
  const again = plan([cite("1", "[1]"), bib(text)], store({ 1: [{ id: "c" }] }), renderer);
  assert.deepEqual([again.citations, again.bibs, again.removes], [[], [], []]);
});

test("an empty list says so", () => {
  const p = plan([bib("old")], store({}), renderer);
  assert.deepEqual(p.bibs[0].entries, [[{ text: BIB_EMPTY }]]);
});

test("emptied citations go away, tracked deletions wait", () => {
  const p = plan(
    [cite("1", ""), cite("2", " ", { original: "" }), cite("3", "", { original: "[1]" }), cite("4", CITE_NEW), bib()],
    store({ 1: [{ id: "a" }], 2: [{ id: "a" }], 3: [{ id: "a" }], 4: [{ id: "b" }] }),
    renderer,
  );
  assert.deepEqual(p.removes, [0, 1]);
  assert.equal(p.pending, 1);
  assert.deepEqual(p.citations.map((c) => [c.index, runsText(c.runs)]), [[3, "[1]"]]);
  assert.equal(p.bibs[0].entries.length, 1);
});

test("a citation pasted from another document is recovered from its title", () => {
  const p = plan([{ ...cite("x9", "[7]"), title: citeTitle([{ id: "b" }, { id: "a" }]) }], store({}), renderer);
  assert.deepEqual(p.store.zitate.x9, [{ id: "b" }, { id: "a" }]);
  assert.equal(runsText(p.citations[0].runs), "[1,2]");
  const lost = plan([{ ...cite("y", "[7]"), title: citeTitle([{ id: "zz" }]) }], store({}), renderer);
  assert.deepEqual(lost.unknown, [{ index: 0, keys: ["zz"], text: "[7]" }]);
  assert.deepEqual(lost.citations, []);
});

test("titles round-trip keys unless cut short", () => {
  assert.deepEqual(keysFromTitle(citeTitle([{ id: "a" }, { id: "b2" }])), ["a", "b2"]);
  const long = citeTitle(Array.from({ length: 12 }, (_, i) => ({ id: "mueller202" + i })));
  assert.equal(long.length, 64);
  assert.equal(keysFromTitle(long), null);
});

test("typed markers", () => {
  assert.deepEqual(parseMarker("[@a]"), [{ id: "a" }]);
  assert.deepEqual(parseMarker("[@a; @b2020, S. 12-14]"), [{ id: "a" }, { id: "b2020", locator: "12-14", label: "page" }]);
  assert.equal(parseMarker("[@a; b]"), null);
  assert.equal(parseMarker("[email]"), null);
});

test("the store survives XML", () => {
  const s = { quellen: { a: { id: "a", title: "A < B & C > \"D\"" } }, zitate: { z: [{ id: "a" }] } };
  const xml = storeToXml(s);
  assert.match(xml, /^<\?xml[^>]*\?><zitate xmlns="urn:newga:zitate:1">/);
  assert.deepEqual(storeFromXml(xml), s);
  assert.deepEqual(storeFromXml(xml.replace("<?xml version=\"1.0\" encoding=\"UTF-8\"?>", "")), s);
  assert.deepEqual(storeFromXml(""), { quellen: {}, zitate: {} });
});
