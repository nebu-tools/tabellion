#!/usr/bin/env bash
# Zips the sample template sources into report-template.docx. The order puts
# [Content_Types].xml first, as Office does. Fixed times in UTC and -X, which
# drops extra file attributes, make the archive the same on every machine.
set -euo pipefail
export TZ=UTC
cd "$(dirname "$0")/report-template"
rm -f ../report-template.docx
find . -type f -exec touch -t 202601010000 {} +
zip -X -q -D ../report-template.docx '[Content_Types].xml' _rels/.rels docProps/core.xml docProps/app.xml \
  word/document.xml word/_rels/document.xml.rels word/styles.xml word/numbering.xml word/settings.xml \
  word/footnotes.xml word/footer1.xml
