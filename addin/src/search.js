// Fuzzy search over a library. Every word of the query has to match somewhere:
// key, author, year, title, journal, DOI or PMID. Better matches rank higher.

import { fold, year } from "./library.js";

const strip = (s) => fold(String(s ?? "").replace(/<[^>]+>/g, ""));
const words = (s) => strip(s).match(/[a-z0-9]+/g) ?? [];

export function buildIndex(library) {
  return Object.values(library).map((item) => {
    const names = [...(item.author ?? []), ...(item.editor ?? [])];
    return {
      item,
      key: strip(item.id),
      authors: names.flatMap((n) => words(n.family ?? n.literal)),
      first: words(names[0]?.family ?? names[0]?.literal)[0] ?? "",
      year: year(item),
      title: words(item.title),
      rest: [...words(item["container-title"]), ...words(item["container-title-short"]), ...words(item.publisher)],
      ids: [strip(item.DOI), strip(item.PMID), strip(item.URL)].filter(Boolean),
    };
  });
}

/** Damerau-Levenshtein distance up to `max`; larger distances come back as max + 1. */
function distance(a, b, max) {
  if (Math.abs(a.length - b.length) > max) return max + 1;
  let prev2 = null;
  let prev = Array.from({ length: b.length + 1 }, (_, j) => j);
  for (let i = 1; i <= a.length; i++) {
    const cur = [i];
    let best = i;
    for (let j = 1; j <= b.length; j++) {
      let v = Math.min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1));
      if (prev2 && i > 1 && j > 1 && a[i - 1] === b[j - 2] && a[i - 2] === b[j - 1]) v = Math.min(v, prev2[j - 2] + 1);
      cur.push(v);
      best = Math.min(best, v);
    }
    if (best > max) return max + 1;
    prev2 = prev;
    prev = cur;
  }
  return prev[b.length];
}

/** How well one query word matches a list of words. 0 means no match. */
function matchWords(q, list, exact, prefix, typo) {
  let s = 0;
  for (const w of list) {
    if (w === q) return exact;
    if (w.startsWith(q)) s = Math.max(s, prefix);
    else if (q.length >= 4 && typo && distance(q, w.slice(0, q.length + 1), 1) <= 1) s = Math.max(s, typo);
  }
  return s;
}

function scoreWord(e, q) {
  let s = 0;
  if (e.key === q) s = 120;
  else if (e.key.startsWith(q)) s = 70;
  else if (q.length >= 3 && e.key.includes(q)) s = 30;
  if (e.first && (e.first === q || e.first.startsWith(q))) s = Math.max(s, e.first === q ? 80 : 60);
  s = Math.max(s, matchWords(q, e.authors, 50, 40, 15));
  if (/^\d{4}$/.test(q)) s = Math.max(s, e.year === q ? 45 : 0);
  s = Math.max(s, matchWords(q, e.title, 25, 20, 8), matchWords(q, e.rest, 15, 12, 4));
  if (q.length >= 4 && e.ids.some((x) => x.includes(q))) s = Math.max(s, 90);
  return s;
}

/** Items that match every query word, best first. An empty query lists all, newest first. */
export function search(index, query, limit = 50) {
  const qs = fold(query).match(/[a-z0-9./]+/g)?.map((w) => w.replace(/^[./]+|[./]+$/g, "")).filter(Boolean) ?? [];
  const hits = [];
  for (const e of index) {
    let total = 0;
    for (const q of qs) {
      const s = scoreWord(e, q);
      if (!s) { total = -1; break; }
      total += s;
    }
    if (total >= 0) hits.push({ e, total });
  }
  hits.sort((a, b) => b.total - a.total || b.e.year.localeCompare(a.e.year) || a.e.key.localeCompare(b.e.key));
  return hits.slice(0, limit).map((h) => h.e.item);
}
