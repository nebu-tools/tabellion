// Fetches a record by DOI (doi.org, CSL-JSON) or PubMed id (NCBI E-utilities).
// Both answer browsers from any origin.

const VARS = new Set([
  "type", "title", "title-short", "container-title", "container-title-short", "collection-title", "author", "editor",
  "issued", "accessed", "volume", "issue", "page", "number", "edition", "publisher", "publisher-place", "DOI", "PMID",
  "PMCID", "URL", "ISBN", "ISSN", "note", "genre", "language", "event-title", "medium", "status",
]);

// doi.org passes some Crossref types through unchanged.
const CROSSREF_TYPES = {
  article: "article-journal", "journal-article": "article-journal", "book-chapter": "chapter",
  "proceedings-article": "paper-conference", "posted-content": "article", monograph: "book", "edited-book": "book",
};

/** Keeps the standard CSL variables and drops names' extras, e.g. Crossref's affiliations. */
export function cleanCsl(item) {
  const out = {};
  for (const [k, v] of Object.entries(item)) {
    if (!VARS.has(k) || v == null || v === "") continue;
    if (k === "author" || k === "editor") {
      out[k] = v.map((n) => Object.fromEntries(["family", "given", "literal", "suffix", "non-dropping-particle", "dropping-particle"].filter((f) => n[f]).map((f) => [f, n[f]])));
    } else if (Array.isArray(v) && typeof v[0] === "string") {
      out[k] = v[0];
    } else if (k === "issued" || k === "accessed") {
      out[k] = v["date-parts"] ? { "date-parts": v["date-parts"] } : v;
    } else {
      out[k] = v;
    }
  }
  out.type = CROSSREF_TYPES[out.type] ?? out.type;
  return out;
}

export function parseIdent(s) {
  const t = s.trim();
  const doi = /(10\.\d{4,9}\/\S+)/.exec(t);
  if (doi) return { doi: doi[1].replace(/[.,;]$/, "") };
  const pmid = /^(?:pmid:?\s*)?(\d{1,9})$/i.exec(t);
  if (pmid) return { pmid: pmid[1] };
  return null;
}

const MONTHS = { jan: 1, feb: 2, mar: 3, apr: 4, may: 5, jun: 6, jul: 7, aug: 8, sep: 9, oct: 10, nov: 11, dec: 12 };

/** A PubMed summary (esummary.fcgi, JSON) as CSL-JSON. */
export function fromEsummary(r) {
  const item = { type: "article-journal", PMID: String(r.uid) };
  const set = (k, v) => { if (v) item[k] = v; };
  set("title", r.title?.replace(/\.$/, ""));
  set("container-title", r.source);
  set("volume", r.volume);
  set("issue", r.issue);
  set("page", r.pages);
  set("DOI", r.articleids?.find((a) => a.idtype === "doi")?.value);
  const names = (r.authors ?? []).filter((a) => a.name);
  if (names.length) {
    item.author = names.map((a) => {
      const m = /^(.*\S)\s+([A-Z]{1,4})$/.exec(a.name);
      return a.authtype === "Author" && m ? { family: m[1], given: m[2] } : { literal: a.name };
    });
  }
  const d = /^(\d{4})(?:\s+([A-Za-z]{3}))?(?:\s+(\d{1,2}))?/.exec(r.pubdate ?? "");
  if (d) {
    const mo = d[2] && MONTHS[d[2].toLowerCase()];
    item.issued = { "date-parts": [[+d[1], ...(mo ? [mo] : []), ...(mo && d[3] ? [+d[3]] : [])]] };
  }
  return item;
}

export async function lookup(s, fetchImpl = fetch) {
  const id = parseIdent(s);
  if (!id) throw new Error("Weder DOI noch PMID: " + s);
  if (id.pmid) {
    const res = await fetchImpl(`https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esummary.fcgi?db=pubmed&retmode=json&id=${id.pmid}`);
    const r = res.ok ? (await res.json()).result?.[id.pmid] : null;
    if (!r || r.error) throw new Error(`PMID nicht gefunden (${r?.error ?? res.status})`);
    return fromEsummary(r);
  }
  const res = await fetchImpl("https://doi.org/" + encodeURIComponent(id.doi).replace(/%2F/g, "/"), {
    headers: { Accept: "application/vnd.citationstyles.csl+json" },
  });
  if (!res.ok) throw new Error(`DOI nicht gefunden (${res.status})`);
  const data = await res.json();
  return cleanCsl(Array.isArray(data) ? data[0] : data);
}
