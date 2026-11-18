"""Turns a check.ps1 report of one file into the reading kept in the corpus.

    python3 reading.py REPORT.json > NAME.word.json

The reading drops what changes from run to run (times, process ids, the
temporary path) and keeps what Word reports about the document.
"""
import json
import sys

report = json.load(open(sys.argv[1], encoding="utf-8-sig"))
f = report["files"][0]
if not f.get("opened"):
    sys.exit(f"Word could not open {f.get('file')}: {f.get('error')}")
reading = f["reading"]
reading = {"word_version": report["word_version"].split(" build ")[0], **reading}
json.dump(reading, sys.stdout, ensure_ascii=False, indent=1, sort_keys=False)
sys.stdout.write("\n")
