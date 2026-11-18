#!/usr/bin/env python3
"""Writes the definitions of the built-in styles that tabellion adds to a
document that lacks them, as Word wrote them in a document it saved.

    python3 -I builtin-styles.py SAVED.docx OUT.xml

Each style is copied as Word stored it, without the w:rsid that records the
editing session and without the w:link to the character style that Word
makes alongside a paragraph style ("Heading 1 Char"), which tabellion does
not add.
"""
import re
import sys
import zipfile

WANTED = [
    "Heading1", "Heading2", "Heading3", "Heading4", "Heading5", "Heading6",
    "Heading7", "Heading8", "Heading9", "Caption", "ListParagraph",
    "FootnoteText", "FootnoteReference", "Hyperlink", "HTMLCode",
    "TableNormal", "TableGrid",
]
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"

saved, out = sys.argv[1], sys.argv[2]
with zipfile.ZipFile(saved) as z:
    styles = z.read("word/styles.xml").decode("utf-8")
found = {}
for m in re.finditer(r'<w:style [^>]*>.*?</w:style>', styles, re.S):
    s = m.group(0)
    sid = re.search(r'w:styleId="([^"]*)"', s).group(1)
    if sid in WANTED:
        s = re.sub(r'<w:rsid w:val="[0-9A-F]+"/>', "", s)
        s = re.sub(r'<w:link w:val="[^"]*"/>', "", s)
        found[sid] = s
missing = [w for w in WANTED if w not in found]
if missing:
    sys.exit("builtin-styles.py: Word did not write " + ", ".join(missing))
with open(out, "w", encoding="utf-8", newline="\n") as f:
    f.write('<?xml version="1.0" encoding="UTF-8"?>\n')
    f.write("<!-- Word's built-in styles as Word 2021 writes them, from tools/word/builtin-styles.sh. -->\n")
    f.write(f'<w:styles xmlns:w="{W}">\n')
    for w in WANTED:
        f.write(found[w] + "\n")
    f.write("</w:styles>\n")
