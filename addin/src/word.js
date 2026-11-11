// Reads and writes the document through Office.js. All decisions are in model.js.
/* global Word */

import {
  NS, CITE_PREFIX, BIB_TAG, CITE_NEW, CITE_PLACEHOLDER, BIB_EMPTY,
  emptyStore, storeToXml, storeFromXml, newId, citeTitle, parseMarker, plan,
} from "./model.js";

// Hanging indent of reference list entries, in points (1 cm).
const HANG = 28.35;

const isCite = (cc) => cc.tag?.startsWith(CITE_PREFIX);

export async function loadStore(context) {
  const parts = context.document.customXmlParts.getByNamespace(NS);
  parts.load("items");
  await context.sync();
  if (!parts.items.length) return emptyStore();
  const xml = parts.items[0].getXml();
  await context.sync();
  return storeFromXml(xml.value);
}

export async function saveStore(context, store) {
  const parts = context.document.customXmlParts.getByNamespace(NS);
  parts.load("items");
  await context.sync();
  if (parts.items.length) parts.items[0].setXml(storeToXml(store));
  else context.document.customXmlParts.add(storeToXml(store));
  await context.sync();
}

/** Citations and reference lists in document order, with their texts. */
async function snapshot(context) {
  const all = context.document.body.contentControls;
  all.load("items/tag,items/title,items/id,items/placeholderText");
  await context.sync();
  const ccs = all.items.filter((cc) => isCite(cc) || cc.tag === BIB_TAG);
  const cur = ccs.map((cc) => cc.getReviewedText("Current"));
  const orig = ccs.map((cc) => (isCite(cc) ? cc.getReviewedText("Original") : null));
  await context.sync();
  // Word wraps a control's reviewed text in "<…>". An emptied control shows its placeholder.
  const text = (cc, r) => {
    const v = r.value.startsWith("<") && r.value.endsWith(">") ? r.value.slice(1, -1) : r.value;
    return v === cc.placeholderText ? "" : v;
  };
  const controls = ccs.map((cc, i) => (isCite(cc)
    ? { kind: "cite", tag: cc.tag, title: cc.title, current: text(cc, cur[i]), original: text(cc, orig[i]) }
    : { kind: "bib", current: text(cc, cur[i]) }));
  return { ccs, controls };
}

function setFont(range, run) {
  range.font.italic = !!run.italic;
  range.font.bold = !!run.bold;
  range.font.superscript = !!run.superscript;
  range.font.subscript = !!run.subscript;
}

const plain = (runs) => runs.length === 1 && !runs[0].italic && !runs[0].bold && !runs[0].superscript && !runs[0].subscript;

/** Replaces a citation's text. Plain text keeps the formatting around it. */
function writeCitation(cc, runs) {
  if (plain(runs)) { cc.insertText(runs[0].text, "Replace"); return; }
  runs.forEach((run, i) => setFont(cc.insertText(run.text, i ? "End" : "Replace"), run));
}

function writeBibliography(cc, entries) {
  entries.forEach((runs, i) => {
    let p;
    let rest = runs;
    if (i === 0) {
      // Replacing the whole content leaves one paragraph with the list's style.
      setFont(cc.insertText(runs[0].text, "Replace"), runs[0]);
      p = cc.paragraphs.getFirst();
      rest = runs.slice(1);
    } else {
      p = cc.insertParagraph("", "End");
    }
    for (const run of rest) setFont(p.insertText(run.text, "End"), run);
    p.leftIndent = HANG;
    p.firstLineIndent = -HANG;
  });
}

async function withTracking(context, mode, fn) {
  const doc = context.document;
  doc.load("changeTrackingMode");
  await context.sync();
  const was = doc.changeTrackingMode;
  if (was === mode) return fn();
  doc.changeTrackingMode = mode;
  await context.sync();
  try {
    return await fn();
  } finally {
    doc.changeTrackingMode = was;
    await context.sync();
  }
}

/**
 * Brings numbers and reference lists up to date. Citations in `fresh` (Word
 * content control ids) were just added by the user and follow the document's
 * track-changes mode. Everything else is derived and is written untracked, so
 * renumbering does not bury the real changes in review.
 */
export async function refresh(context, renderer, { fresh = [], store } = {}) {
  store = store ?? await loadStore(context);
  const { ccs, controls } = await snapshot(context);
  const p = plan(controls, store, renderer);
  if (p.store) await saveStore(context, p.store);
  const own = p.citations.filter((c) => fresh.includes(ccs[c.index].id));
  for (const c of own) writeCitation(ccs[c.index], c.runs);
  await context.sync();
  const rest = p.citations.filter((c) => !own.includes(c));
  if (rest.length || p.bibs.length || p.removes.length) {
    await withTracking(context, "Off", async () => {
      for (const i of p.removes) ccs[i].delete(false);
      for (const c of rest) writeCitation(ccs[c.index], c.runs);
      for (const b of p.bibs) writeBibliography(ccs[b.index], b.entries);
      await context.sync();
    });
  }
  return { ...p, store: p.store ?? store };
}

function tagCitation(cc, id, items) {
  cc.tag = CITE_PREFIX + id;
  cc.title = citeTitle(items);
  cc.placeholderText = CITE_PLACEHOLDER;
  cc.appearance = "BoundingBox";
}

/**
 * Cites `items` at the cursor. Inside an existing citation the items join it,
 * so a click on [3] and a pick gives [3,4]. With track changes on they become a
 * citation of their own right after it, which review can reject on its own.
 */
export async function insertCitation(context, renderer, items) {
  const store = await loadStore(context);
  const sel = context.document.getSelection();
  const host = sel.parentContentControlOrNullObject;
  host.load("tag,title,id");
  context.document.load("changeTrackingMode");
  await context.sync();
  let cc;
  let all = items;
  const inCite = !host.isNullObject && isCite(host);
  if (inCite && context.document.changeTrackingMode === "Off") {
    const before = store.zitate[host.tag.slice(CITE_PREFIX.length)] ?? [];
    all = [...before, ...items.filter((x) => !before.some((b) => b.id === x.id))];
    cc = host;
  } else if (inCite) {
    cc = host.getRange("After").insertText(CITE_NEW, "Replace").insertContentControl();
  } else if (!host.isNullObject && host.tag === BIB_TAG) {
    throw new Error("Der Cursor steht im Literaturverzeichnis.");
  } else {
    cc = sel.insertText(CITE_NEW, "End").insertContentControl();
  }
  // A new id each time, so copies of a citation elsewhere keep their items.
  const id = newId();
  tagCitation(cc, id, all);
  cc.load("id");
  await context.sync();
  const next = { ...store, zitate: { ...store.zitate, [id]: all } };
  await saveStore(context, next);
  cc.getRange("After").select("End");
  await context.sync();
  return refresh(context, renderer, { fresh: [cc.id], store: next });
}

/**
 * Adds a reference list for the citations since the previous one. An empty or
 * fully selected paragraph, such as the template's answer placeholder, becomes
 * the list; otherwise it goes below the cursor's paragraph. Like its entries,
 * the list is written untracked.
 */
export async function insertBibliography(context, renderer) {
  await withTracking(context, "Off", () => addList(context));
  return refresh(context, renderer);
}

async function addList(context) {
  const sel = context.document.getSelection();
  const host = sel.parentContentControlOrNullObject;
  host.load("tag");
  sel.load("text");
  const para = sel.paragraphs.getFirst();
  para.load("text,styleBuiltIn");
  await context.sync();
  if (!host.isNullObject && (isCite(host) || host.tag === BIB_TAG)) throw new Error("Der Cursor steht in einem Zitat oder Literaturverzeichnis.");
  const whole = !para.text.trim() || sel.text.trim() === para.text.trim();
  const target = whole ? para : para.insertParagraph("", "After");
  if (!whole && /^Heading/.test(para.styleBuiltIn)) target.styleBuiltIn = Word.Style.normal;
  target.load("isListItem");
  await context.sync();
  // The entries bring their own numbers.
  if (target.isListItem) target.detachFromList();
  target.insertText(BIB_EMPTY, "Replace");
  const cc = target.insertContentControl();
  cc.tag = BIB_TAG;
  cc.title = "Literaturverzeichnis";
  cc.appearance = "BoundingBox";
  await context.sync();
}

/**
 * Turns typed `[@key]` markers into citations. Markers with unknown keys stay
 * as text and come back in `unknown`.
 */
export async function convertMarkers(context, renderer) {
  const store = await loadStore(context);
  const found = context.document.body.search("\\[@*\\]", { matchWildcards: true });
  found.load("items/text");
  await context.sync();
  const zitate = { ...store.zitate };
  const made = [];
  const unknown = [];
  for (const r of found.items) {
    const items = parseMarker(r.text);
    if (!items) continue;
    const missing = items.filter((x) => !store.quellen[x.id]).map((x) => x.id);
    if (missing.length) { unknown.push({ text: r.text, keys: missing }); continue; }
    const id = newId();
    const cc = r.insertText(CITE_NEW, "Replace").insertContentControl();
    tagCitation(cc, id, items);
    cc.load("id");
    zitate[id] = items;
    made.push(cc);
  }
  await context.sync();
  const next = { ...store, zitate };
  if (made.length) await saveStore(context, next);
  const res = await refresh(context, renderer, { fresh: made.map((cc) => cc.id), store: next });
  return { ...res, converted: made.length, unknownMarkers: unknown };
}

/** Adds works to the document's library. */
export async function addSources(context, merge) {
  const store = await loadStore(context);
  const r = merge(store.quellen);
  const next = { ...store, quellen: r.library };
  await saveStore(context, next);
  return { ...r, store: next };
}

/** A cheap fingerprint of the citations' order, to notice deletions and pastes. */
export async function signature(context) {
  const all = context.document.body.contentControls;
  all.load("items/tag");
  await context.sync();
  return all.items.filter((cc) => isCite(cc) || cc.tag === BIB_TAG).map((cc) => cc.tag).join("|");
}
