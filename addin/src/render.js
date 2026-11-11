// Formats citations and reference lists with citeproc-js. A run is
// {text, italic?, bold?, superscript?, subscript?, smallCaps?}.

import CSL from "citeproc";

/** Turns citeproc's HTML into runs. Block elements become paragraph breaks ("\n"). */
export function htmlToRuns(html) {
  const runs = [];
  const stack = [{}];
  const re = /<(\/?)([a-z]+)([^>]*)>|([^<]+)/gi;
  let m;
  while ((m = re.exec(html))) {
    if (m[4] != null) {
      const text = decode(m[4]);
      if (!text) continue;
      const fmt = stack[stack.length - 1];
      const prev = runs[runs.length - 1];
      if (prev && sameFormat(prev, fmt)) prev.text += text;
      else runs.push({ text, ...fmt });
      continue;
    }
    const [, close, tag, attrs] = m;
    if (close) { if (stack.length > 1) stack.pop(); continue; }
    if (tag === "br") { runs.push({ text: "\n" }); continue; }
    const f = { ...stack[stack.length - 1] };
    if (tag === "i" || tag === "em") f.italic = !f.italic;
    if (tag === "b" || tag === "strong") f.bold = true;
    if (tag === "sup") f.superscript = true;
    if (tag === "sub") f.subscript = true;
    if (/font-style:\s*italic/.test(attrs)) f.italic = true;
    if (/font-style:\s*normal/.test(attrs)) f.italic = false;
    if (/font-weight:\s*bold/.test(attrs)) f.bold = true;
    if (/font-variant:\s*small-caps/.test(attrs)) f.smallCaps = true;
    if (/vertical-align:\s*sup/.test(attrs)) f.superscript = true;
    if (/vertical-align:\s*sub/.test(attrs)) f.subscript = true;
    for (const k of Object.keys(f)) if (!f[k]) delete f[k];
    stack.push(f);
  }
  return runs;
}

const FLAGS = ["italic", "bold", "superscript", "subscript", "smallCaps"];
const sameFormat = (a, b) => FLAGS.every((k) => !!a[k] === !!b[k]);

function decode(s) {
  return s
    .replace(/&#(\d+);/g, (_, n) => String.fromCodePoint(+n))
    .replace(/&#x([0-9a-f]+);/gi, (_, n) => String.fromCodePoint(parseInt(n, 16)))
    .replace(/&nbsp;/g, " ").replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&quot;/g, "\"").replace(/&amp;/g, "&")
    .replace(/\s*\n\s*/g, "");
}

export const runsText = (runs) => runs.map((r) => r.text).join("");

/**
 * One reference list entry: number, tab, text. citeproc puts the number in
 * csl-left-margin when the style aligns it; other styles give plain text.
 */
function entryRuns(html) {
  const left = /<div class="csl-left-margin">([\s\S]*?)<\/div>/.exec(html);
  const right = /<div class="csl-right-inline">([\s\S]*?)<\/div>/.exec(html);
  if (left && right) return [...htmlToRuns(left[1]), { text: "\t" }, ...htmlToRuns(right[1])];
  return htmlToRuns(html.replace(/<\/?div[^>]*>/g, ""));
}

/**
 * Creates a renderer for one style. `locales` maps a language to locale XML.
 * render(sections, library) takes [{citations: [{items: [{id, locator?, label?}]}]}]
 * and returns, per section, the runs of each citation and of each entry.
 */
export function createRenderer({ style, locales, lang = "de-DE" }) {
  return {
    render(sections, library) {
      const sys = {
        retrieveLocale: (l) => locales[l] ?? locales[Object.keys(locales).find((k) => k.startsWith(l.slice(0, 2)))] ?? locales["en-US"],
        retrieveItem: (id) => library[id],
      };
      return sections.map((sec) => {
        if (!sec.citations.length) return { citations: [], entries: [] };
        // A fresh engine per section restarts the numbering, as each G-BA
        // reference list counts from 1.
        const engine = new CSL.Engine(sys, style, lang, true);
        const cits = sec.citations.map((c, i) => ({
          citationID: "c" + i,
          citationItems: c.items.map((x) => ({ id: x.id, ...(x.locator ? { locator: x.locator, label: x.label ?? "page" } : {}) })),
          properties: { noteIndex: 0 },
        }));
        const res = engine.rebuildProcessorState(cits, "html");
        const byId = new Map(res.map(([id, , html]) => [id, html]));
        const [, entries] = engine.makeBibliography();
        return {
          citations: cits.map((c) => htmlToRuns(byId.get(c.citationID) ?? "")),
          entries: entries.map(entryRuns),
        };
      });
    },
  };
}
