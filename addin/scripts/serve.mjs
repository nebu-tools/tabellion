// Serves a built add-in over http for local use. Word on Windows accepts
// http://localhost for a sideloaded add-in; anything else needs https.
//
//   node scripts/build.mjs --base http://localhost:3000 && node scripts/serve.mjs [dist] [3000]
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const TYPES = { ".html": "text/html; charset=utf-8", ".js": "text/javascript; charset=utf-8", ".css": "text/css; charset=utf-8", ".png": "image/png", ".xml": "application/xml" };

/** Starts a static server. POST /log bodies go to `onLog` as parsed JSON. */
export function serve(dir, port, onLog = () => {}) {
  const root = path.resolve(dir);
  const server = http.createServer((req, res) => {
    if (req.method === "POST" && req.url === "/log") {
      let body = "";
      req.on("data", (d) => (body += d));
      req.on("end", () => {
        try { onLog(JSON.parse(body)); } catch { onLog({ raw: body }); }
        res.end("ok");
      });
      return;
    }
    const rel = decodeURIComponent(new URL(req.url, "http://x").pathname);
    const file = path.join(root, rel === "/" ? "taskpane.html" : rel);
    if (!file.startsWith(root + path.sep)) { res.statusCode = 403; res.end(); return; }
    fs.readFile(file, (err, data) => {
      if (err) { res.statusCode = 404; res.end(); return; }
      res.setHeader("Content-Type", TYPES[path.extname(file)] ?? "application/octet-stream");
      res.setHeader("Cache-Control", "no-store");
      res.end(data);
    });
  });
  return new Promise((resolve) => server.listen(port, "127.0.0.1", () => resolve(server)));
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const dir = process.argv[2] ?? "dist";
  const port = +(process.argv[3] ?? 3000);
  await serve(dir, port);
  console.log(`http://localhost:${port}/ (${dir}/manifest.xml zum Sideloaden)`);
}
