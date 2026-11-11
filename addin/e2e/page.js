// Runs inside Word's task pane next to the real pane (dist-e2e/e2e.html). Drives
// the pane like a user would and reports each check to the e2e server.
/* global Word */

const results = [];
let last = null;
let first = null;
document.addEventListener("zitate:done", (e) => { last = e.detail; first = first ?? e.detail; });
const post = (o) => fetch("/log", { method: "POST", body: JSON.stringify(o) }).catch(() => {});
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const $ = (id) => document.getElementById(id);

async function idle() {
  await sleep(150);
  for (let calm = 0; calm < 3; ) {
    await sleep(100);
    calm = document.body.hasAttribute("aria-busy") ? 0 : calm + 1;
  }
}

function check(name, ok, detail) {
  results.push({ name, ok: !!ok, detail });
  post({ check: name, ok: !!ok, detail });
}

const word = (fn) => Word.run(fn);

/** Citations and lists as [tag, text] in document order. */
async function controls() {
  return word(async (c) => {
    const all = c.document.body.contentControls;
    all.load("items/tag,items/text");
    await c.sync();
    return all.items.filter((x) => x.tag.startsWith("zitat:") || x.tag === "literaturverzeichnis").map((x) => [x.tag === "literaturverzeichnis" ? "bib" : "cite", x.text]);
  });
}
const citeTexts = async () => (await controls()).filter((x) => x[0] === "cite").map((x) => x[1]);
const bibTexts = async () => (await controls()).filter((x) => x[0] === "bib").map((x) => x[1].split(/[\r\n]+/).filter(Boolean));

/** What Word reports for the i-th citation. */
async function probe(i) {
  return word(async (c) => {
    const cc = c.document.body.contentControls;
    cc.load("items/tag");
    await c.sync();
    const x = cc.items.filter((y) => y.tag.startsWith("zitat:"))[i];
    x.load("text,placeholderText,tag");
    const cur = x.getReviewedText("Current");
    const orig = x.getReviewedText("Original");
    const ox = x.getOoxml();
    await c.sync();
    const body = /<w:body>([\s\S]*)<\/w:body>/.exec(ox.value)?.[1] ?? "";
    return { tag: x.tag, text: x.text, ph: x.placeholderText, cur: cur.value, orig: orig.value, xml: body.replace(/<w:rPr>[\s\S]*?<\/w:rPr>/g, "").slice(0, 1500) };
  });
}

async function cursorAfter(text) {
  await word(async (c) => {
    const r = c.document.body.search(text, { matchCase: true });
    r.load("items");
    await c.sync();
    if (!r.items.length) throw new Error("nicht gefunden: " + text);
    r.items[0].getRange("End").select("End");
    await c.sync();
  });
}

function key(target, k, opts = {}) {
  target.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...opts }));
}

async function pick(query, opts) {
  $("q").value = query;
  $("q").dispatchEvent(new Event("input"));
  key($("q"), "Enter", opts);
  await idle();
}

const BIB = String.raw`
@article{mueller2020, author={M{\"u}ller, Anna and Smith, John}, title={Overall survival with drug X in colorectal cancer},
  journal={N Engl J Med}, year={2020}, volume={382}, pages={301--310}, doi={10.1056/NEJMoa2001}}
@techreport{csr, author={{Muster Pharma GmbH}}, title={Clinical study report XY-123}, year={2023}, institution={Muster Pharma}}
@book{lee2010, author={Lee, Kim}, title={Statistics for trials}, publisher={Springer}, address={Berlin}, year={2010}}
@online{fi2024, author={{Muster Pharma GmbH}}, title={Fachinformation Demozil}, url={https://www.fachinfo.de/x}, year={2024}, urldate={2026-11-02}}
`;

async function setup() {
  await word(async (c) => {
    const b = c.document.body;
    b.clear();
    c.document.changeTrackingMode = "Off";
    const lines = [
      "Abschnitt 1", "Erste Aussage.", "Zweite Aussage [@csr] mit Marker.", "Dritte Aussage.", "Referenzliste 1",
      "Abschnitt 2", "Vierte Aussage.", "Fünfte Aussage.", "Referenzliste 2", "Anhang mit Aussage.",
    ];
    b.insertParagraph(lines[0], "Start");
    for (const l of lines.slice(1)) b.insertParagraph(l, "End");
    const t = b.insertTable(2, 2, "End", [["Studie", "Quelle"], ["XY-123", "Tabellenzelle."]]);
    t.styleBuiltIn = Word.Style.gridTable1Light;
    b.insertParagraph("Ende.", "End");
    await c.sync();
  });
}

async function scenario() {
  await setup();

  // Import a .bib through the file input.
  const dt = new DataTransfer();
  dt.items.add(new File([BIB], "bibliothek.bib", { type: "text/plain" }));
  $("file").files = dt.files;
  $("file").dispatchEvent(new Event("change"));
  await idle();
  check("import", /4 neu/.test($("status").textContent), $("status").textContent);
  $("q").value = "muel";
  $("q").dispatchEvent(new Event("input"));
  check("fuzzy search", document.querySelector("#results li .key")?.textContent === "mueller2020", document.querySelector("#results li .key")?.textContent);

  // Cite by search and Enter.
  await cursorAfter("Erste Aussage");
  await pick("muel smith");
  check("cite by search", JSON.stringify(await citeTexts()) === JSON.stringify(["[1]"]), await citeTexts());

  // Typed marker, converted by Aktualisieren.
  $("refresh").click();
  await idle();
  check("marker converted", JSON.stringify(await citeTexts()) === JSON.stringify(["[1]", "[2]"]), [await citeTexts(), $("status").textContent]);

  // A refresh of a current document writes nothing.
  $("refresh").click();
  await idle();
  check("idle refresh writes nothing", last && last.citations.length === 0 && last.bibs.length === 0 && last.removes.length === 0, last && { c: last.citations.length, b: last.bibs.length, p: await probe(0) });

  // Reference list after "Referenzliste 1".
  await cursorAfter("Referenzliste 1");
  $("bib").click();
  await idle();
  let bibs = await bibTexts();
  check("first list", bibs.length === 1 && bibs[0].length === 2 && /^1\.\s*Müller A, Smith J\. Overall survival/.test(bibs[0][0]) && /^2\.\s*Muster Pharma GmbH/.test(bibs[0][1]), bibs);

  // Mark two with Ctrl+Enter, cite both at once.
  await cursorAfter("Dritte Aussage");
  $("q").value = "lee"; $("q").dispatchEvent(new Event("input"));
  key($("q"), "Enter", { ctrlKey: true });
  $("q").value = "fachinfo"; $("q").dispatchEvent(new Event("input"));
  key($("q"), "Enter", { ctrlKey: true });
  key($("q"), "Enter");
  await idle();
  check("multi cite", JSON.stringify(await citeTexts()) === JSON.stringify(["[1]", "[2]", "[3,4]"]), await citeTexts());

  // Section 2 starts again at 1, and its list is separate.
  await cursorAfter("Vierte Aussage");
  await pick("lee");
  await cursorAfter("Referenzliste 2");
  $("bib").click();
  await idle();
  bibs = await bibTexts();
  check("second section restarts", JSON.stringify(await citeTexts()) === JSON.stringify(["[1]", "[2]", "[3,4]", "[1]"]) && bibs.length === 2 && bibs[0].length === 4 && bibs[1].length === 1 && /^1\.\s*Lee K/.test(bibs[1][0]), [await citeTexts(), bibs]);

  // With two lists in place, another refresh still writes nothing.
  $("refresh").click();
  await idle();
  check("idle refresh with lists", last && last.citations.length === 0 && last.bibs.length === 0, last && { c: last.citations, b: last.bibs.length });

  // Appendix after the last list and a table cell join the last list.
  await cursorAfter("Anhang mit Aussage");
  await pick("csr");
  await cursorAfter("Tabellenzelle");
  await pick("mueller2020");
  bibs = await bibTexts();
  check("appendix and table", JSON.stringify(await citeTexts()) === JSON.stringify(["[1]", "[2]", "[3,4]", "[1]", "[2]", "[3]"]) && bibs[1].length === 3, [await citeTexts(), bibs]);

  // Inside an existing citation, a pick joins it.
  await word(async (c) => {
    const cc = c.document.body.contentControls;
    cc.load("items/tag");
    await c.sync();
    cc.items.filter((x) => x.tag.startsWith("zitat:"))[3].getRange("Content").select("End");
    await c.sync();
  });
  await pick("fachinfo");
  check("join citation", (await citeTexts())[3] === "[1,2]", await citeTexts());

  // A new citation early on renumbers later ones and the list.
  await cursorAfter("Zweite Aussage");
  await pick("lee");
  bibs = await bibTexts();
  check("renumber", JSON.stringify((await citeTexts()).slice(0, 4)) === JSON.stringify(["[1]", "[2]", "[3]", "[2,4]"]) && /^2\.\s*Lee K/.test(bibs[0][1]), [await citeTexts(), bibs[0]]);

  // Deleting a citation: the list follows on refresh.
  await word(async (c) => {
    const cc = c.document.body.contentControls;
    cc.load("items/tag");
    await c.sync();
    cc.items.filter((x) => x.tag.startsWith("zitat:"))[1].delete(false);
    await c.sync();
  });
  $("refresh").click();
  await idle();
  bibs = await bibTexts();
  check("delete", JSON.stringify((await citeTexts()).slice(0, 3)) === JSON.stringify(["[1]", "[2]", "[3,4]"]) && bibs[0].length === 4 && /^3\.\s*Lee K/.test(bibs[0][2]), [await citeTexts(), bibs[0]]);

  // Emptying a citation's text by hand removes the leftover control.
  await word(async (c) => {
    const cc = c.document.body.contentControls;
    cc.load("items/tag");
    await c.sync();
    cc.items.filter((x) => x.tag.startsWith("zitat:"))[1].getRange("Content").delete();
    await c.sync();
  });
  const emptied = await probe(1);
  $("refresh").click();
  await idle();
  const after = await citeTexts();
  check("emptied", after.length === 5 && after[1] === "[2,3]", { after, emptied });

  // A pasted copy of a citation: same number, both survive.
  await word(async (c) => {
    const cc = c.document.body.contentControls;
    cc.load("items/tag");
    await c.sync();
    const ox = cc.items.filter((x) => x.tag.startsWith("zitat:"))[0].getRange("Whole").getOoxml();
    await c.sync();
    const r = c.document.body.search("Ende", { matchCase: true });
    r.load("items");
    await c.sync();
    r.items[0].getRange("End").insertOoxml(ox.value, "Replace");
    await c.sync();
  });
  $("refresh").click();
  await idle();
  const pasted = await citeTexts();
  // The copy sits behind the last list, where this work is number 3.
  check("paste copy", pasted.length === 6 && pasted[5] === pasted[4], pasted);

  // Track changes on: a new citation is a tracked insertion, the renumbering
  // and the list are not, and the mode stays on.
  await word(async (c) => { c.document.changeTrackingMode = "TrackAll"; await c.sync(); });
  await cursorAfter("Erste Aussage");
  await pick("fachinfo");
  const tracked = await word(async (c) => {
    c.document.load("changeTrackingMode");
    const tcs = c.document.body.getTrackedChanges();
    tcs.load("items/type,items/text");
    await c.sync();
    return { mode: c.document.changeTrackingMode, changes: tcs.items.map((t) => [t.type, t.text]) };
  });
  const onlyCitation = tracked.changes.length > 0 && tracked.changes.every(([type, text]) => type === "Added" && /^\[[\d,–-]+\]$/.test(text.trim()));
  check("tracked insert only", tracked.mode === "TrackAll" && onlyCitation, tracked);
  const nums = await citeTexts();
  // The cursor sits before the first citation, so the new one comes first.
  check("tracked renumber", JSON.stringify(nums.slice(0, 3)) === JSON.stringify(["[1]", "[2]", "[1,3]"]) && nums.length === 7, nums);

  // With tracking on, a pick inside a citation becomes its own citation after it.
  await word(async (c) => {
    const cc = c.document.body.contentControls;
    cc.load("items/tag");
    await c.sync();
    cc.items.filter((x) => x.tag.startsWith("zitat:"))[1].getRange("Content").select("End");
    await c.sync();
  });
  await pick("csr");
  const joined = await citeTexts();
  check("tracked join is separate", joined.length === 8 && joined[1] === "[2]" && joined[2] === "[3]", joined);
  const changes = await word(async (c) => {
    const tcs = c.document.body.getTrackedChanges();
    tcs.load("items/type,items/text");
    await c.sync();
    return tcs.items.map((t) => [t.type, t.text]);
  });
  check("tracked join only adds", changes.every(([t]) => t === "Added"), changes);

  // A tracked deletion of an older citation stays for review, without a number.
  // (Deleting one's own tracked insertion just removes it.)
  await word(async (c) => {
    const cc = c.document.body.contentControls;
    cc.load("items/tag");
    await c.sync();
    cc.items.filter((x) => x.tag.startsWith("zitat:"))[1].getRange("Content").delete();
    await c.sync();
  });
  const pend = await probe(1);
  $("refresh").click();
  await idle();
  const left = await citeTexts();
  check("tracked delete pending", /1 gelöscht, offen/.test($("status").textContent) && left.length === 8 && left[0] === "[1]" && left[2] === "[2]" && left[3] === "[1,3]", { status: $("status").textContent, left, pend });
  await word(async (c) => { c.document.changeTrackingMode = "Off"; await c.sync(); });

  // DOI lookup over the network.
  $("ident").value = "10.1056/NEJMoa1406498";
  $("lookup").dispatchEvent(new Event("submit", { cancelable: true }));
  await idle();
  check("doi lookup", /Hinzugefügt/.test($("status").textContent), $("status").textContent);
  $("ident").value = "PMID 25265492";
  $("lookup").dispatchEvent(new Event("submit", { cancelable: true }));
  await idle();
  check("pmid lookup", /Hinzugefügt: long2014/.test($("status").textContent), $("status").textContent);
}

const ANSWER = "<< Angaben des pharmazeutischen Unternehmers >>";

/** The first answer placeholder below a heading of the template. */
async function selectAnswerAfter(heading) {
  await word(async (c) => {
    const hs = c.document.body.search(heading, { matchCase: true });
    const as = c.document.body.search(ANSWER, { matchCase: true });
    hs.load("items");
    as.load("items");
    await c.sync();
    // The last hit is the heading itself; earlier ones sit in the table of contents.
    const h = hs.items[hs.items.length - 1];
    const where = as.items.map((a) => a.compareLocationWith(h));
    await c.sync();
    const i = where.findIndex((w) => w.value === "After" || w.value === "AdjacentAfter");
    as.items[i].select();
    await c.sync();
  });
}

/** Types an answer into the placeholder below a heading and puts the cursor at its end. */
async function answer(heading, text) {
  await selectAnswerAfter(heading);
  await word(async (c) => {
    c.document.getSelection().insertText(text, "Replace").select("End");
    await c.sync();
  });
}

// On a G-BA Modul 3 template: citations in two sections, each list in its
// "Referenzliste für Abschnitt" answer slot.
async function templateScenario() {
  const dt = new DataTransfer();
  dt.items.add(new File([BIB], "bibliothek.bib", { type: "text/plain" }));
  $("file").files = dt.files;
  $("file").dispatchEvent(new Event("change"));
  await idle();
  check("template import", /4 neu/.test($("status").textContent), $("status").textContent);

  await answer("Benennung der zweckmäßigen Vergleichstherapie", "Die zweckmäßige Vergleichstherapie ist Docetaxel.");
  await pick("mueller");
  await answer("Begründung für die Wahl der zweckmäßigen Vergleichstherapie", "Ein Beratungsgespräch fand statt.");
  await pick("csr");
  await selectAnswerAfter("Referenzliste für Abschnitt 3.1");
  $("bib").click();
  await idle();

  await answer("Beschreibung der Erkrankung und Charakterisierung der Zielpopulation", "Die Erkrankung ist selten.");
  $("q").value = "lee"; $("q").dispatchEvent(new Event("input"));
  key($("q"), "Enter", { ctrlKey: true });
  await pick("mueller");
  // A list inserted with tracking on is derived content and stays untracked.
  await word(async (c) => { c.document.changeTrackingMode = "TrackAll"; await c.sync(); });
  await selectAnswerAfter("Referenzliste für Abschnitt 3.2");
  $("bib").click();
  await idle();
  const tracked = await word(async (c) => {
    c.document.load("changeTrackingMode");
    const tcs = c.document.body.getTrackedChanges();
    tcs.load("items/type,items/text");
    await c.sync();
    const r = { mode: c.document.changeTrackingMode, changes: tcs.items.map((t) => [t.type, t.text]) };
    c.document.changeTrackingMode = "Off";
    await c.sync();
    return r;
  });
  check("template list untracked", tracked.mode === "TrackAll" && tracked.changes.length === 0, tracked);

  const cites = await citeTexts();
  const bibs = await bibTexts();
  check("template sections", JSON.stringify(cites) === JSON.stringify(["[1]", "[2]", "[1,2]"]) && bibs.length === 2 && bibs[0].length === 2 && bibs[1].length === 2 && /^1\.\s*Lee K/.test(bibs[1][0]), { cites, bibs });
  const styles = await word(async (c) => {
    const all = c.document.body.contentControls;
    all.load("items/tag");
    await c.sync();
    const ps = all.items.filter((x) => x.tag === "literaturverzeichnis").map((x) => { const p = x.paragraphs; p.load("items/style,items/styleBuiltIn,items/isListItem"); return p; });
    await c.sync();
    return ps.flatMap((p) => p.items.map((q) => [q.style, q.styleBuiltIn, q.isListItem]));
  });
  check("template list style", styles.length === 4 && styles.every(([, b, li]) => !/^Heading/.test(b) && !li), styles);
}

// A document saved earlier, maybe changed by the R pipeline in between: the
// pane's first refresh must find everything current.
async function verifyScenario() {
  check("reopened: first refresh writes nothing", first && first.citations.length === 0 && first.bibs.length === 0 && first.removes.length === 0, first && { c: first.citations, b: first.bibs.length, r: first.removes });
  check("reopened: library and citations", first && first.cited === 3 && Object.keys(first.store.quellen).length === 4, first && { cited: first.cited, unknown: first.unknown });
  // Before the sentence's period, so ahead of the citation behind it.
  await cursorAfter("Die Erkrankung ist selten");
  await pick("fachinfo");
  const bibs = await bibTexts();
  check("reopened: cite again", JSON.stringify(await citeTexts()) === JSON.stringify(["[1]", "[2]", "[1]", "[2,3]"]) && bibs[1].length === 3 && /^1\.\s*Muster Pharma GmbH\. Fachinformation/.test(bibs[1][0]), { cites: await citeTexts(), bibs });
}

async function mode() {
  return word(async (c) => {
    const all = c.document.body.contentControls;
    all.load("items/tag");
    const t = c.document.body.search("Referenzliste für Abschnitt 3.1", { matchCase: true });
    t.load("items");
    await c.sync();
    if (all.items.some((x) => x.tag.startsWith("zitat:"))) return "verify";
    return t.items.length ? "template" : "main";
  });
}

document.addEventListener("zitate:ready", async () => {
  const m = await mode();
  post({ stage: "ready", mode: m });
  try {
    await { main: scenario, template: templateScenario, verify: verifyScenario }[m]();
  } catch (e) {
    check("scenario", false, String(e && (e.stack || e.message || e)));
  }
  const final = await controls().catch((e) => String(e));
  post({ done: true, ok: results.every((r) => r.ok), failed: results.filter((r) => !r.ok).map((r) => r.name), controls: final });
});
window.addEventListener("error", (e) => post({ error: String(e.message) }));
window.addEventListener("unhandledrejection", (e) => post({ error: String(e.reason && (e.reason.stack || e.reason)) }));
