// Reference libraries: BibTeX, RIS and CSL-JSON in, CSL-JSON items out.
// Every item carries its citation key as `id`.

/** Parses a library file. The format comes from the file name or the content. */
export function parseLibrary(text, name = "") {
  const t = text.replace(/^﻿/, "");
  const ext = name.toLowerCase().split(".").pop();
  if (ext === "json" || /^\s*[[{]/.test(t)) return fromCslJson(JSON.parse(t));
  if (ext === "ris" || /^TY {2}- /m.test(t)) return parseRis(t);
  return parseBibtex(t);
}

function fromCslJson(data) {
  const list = Array.isArray(data) ? data : data.items ?? [data];
  return list.filter((x) => x && typeof x === "object").map((x) => ({ ...x, id: x.id == null ? undefined : String(x.id) }));
}

// ---- BibTeX -----------------------------------------------------------------

const BIB_TYPES = {
  article: "article-journal", book: "book", booklet: "pamphlet", inbook: "chapter", incollection: "chapter",
  inproceedings: "paper-conference", conference: "paper-conference", manual: "report", mastersthesis: "thesis",
  phdthesis: "thesis", thesis: "thesis", techreport: "report", report: "report", online: "webpage",
  electronic: "webpage", www: "webpage", misc: "document", unpublished: "manuscript", proceedings: "book",
  dataset: "dataset", software: "software", patent: "patent", legislation: "legislation", standard: "standard",
};

const ACCENTS = { "\"": "̈", "'": "́", "`": "̀", "^": "̂", "~": "̃", "=": "̄", ".": "̇", u: "̆", v: "̌", H: "̋", c: "̧", k: "̨", r: "̊" };
const SYMBOLS = { ss: "ß", ae: "æ", AE: "Æ", oe: "œ", OE: "Œ", aa: "å", AA: "Å", o: "ø", O: "Ø", l: "ł", L: "Ł", i: "ı", j: "ȷ", textendash: "–", textemdash: "—", textquoteright: "’", textquoteleft: "‘", textregistered: "®", texttrademark: "™", textcopyright: "©", textbackslash: "\\", textdollar: "$", textunderscore: "_", textasciitilde: "~", ldots: "…", dots: "…", textellipsis: "…", textpm: "±", pm: "±", geq: "≥", leq: "≤", ge: "≥", le: "≤", alpha: "α", beta: "β", gamma: "γ", delta: "δ", kappa: "κ", mu: "μ", times: "×" };
const MARKUP = { textit: "i", emph: "i", textbf: "b", textsuperscript: "sup", textsubscript: "sub", textsc: "sc" };

/** LaTeX text to plain text with the small HTML subset CSL understands. */
export function latexToText(s) {
  let out = "";
  let i = 0;
  const group = () => {
    // Reads one argument: {…} or a single character.
    while (s[i] === " ") i++;
    if (s[i] === "{") {
      let depth = 0;
      const start = i;
      for (; i < s.length; i++) {
        if (s[i] === "\\") { i++; continue; }
        if (s[i] === "{") depth++;
        if (s[i] === "}" && --depth === 0) { i++; break; }
      }
      return latexToText(s.slice(start + 1, i - 1));
    }
    return s[i++] ?? "";
  };
  while (i < s.length) {
    const c = s[i];
    if (c === "{" || c === "}") { i++; continue; }
    if (c === "~") { out += " "; i++; continue; }
    if (c === "-" && s.startsWith("---", i)) { out += "—"; i += 3; continue; }
    if (c === "-" && s.startsWith("--", i)) { out += "–"; i += 2; continue; }
    if (c === "$") { i++; continue; }
    if (c !== "\\") { out += c; i++; continue; }
    i++;
    const m = /^[A-Za-z]+/.exec(s.slice(i));
    if (!m) {
      const sym = s[i++];
      if (sym in ACCENTS) { out += (group() + ACCENTS[sym]).normalize("NFC"); continue; }
      out += sym === "\\" ? " " : sym;
      continue;
    }
    const cmd = m[0];
    i += cmd.length;
    if (cmd.length === 1 && cmd in ACCENTS) { out += (group() + ACCENTS[cmd]).normalize("NFC"); continue; }
    if (cmd in SYMBOLS) { if (s[i] === " ") i++; if (s.startsWith("{}", i)) i += 2; out += SYMBOLS[cmd]; continue; }
    if (cmd in MARKUP) {
      const tag = MARKUP[cmd];
      const inner = group();
      out += tag === "sc" ? `<span style="font-variant:small-caps;">${inner}</span>` : `<${tag}>${inner}</${tag}>`;
      continue;
    }
    if (s[i] === " ") i++;
    // Unknown command: keep its argument, drop the command.
    if (s[i] === "{") out += group();
  }
  return out.replace(/[ \t\r\n]+/g, " ").trim();
}

/** Splits a BibTeX name list into CSL names. */
export function bibNames(s) {
  return splitTop(s, /\s+and\s+/i).filter((n) => n && n.toLowerCase() !== "others").map((n) => {
    const raw = n.trim();
    if (/^\{.*\}$/.test(raw) && splitTop(raw.slice(1, -1), /,/).length === 1) return { literal: latexToText(raw) };
    const parts = splitTop(raw, /\s*,\s*/);
    if (parts.length >= 2) {
      const r = { family: latexToText(parts[0]), given: latexToText(parts[parts.length - 1]) };
      if (parts.length === 3) r.suffix = latexToText(parts[1]);
      return r;
    }
    const words = splitTop(raw, /\s+/);
    if (words.length === 1) return { family: latexToText(words[0]) };
    // "von" particles start lower case and belong to the family name.
    let k = words.length - 1;
    while (k > 1 && /^[a-z]/.test(words[k - 1])) k--;
    return { family: latexToText(words.slice(k).join(" ")), given: latexToText(words.slice(0, k).join(" ")) };
  });
}

/** Splits at a separator outside braces. */
function splitTop(s, sep) {
  const out = [];
  let depth = 0;
  let last = 0;
  const re = new RegExp(sep.source, "g" + (sep.flags.includes("i") ? "i" : ""));
  let m;
  while ((m = re.exec(s))) {
    depth = 0;
    for (let j = 0; j < m.index; j++) { if (s[j] === "{") depth++; if (s[j] === "}") depth--; }
    if (depth === 0) { out.push(s.slice(last, m.index)); last = m.index + m[0].length; }
    if (m[0].length === 0) re.lastIndex++;
  }
  out.push(s.slice(last));
  return out.map((x) => x.trim()).filter((x) => x !== "");
}

const MONTHS = { jan: 1, feb: 2, mar: 3, apr: 4, may: 5, jun: 6, jul: 7, aug: 8, sep: 9, oct: 10, nov: 11, dec: 12 };

function parseDate(s) {
  if (!s) return undefined;
  const m = /^(\d{4})(?:-(\d{1,2}))?(?:-(\d{1,2}))?/.exec(String(s).trim());
  if (!m) return { literal: String(s) };
  return { "date-parts": [[+m[1], ...(m[2] ? [+m[2]] : []), ...(m[3] ? [+m[3]] : [])]] };
}

export function parseBibtex(src) {
  const strings = { ...Object.fromEntries(Object.keys(MONTHS).map((k) => [k, String(MONTHS[k])])) };
  const items = [];
  let i = 0;
  const ws = () => { while (i < src.length && /\s/.test(src[i])) i++; };
  const readBraced = (open, close) => {
    let depth = 0;
    const start = i;
    for (; i < src.length; i++) {
      if (src[i] === "\\") { i++; continue; }
      if (src[i] === open) depth++;
      else if (src[i] === close && --depth === 0) { i++; return src.slice(start + 1, i - 1); }
    }
    throw new Error("BibTeX: Klammer nicht geschlossen");
  };
  const readValue = () => {
    let v = "";
    for (;;) {
      ws();
      if (src[i] === "{") v += readBraced("{", "}");
      else if (src[i] === "\"") {
        const start = ++i;
        let depth = 0;
        for (; i < src.length; i++) {
          if (src[i] === "\\") { i++; continue; }
          if (src[i] === "{") depth++;
          if (src[i] === "}") depth--;
          if (src[i] === "\"" && depth === 0) break;
        }
        v += src.slice(start, i++);
      } else {
        const m = /^[^\s,#}\)]+/.exec(src.slice(i));
        if (!m) break;
        i += m[0].length;
        v += /^\d+$/.test(m[0]) ? m[0] : strings[m[0].toLowerCase()] ?? m[0];
      }
      ws();
      if (src[i] !== "#") break;
      i++;
    }
    return v;
  };
  while ((i = src.indexOf("@", i)) !== -1) {
    i++;
    const tm = /^([A-Za-z]+)\s*([{(])/.exec(src.slice(i));
    if (!tm) continue;
    const type = tm[1].toLowerCase();
    i += tm[0].length;
    const close = tm[2] === "{" ? "}" : ")";
    if (type === "comment" || type === "preamble") { i -= 1; readBraced(tm[2], close); continue; }
    if (type === "string") {
      ws();
      const nm = /^[^\s=]+/.exec(src.slice(i))[0];
      i += nm.length; ws(); i++;
      strings[nm.toLowerCase()] = readValue();
      ws(); if (src[i] === close) i++;
      continue;
    }
    ws();
    const km = /^[^\s,]*/.exec(src.slice(i));
    const key = km[0];
    i += key.length;
    const fields = {};
    for (;;) {
      ws();
      if (src[i] === ",") { i++; ws(); }
      if (i >= src.length || src[i] === close) { i++; break; }
      const fm = /^[^\s=,]+/.exec(src.slice(i));
      if (!fm) { i++; continue; }
      i += fm[0].length;
      ws();
      if (src[i] !== "=") continue;
      i++;
      fields[fm[0].toLowerCase()] = readValue();
    }
    items.push(bibToCsl(type, key, fields));
  }
  return items;
}

function bibToCsl(type, key, f) {
  const t = (k) => (f[k] == null ? undefined : latexToText(f[k]));
  const item = { id: key || undefined, type: BIB_TYPES[type] ?? "document" };
  const set = (k, v) => { if (v != null && v !== "") item[k] = v; };
  set("title", t("title"));
  if (f.author) item.author = bibNames(f.author);
  if (f.editor) item.editor = bibNames(f.editor);
  set("container-title", t("journaltitle") ?? t("journal") ?? t("booktitle"));
  set("container-title-short", t("shortjournal"));
  set("collection-title", t("series"));
  set("volume", t("volume"));
  set("issue", t("number") ?? t("issue"));
  set("page", t("pages")?.replace(/\s*[–—-]+\s*/g, "-"));
  set("publisher", t("publisher") ?? t("institution") ?? t("organization") ?? t("school"));
  set("publisher-place", t("address") ?? t("location"));
  set("edition", t("edition"));
  set("DOI", t("doi")?.replace(/^https?:\/\/(dx\.)?doi\.org\//i, ""));
  set("URL", f.url?.trim());
  set("PMID", t("pmid"));
  set("ISBN", t("isbn"));
  set("ISSN", t("issn"));
  set("note", t("note"));
  set("number", type === "techreport" || type === "report" ? t("number") : undefined);
  if (item.number) delete item.issue;
  if (f.date) item.issued = parseDate(t("date"));
  else if (f.year) {
    const y = /\d{4}/.exec(t("year"))?.[0];
    const mo = f.month ? MONTHS[String(t("month")).slice(0, 3).toLowerCase()] ?? (parseInt(t("month"), 10) || undefined) : undefined;
    item.issued = y ? { "date-parts": [[+y, ...(mo ? [mo] : [])]] } : { literal: t("year") };
  }
  set("accessed", parseDate(t("urldate")));
  if (type === "phdthesis") item.genre = "Dissertation";
  return item;
}

// ---- RIS --------------------------------------------------------------------

const RIS_TYPES = {
  JOUR: "article-journal", JFULL: "article-journal", ABST: "article-journal", MGZN: "article-magazine",
  NEWS: "article-newspaper", BOOK: "book", EBOOK: "book", EDBOOK: "book", CHAP: "chapter", ECHAP: "chapter",
  CONF: "paper-conference", CPAPER: "paper-conference", RPRT: "report", GOVDOC: "report", THES: "thesis",
  ELEC: "webpage", WEB: "webpage", ICOMM: "webpage", DATA: "dataset", COMP: "software", PAT: "patent",
  STAT: "legislation", LEGAL: "legislation", BILL: "bill", CASE: "legal_case", STAND: "standard", GEN: "document",
};

export function parseRis(src) {
  const items = [];
  let rec = null;
  let last = null;
  for (const line of src.split(/\r?\n/)) {
    const m = /^([A-Z][A-Z0-9])  -(?: (.*))?$/.exec(line);
    if (!m) {
      if (rec && last && line.trim()) rec[last][rec[last].length - 1] += " " + line.trim();
      continue;
    }
    const [, tag, value = ""] = m;
    if (tag === "TY") { rec = { TY: [value.trim()] }; last = "TY"; continue; }
    if (!rec) continue;
    if (tag === "ER") { items.push(risToCsl(rec)); rec = null; continue; }
    (rec[tag] ??= []).push(value.trim());
    last = tag;
  }
  if (rec) items.push(risToCsl(rec));
  return items;
}

function risName(s) {
  const p = s.split(/\s*,\s*/);
  if (p.length >= 2) return { family: p[0], given: p.slice(1).join(" ") };
  return { literal: s };
}

function risToCsl(r) {
  const one = (...tags) => { for (const t of tags) if (r[t]?.[0]) return r[t][0]; return undefined; };
  const ty = r.TY[0];
  const item = { type: RIS_TYPES[ty] ?? "document" };
  const set = (k, v) => { if (v != null && v !== "") item[k] = v; };
  set("id", one("ID"));
  set("title", one("TI", "T1", "CT"));
  const authors = [...(r.AU ?? []), ...(r.A1 ?? [])];
  if (authors.length) item.author = authors.map(risName);
  const editors = [...(r.ED ?? []), ...(r.A2 ?? []), ...(r.A3 ?? [])];
  if (editors.length && ty !== "JOUR") item.editor = editors.map(risName);
  set("container-title", one("T2", "JF", "JO", "BT"));
  set("container-title-short", one("J2", "JA"));
  set("volume", one("VL"));
  set("issue", one("IS"));
  const sp = one("SP");
  const ep = one("EP");
  set("page", sp && ep ? `${sp}-${ep}` : sp?.replace(/\s*[–—-]+\s*/g, "-"));
  set("publisher", one("PB"));
  set("publisher-place", one("CY", "PP"));
  set("edition", one("ET"));
  set("DOI", one("DO")?.replace(/^https?:\/\/(dx\.)?doi\.org\//i, ""));
  set("URL", one("UR", "L2"));
  set("ISSN", ty === "BOOK" || ty === "CHAP" ? undefined : one("SN"));
  set("ISBN", ty === "BOOK" || ty === "CHAP" ? one("SN") : undefined);
  set("note", one("N1"));
  const an = one("AN");
  if (an && /^\d{1,9}$/.test(an)) item.PMID = an;
  const date = one("PY", "Y1", "DA");
  if (date) {
    const m = /(\d{4})(?:[/-](\d{1,2}))?(?:[/-](\d{1,2}))?/.exec(date);
    item.issued = m ? { "date-parts": [[+m[1], ...(m[2] ? [+m[2]] : []), ...(m[3] ? [+m[3]] : [])]] } : { literal: date };
  }
  const y2 = one("Y2");
  if (y2 && item.type === "webpage") item.accessed = parseDate(y2.replace(/\//g, "-"));
  return item;
}

// ---- keys and merging -------------------------------------------------------

/** Lower-case ASCII for keys and search: "Müller" → "mueller", "Señor" → "senor". */
export function fold(s) {
  return String(s ?? "")
    .toLowerCase()
    .replace(/ä/g, "ae").replace(/ö/g, "oe").replace(/ü/g, "ue").replace(/ß/g, "ss")
    .normalize("NFKD").replace(/[̀-ͯ]/g, "");
}

export function year(item) {
  const d = item.issued;
  const y = d?.["date-parts"]?.[0]?.[0] ?? /\d{4}/.exec(d?.literal ?? d?.raw ?? "")?.[0];
  return y ? String(y) : "";
}

/** A key like `mueller2020`, with a letter added until it is free. */
export function makeKey(item, taken) {
  const a = item.author?.[0] ?? item.editor?.[0];
  const name = a?.family ?? a?.literal ?? item.title ?? "quelle";
  const stem = (fold(name).match(/[a-z0-9]+/g) ?? ["quelle"]).filter((w) => !["the", "a", "der", "die", "das"].includes(w))[0] ?? "quelle";
  const base = stem.slice(0, 20) + (year(item) || "");
  if (!taken.has(base)) return base;
  for (let n = 0; ; n++) {
    let suffix = "";
    let k = n;
    do { suffix = String.fromCharCode(97 + (k % 26)) + suffix; k = Math.floor(k / 26) - 1; } while (k >= 0);
    if (!taken.has(base + suffix)) return base + suffix;
  }
}

/** What makes two records the same work, independent of their keys. */
export function identities(item) {
  const out = [];
  if (item.DOI) out.push("doi:" + item.DOI.toLowerCase().trim());
  if (item.PMID) out.push("pmid:" + String(item.PMID).trim());
  const t = fold(item.title).replace(/<[^>]+>/g, "").replace(/[^a-z0-9]+/g, "");
  if (t.length >= 12) out.push("title:" + t + ":" + year(item));
  return out;
}

const KEY_RE = /^[A-Za-z0-9_:.\-]+$/;

/**
 * Adds incoming records to a library. A record that is already there, by DOI,
 * PMID, title and year, or key, updates the existing entry.
 * Returns the new library and counts.
 */
export function mergeItems(library, incoming) {
  const out = { ...library };
  const taken = new Set(Object.keys(out));
  const byIdentity = new Map();
  for (const [k, it] of Object.entries(out)) for (const id of identities(it)) byIdentity.set(id, k);
  let added = 0;
  let updated = 0;
  for (const raw of incoming) {
    const { id: wanted, ...rest } = raw;
    const same = identities(rest).map((x) => byIdentity.get(x)).find(Boolean);
    let key;
    // A known work keeps its key, so citations in the document stay valid. A
    // key the file shares with a work of another DOI or PMID gets a new key;
    // without such ids the file's entry counts as a correction.
    const strong = (it) => identities(it).filter((x) => !x.startsWith("title:"));
    const clash = wanted && out[wanted] && strong(out[wanted]).length && strong(rest).length;
    if (same) key = same;
    else if (wanted && KEY_RE.test(wanted) && !clash) key = wanted;
    else key = makeKey(rest, taken);
    if (out[key]) updated++; else added++;
    out[key] = { ...rest, id: key };
    taken.add(key);
    for (const x of identities(rest)) byIdentity.set(x, key);
  }
  return { library: out, added, updated };
}
