// What the document should look like. Works on a plain snapshot of the
// document, so everything here runs without Word.
//
// In the docx a citation is an inline content control tagged `zitat:<id>`, and
// a reference list is a block content control tagged `literaturverzeichnis`.
// The cited works and each citation's items live in one custom XML part:
//
//   <zitate xmlns="urn:newga:zitate:1">{"quellen": {key: CSL-JSON}, "zitate": {id: [{id, locator?}]}}</zitate>

import { runsText } from "./render.js";

export const NS = "urn:newga:zitate:1";
export const CITE_PREFIX = "zitat:";
export const BIB_TAG = "literaturverzeichnis";
// Text of a new citation until it is numbered, and what Word shows in an emptied one.
export const CITE_NEW = "[?]";
export const CITE_PLACEHOLDER = "[leeres Zitat]";
export const BIB_EMPTY = "Keine Quellen zitiert.";
const TITLE_PREFIX = "Zitat: ";

export function emptyStore() {
  return { quellen: {}, zitate: {} };
}

const escapeXml = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

export function storeToXml(store) {
  return `<?xml version="1.0" encoding="UTF-8"?><zitate xmlns="${NS}">${escapeXml(JSON.stringify({ quellen: store.quellen, zitate: store.zitate }))}</zitate>`;
}

export function storeFromXml(xml) {
  const m = /<zitate\b[^>]*>([\s\S]*)<\/zitate>/.exec(xml ?? "");
  if (!m || !m[1].trim()) return emptyStore();
  const json = m[1].replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&quot;/g, "\"").replace(/&apos;/g, "'").replace(/&amp;/g, "&");
  const d = JSON.parse(json);
  return { quellen: d.quellen ?? {}, zitate: d.zitate ?? {} };
}

export function newId() {
  const a = new Uint8Array(6);
  globalThis.crypto.getRandomValues(a);
  return Array.from(a, (b) => (b % 36).toString(36)).join("") + Date.now().toString(36).slice(-4);
}

export function citeTitle(items) {
  const t = TITLE_PREFIX + items.map((x) => x.id).join("; ");
  return t.length <= 64 ? t : t.slice(0, 63) + "…";
}

/** Keys from a title, unless it was cut short. */
export function keysFromTitle(title) {
  if (!title?.startsWith(TITLE_PREFIX) || title.endsWith("…")) return null;
  const keys = title.slice(TITLE_PREFIX.length).split(";").map((k) => k.trim()).filter(Boolean);
  return keys.length ? keys : null;
}

/**
 * A typed citation: `[@key]`, `[@a; @b]` or `[@key, S. 12]`. Returns the items
 * or null when the text is not a citation.
 */
export function parseMarker(text) {
  const m = /^\[\s*(@[\s\S]*)\]$/.exec(text.trim());
  if (!m) return null;
  const items = [];
  for (const part of m[1].split(";")) {
    const p = /^\s*@([A-Za-z0-9_:.\-]+)\s*(?:,\s*(.+?))?\s*$/.exec(part);
    if (!p) return null;
    const loc = p[2]?.replace(/^(S\.|pp?\.|Seite|page)\s*/i, "").trim();
    items.push(loc ? { id: p[1], locator: loc, label: "page" } : { id: p[1] });
  }
  return items;
}

const norm = (s) => String(s ?? "").replace(/ /g, " ").replace(/[ \t]+/g, " ").trim();
const lines = (s) => String(s ?? "").split(/[\r\n\v]+/).map(norm).filter(Boolean);
const isEmpty = (s) => norm(s) === "";

/**
 * Plans an update. `controls` are the citations and reference lists in document
 * order: {kind: "cite", tag, title, current, original} or {kind: "bib", current}.
 * `current` and `original` are the texts with tracked changes accepted and
 * rejected, "" while Word shows the placeholder. Returns what to write, by
 * position in `controls`.
 */
export function plan(controls, store, renderer) {
  const sections = [{ citations: [], bib: null }];
  const out = { citations: [], bibs: [], removes: [], unknown: [], pending: 0, store: null };
  let zitate = store.zitate;
  controls.forEach((c, index) => {
    if (c.kind === "bib") {
      sections[sections.length - 1].bib = index;
      sections.push({ citations: [], bib: null });
      return;
    }
    // Emptied by hand: the control is a leftover. A tracked deletion waits for review.
    if (isEmpty(c.current)) {
      if (isEmpty(c.original)) out.removes.push(index);
      else out.pending++;
      return;
    }
    const id = c.tag.slice(CITE_PREFIX.length);
    let items = zitate[id];
    if (!items) {
      // Pasted from another document: the title still names the keys.
      const keys = keysFromTitle(c.title);
      if (keys && keys.every((k) => store.quellen[k])) {
        items = keys.map((k) => ({ id: k }));
        zitate = { ...zitate, [id]: items };
      }
    }
    const missing = items ? items.filter((x) => !store.quellen[x.id]).map((x) => x.id) : keysFromTitle(c.title) ?? [];
    if (!items || missing.length) {
      out.unknown.push({ index, keys: missing, text: norm(c.current) });
      return;
    }
    sections[sections.length - 1].citations.push({ index, items, current: c.current });
  });
  if (zitate !== store.zitate) out.store = { ...store, zitate };

  // Citations behind the last reference list belong to it, e.g. in appendices.
  const last = sections.pop();
  if (sections.length) sections[sections.length - 1].citations.push(...last.citations);
  else sections.push(last);

  const rendered = renderer.render(sections, store.quellen);
  sections.forEach((sec, s) => {
    sec.citations.forEach((c, i) => {
      const runs = rendered[s].citations[i];
      if (norm(runsText(runs)) !== norm(c.current)) out.citations.push({ index: c.index, runs });
    });
    if (sec.bib == null) return;
    const entries = rendered[s].entries.length ? rendered[s].entries : [[{ text: BIB_EMPTY }]];
    const want = entries.map((e) => norm(runsText(e)));
    const have = lines(controls[sec.bib].current);
    if (want.join("\n") !== have.join("\n")) out.bibs.push({ index: sec.bib, entries });
  });
  out.sections = sections.length;
  out.cited = sections.reduce((n, s) => n + s.citations.length, 0);
  return out;
}
