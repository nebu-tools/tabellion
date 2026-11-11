// Task pane: search the document's library, cite, keep the lists current.
/* global Office, Word */

import style from "../csl/nlm-citation-sequence-brackets.csl";
import deDE from "../csl/locales-de-DE.xml";
import enUS from "../csl/locales-en-US.xml";
import { createRenderer } from "./render.js";
import { parseLibrary, mergeItems, year } from "./library.js";
import { buildIndex, search } from "./search.js";
import { lookup } from "./lookup.js";
import * as doc from "./word.js";

const renderer = createRenderer({ style, locales: { "de-DE": deDE, "en-US": enUS }, lang: "de-DE" });
const $ = (id) => document.getElementById(id);

const state = { library: {}, index: [], hits: [], active: 0, marked: [], busy: false, sig: null };

function setLibrary(store) {
  state.library = store.quellen;
  state.index = buildIndex(state.library);
  showResults();
}

function names(item) {
  const a = item.author ?? item.editor ?? [];
  const n = a.slice(0, 2).map((x) => x.literal ?? [x.family, (x.given ?? "").replace(/[^A-ZÄÖÜ]/g, "")].filter(Boolean).join(" "));
  return n.join(", ") + (a.length > 2 ? " et al." : "") || "o. A.";
}

function showResults() {
  state.hits = search(state.index, $("q").value, 60);
  state.active = Math.min(state.active, Math.max(0, state.hits.length - 1));
  const ul = $("results");
  ul.replaceChildren();
  if (!state.hits.length) {
    const li = document.createElement("li");
    li.className = "empty";
    li.textContent = Object.keys(state.library).length ? "Keine Treffer." : "Noch keine Quellen im Dokument. Unter „Quellen hinzufügen“ eine Datei importieren oder eine DOI holen.";
    ul.append(li);
    return;
  }
  state.hits.forEach((item, i) => {
    const li = document.createElement("li");
    li.id = "r" + i;
    li.setAttribute("role", "option");
    li.setAttribute("aria-selected", String(i === state.active));
    if (state.marked.includes(item.id)) li.classList.add("on");
    const who = document.createElement("span");
    who.className = "who";
    who.textContent = `${names(item)} (${year(item) || "o. J."})`;
    const key = document.createElement("span");
    key.className = "key";
    key.textContent = item.id;
    const what = document.createElement("span");
    what.className = "what";
    what.textContent = [String(item.title ?? "").replace(/<[^>]+>/g, ""), item["container-title"]].filter(Boolean).join(" · ");
    li.append(who, key, what);
    li.addEventListener("click", (e) => (e.ctrlKey || e.metaKey || e.shiftKey ? toggleMark(item.id) : cite([item.id])));
    ul.append(li);
  });
  $("q").setAttribute("aria-activedescendant", "r" + state.active);
  $("r" + state.active)?.scrollIntoView({ block: "nearest" });
}

function showMarked() {
  const box = $("marked");
  box.hidden = !state.marked.length;
  box.replaceChildren(...state.marked.map((k) => {
    const chip = document.createElement("span");
    chip.className = "chip";
    chip.textContent = k;
    const x = document.createElement("button");
    x.type = "button";
    x.textContent = "×";
    x.setAttribute("aria-label", k + " entfernen");
    x.addEventListener("click", () => toggleMark(k));
    chip.append(x);
    return chip;
  }));
}

function toggleMark(key) {
  state.marked = state.marked.includes(key) ? state.marked.filter((k) => k !== key) : [...state.marked, key];
  showMarked();
  showResults();
  $("q").focus();
}

function status(msg, error = false) {
  $("status").textContent = msg;
  $("status").classList.toggle("error", error);
}

function summary(r) {
  const parts = [`${Object.keys(r.store.quellen).length} Quellen`, `${r.cited} Zitate`];
  if (r.sections > 1) parts.push(`${r.sections} Abschnitte`);
  if (r.pending) parts.push(`${r.pending} gelöscht, offen in der Überarbeitung`);
  let msg = parts.join(" · ");
  if (r.converted) msg = `${r.converted} [@…] umgewandelt. ` + msg;
  const unk = [...(r.unknown ?? []).map((u) => u.keys.join(", ") || u.text), ...(r.unknownMarkers ?? []).map((u) => u.keys.join(", "))];
  if (unk.length) msg += `\nUnbekannt: ${[...new Set(unk)].join("; ")}`;
  return msg;
}

/** Runs document operations one after another and reports their errors. */
let queue = Promise.resolve();
function op(fn) {
  const p = queue.then(() => run(fn));
  queue = p.catch(() => {});
  return p;
}

async function run(fn) {
  state.busy = true;
  document.body.setAttribute("aria-busy", "true");
  try {
    const r = await Word.run((context) => fn(context));
    if (r?.store) setLibrary(r.store);
    if (r?.cited != null) status(summary(r), Boolean(r.unknown?.length || r.unknownMarkers?.length));
    state.sig = await Word.run((context) => doc.signature(context));
    document.dispatchEvent(new CustomEvent("zitate:done", { detail: r }));
    return r;
  } catch (e) {
    status(e.message ?? String(e), true);
    console.error(e);
    return null;
  } finally {
    state.busy = false;
    document.body.removeAttribute("aria-busy");
  }
}

async function cite(keys) {
  const list = keys.length ? keys : state.marked;
  if (!list.length) return;
  const loc = $("loc").value.trim();
  const items = list.map((id) => (loc && list.length === 1 ? { id, locator: loc, label: "page" } : { id }));
  const r = await op((c) => doc.insertCitation(c, renderer, items));
  if (r) {
    state.marked = [];
    $("loc").value = "";
    showMarked();
    showResults();
  }
}

async function importFiles(files) {
  let incoming = [];
  for (const f of files) incoming = incoming.concat(parseLibrary(await f.text(), f.name));
  const r = await op((c) => doc.addSources(c, (lib) => mergeItems(lib, incoming)));
  if (r) status(`${r.added} neu, ${r.updated} aktualisiert · ${Object.keys(r.library).length} Quellen`);
}

async function fetchIdent(text) {
  let item;
  try {
    item = await lookup(text);
  } catch (e) {
    status(e.message, true);
    return;
  }
  const r = await op((c) => doc.addSources(c, (lib) => mergeItems(lib, [item])));
  if (!r) return;
  const same = (a, b) => a && b && String(a).toLowerCase() === String(b).toLowerCase();
  const key = Object.values(r.library).find((x) => same(x.DOI, item.DOI) || same(x.PMID, item.PMID))?.id;
  status(`${r.added ? "Hinzugefügt" : "Schon vorhanden"}: ${key}`);
  $("q").value = key ?? "";
  $("ident").value = "";
  showResults();
  $("q").focus();
}

function wire() {
  $("q").addEventListener("input", () => { state.active = 0; showResults(); });
  $("q").addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      state.active = Math.max(0, Math.min(state.hits.length - 1, state.active + (e.key === "ArrowDown" ? 1 : -1)));
      showResults();
    } else if (e.key === "Enter") {
      e.preventDefault();
      const hit = state.hits[state.active];
      if ((e.ctrlKey || e.metaKey) && hit) toggleMark(hit.id);
      else if (state.marked.length) cite([...state.marked, ...(hit && !state.marked.includes(hit.id) && $("q").value.trim() ? [hit.id] : [])]);
      else if (hit) cite([hit.id]);
    } else if (e.key === "Escape") {
      $("q").value = "";
      showResults();
    }
  });
  $("loc").addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); const h = state.hits[state.active]; if (h) cite([h.id]); } });
  $("bib").addEventListener("click", () => op((c) => doc.insertBibliography(c, renderer)));
  $("refresh").addEventListener("click", () => op((c) => doc.convertMarkers(c, renderer)));
  $("file").addEventListener("change", (e) => { importFiles([...e.target.files]); e.target.value = ""; });
  $("lookup").addEventListener("submit", (e) => { e.preventDefault(); fetchIdent($("ident").value); });
}

// Deleting or pasting a citation changes the order of tags. A quiet check after
// the cursor moves keeps numbers and lists current without a click.
function watch() {
  let timer = null;
  const check = async () => {
    if (state.busy) return;
    try {
      const sig = await Word.run((context) => doc.signature(context));
      if (sig !== state.sig) await op((c) => doc.refresh(c, renderer));
    } catch (e) {
      console.error(e);
    }
  };
  Office.context.document.addHandlerAsync(Office.EventType.DocumentSelectionChanged, () => {
    clearTimeout(timer);
    timer = setTimeout(check, 1200);
  });
}

Office.onReady(async () => {
  wire();
  await op((c) => doc.convertMarkers(c, renderer));
  watch();
  $("q").focus();
  document.dispatchEvent(new CustomEvent("zitate:ready"));
});
