// End-to-end test in real Word, from WSL on a Windows machine with Word.
// Builds the e2e pane, sideloads it for one Word instance at a time, opens
// documents that open the pane by themselves and lets e2e/page.js drive it:
//
//   A  an empty document: citing, markers, lists, deleting, track changes, DOI
//   B  the G-BA Modul 3 template: lists in two "Referenzliste" sections
//   R  `gba.R apply` rewrites B's result, as the pipeline does with team files
//   C  Word reopens R's result: everything still current, citing goes on
//
//   npm run e2e        results in dist-e2e/{a,b,r,c}.docx
import { execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { serve } from "../scripts/serve.mjs";

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");
const repo = path.join(root, "..");
const PS = "/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe";
const PORT = 3187;
const ADDIN_ID = /<Id>([^<]+)<\/Id>/.exec(fs.readFileSync(path.join(root, "manifest.xml"), "utf8"))[1];
const TEMPLATE = fs.readdirSync(path.join(repo, "templates"), { withFileTypes: true }).map((d) => d.name).find((n) => /Modul3\.docx$/.test(n));

if (!fs.existsSync(PS)) {
  console.error("Braucht WSL auf Windows mit Word.");
  process.exit(2);
}
if (!TEMPLATE) {
  console.error("Modul-3-Vorlage fehlt: erst `rv run gba.R fetch`.");
  process.exit(2);
}

execFileSync("node", [path.join(root, "scripts/build.mjs"), "--e2e", "--base", `http://localhost:${PORT}`], { stdio: "inherit" });
const dist = path.join(root, "dist-e2e");

const XML = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>';
const EMPTY = {
  "[Content_Types].xml": `${XML}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>`,
  "_rels/.rels": `${XML}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>`,
  "word/document.xml": `${XML}<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p/></w:body></w:document>`,
};

/** Copies a docx (or makes an empty one) and adds a task pane that opens with it. */
function withPane(base, file) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "zitate-docx-"));
  if (base) execFileSync("unzip", ["-q", base, "-d", dir]);
  else for (const [p, s] of Object.entries(EMPTY)) { fs.mkdirSync(path.dirname(path.join(dir, p)), { recursive: true }); fs.writeFileSync(path.join(dir, p), s); }
  const put = (p, s) => { fs.mkdirSync(path.dirname(path.join(dir, p)), { recursive: true }); fs.writeFileSync(path.join(dir, p), s); };
  const ct = path.join(dir, "[Content_Types].xml");
  fs.writeFileSync(ct, fs.readFileSync(ct, "utf8").replace("</Types>", '<Override PartName="/word/webextensions/taskpanes.xml" ContentType="application/vnd.ms-office.webextensiontaskpanes+xml"/><Override PartName="/word/webextensions/webextension1.xml" ContentType="application/vnd.ms-office.webextension+xml"/></Types>'));
  const rels = path.join(dir, "_rels/.rels");
  fs.writeFileSync(rels, fs.readFileSync(rels, "utf8").replace("</Relationships>", '<Relationship Id="rIdZitate" Type="http://schemas.microsoft.com/office/2011/relationships/webextensiontaskpanes" Target="word/webextensions/taskpanes.xml"/></Relationships>'));
  put("word/webextensions/taskpanes.xml", `${XML}<wetp:taskpanes xmlns:wetp="http://schemas.microsoft.com/office/webextensions/taskpanes/2010/11"><wetp:taskpane dockstate="right" visibility="1" width="350" row="0"><wetp:webextensionref xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:id="rId1"/></wetp:taskpane></wetp:taskpanes>`);
  put("word/webextensions/_rels/taskpanes.xml.rels", `${XML}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.microsoft.com/office/2011/relationships/webextension" Target="webextension1.xml"/></Relationships>`);
  put("word/webextensions/webextension1.xml", `${XML}<we:webextension xmlns:we="http://schemas.microsoft.com/office/webextensions/webextension/2010/11" id="{0A1B2C3D-0000-4000-8000-000000000001}"><we:reference id="${ADDIN_ID}" version="1.0.0.0" store="developer" storeType="Registry"/><we:alternateReferences/><we:properties><we:property name="Office.AutoShowTaskpaneWithDocument" value="true"/></we:properties><we:bindings/><we:snapshot xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"/></we:webextension>`);
  fs.rmSync(file, { force: true });
  execFileSync("zip", ["-X", "-q", "-r", file, "."], { cwd: dir });
  fs.rmSync(dir, { recursive: true });
}

const win = execFileSync(PS, ["-NoProfile", "-Command", "$env:TEMP"], { encoding: "utf8" }).trim();
const winDir = `${win}\\zitate-e2e`;
const dir = execFileSync("wslpath", ["-u", winDir], { encoding: "utf8" }).trim();
let log = () => {};
const server = await serve(dist, PORT, (msg) => log(msg));

/** Opens `docx` in Word with the e2e pane and returns its checks and the saved file. */
async function inWord(name, docx, out) {
  console.log(`\n== ${name}`);
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(dir, { recursive: true });
  fs.copyFileSync(path.join(dist, "manifest.xml"), path.join(dir, "manifest.xml"));
  fs.copyFileSync(path.join(root, "e2e/word.ps1"), path.join(dir, "word.ps1"));
  fs.copyFileSync(docx, path.join(dir, "in.docx"));
  const checks = [];
  let done = null;
  log = (msg) => {
    if (msg.check) {
      checks.push(msg);
      console.log(`${msg.ok ? "ok  " : "FAIL"} ${msg.check}${msg.ok ? "" : "  " + JSON.stringify(msg.detail)}`);
    } else if (msg.done) {
      done = msg;
      fs.writeFileSync(path.join(dir, "done"), "");
    } else {
      console.log("    " + JSON.stringify(msg));
    }
  };
  const code = await new Promise((resolve) => {
    const p = spawn(PS, ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", `${winDir}\\word.ps1`, "-Dir", winDir, "-AddinId", ADDIN_ID], { stdio: "inherit" });
    p.on("exit", resolve);
  });
  const saved = fs.existsSync(path.join(dir, "out.docx"));
  if (saved) fs.copyFileSync(path.join(dir, "out.docx"), out);
  fs.rmSync(dir, { recursive: true, force: true });
  const ok = code === 0 && saved && done?.ok === true && checks.length > 0;
  if (!ok) console.log(`FAIL ${name}: ${done?.failed?.join(", ") || (saved ? "kein Abschluss" : "nicht gespeichert")}`);
  return ok;
}

function check(name, pass, detail) {
  console.log(`${pass ? "ok  " : "FAIL"} ${name}${pass ? "" : "  " + JSON.stringify(detail)}`);
  return pass;
}

const outline = (docx) => JSON.parse(execFileSync("rv", ["run", "gba.R", "outline", docx], { cwd: repo, encoding: "utf8" }));
const file = (n) => path.join(dist, n);
let ok = true;
try {
  withPane(null, file("a-in.docx"));
  ok &&= await inWord("A: leeres Dokument", file("a-in.docx"), file("a.docx"));
  if (ok) {
    const doc = execFileSync("unzip", ["-p", file("a.docx"), "word/document.xml"], { encoding: "utf8" });
    ok &&= check("A saved: citations as content controls", (doc.match(/<w:tag w:val="zitat:/g) ?? []).length >= 7);
    ok &&= check("A saved: two reference lists", (doc.match(/<w:tag w:val="literaturverzeichnis"/g) ?? []).length === 2);
  }

  withPane(path.join(repo, "templates", TEMPLATE), file("b-in.docx"));
  ok &&= await inWord("B: Modul-3-Vorlage", file("b-in.docx"), file("b.docx"));

  if (ok) {
    console.log("\n== R: gba.R apply (Felder) und outline");
    const ops = file("ops.json");
    fs.writeFileSync(ops, JSON.stringify({ aenderungen: [{ op: "felder" }] }));
    execFileSync("rv", ["run", "gba.R", "apply", file("b.docx"), ops, file("r.docx")], { cwd: repo, stdio: ["ignore", "ignore", "inherit"] });
    const o = outline(file("r.docx"));
    ok &&= check("R outline: citations", JSON.stringify(o.zitate.map((z) => [z.schluessel, z.text])) === JSON.stringify([[["mueller2020"], "[1]"], [["csr"], "[2]"], [["lee2010", "mueller2020"], "[1,2]"]]), o.zitate);
    ok &&= check("R outline: two lists, four sources", o.literaturverzeichnisse.length === 2 && o.quellen === 4, [o.literaturverzeichnisse, o.quellen]);
    ok &&= await inWord("C: R-Ergebnis wieder in Word", file("r.docx"), file("c.docx"));
  }
} finally {
  server.close();
}
console.log(ok ? "\nE2E bestanden." : "\nE2E fehlgeschlagen.");
process.exit(ok ? 0 : 1);
