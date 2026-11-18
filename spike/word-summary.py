"""Summarises tools/word/check.sh reports: open status, field agreement,
revisions, table styles, accept/reject text."""
import json, sys, glob, os

for path in sorted(glob.glob(os.path.join(sys.argv[1], "*.json"))):
    r = json.load(open(path, encoding="utf-8-sig"))
    for f in r["files"]:
        name = os.path.basename(f["file"].replace("\\", "/"))
        print(f"== {name}: opened={f['opened']} error={f.get('error')} pages={f.get('pages')} "
              f"open_ms={f.get('open_ms')} update_ms={f.get('update_ms')}")
        if not f["opened"]:
            continue
        before, after = f.get("fields_before") or [], f.get("fields_after") or []
        diff = [(a["code"], a["result"], b["result"]) for a, b in zip(before, after) if a["result"] != b["result"]]
        kinds = {}
        for a in before:
            k = a["code"].split()[0] if a["code"] else "?"
            kinds[k] = kinds.get(k, 0) + 1
        print(f"   fields {len(before)} {kinds}; changed by Word's update: {len(diff)}")
        for d in diff[:5]:
            print("     ", d)
        revs = f.get("revisions") or []
        print(f"   revisions {len(revs)}: " + "; ".join(f"{x['type']}/{x['author']}/{(x['text'] or '')[:40]!r}" for x in revs[:4]))
        print("   table styles:", [(s["name"], "hidden" if s["visibility"] else "shown") for s in f.get("table_styles") or []])
        print("   tables:", sorted({t["style"] for t in f.get("tables") or []}), f.get("table_count"))
        if f.get("footnotes"):
            print("   footnotes:", len(f["footnotes"]))
        for k in ("accepted_text", "rejected_text"):
            if f.get(k) is not None:
                print(f"   {k}: has_new={('seven' in f[k]) or ('Szenario A' in f[k]) or ('tracked change' in f[k])} "
                      f"has_old={('Region 7' in f[k]) or ('Szenario 1' in f[k])}")
