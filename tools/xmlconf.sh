#!/usr/bin/env bash
# Downloads the W3C XML Conformance Test Suite into target/xmlconf, once,
# and runs the parser against it (crates/nebu-docx/tests/xmlconf.rs).
set -euo pipefail
cd "$(dirname "$0")/.."
url=https://www.w3.org/XML/Test/xmlts20130923.tar.gz
sum=9b61db9f5dbffa545f4b8d78422167083a8568c59bd1129f94138f936cf6fc1f
dir=target/xmlconf
if [ ! -f "$dir/xmlconf/xmlconf.xml" ]; then
  mkdir -p "$dir"
  curl -fsSL -o "$dir/xmlts.tar.gz" "$url"
  echo "$sum  $dir/xmlts.tar.gz" | sha256sum -c --quiet
  tar -xzf "$dir/xmlts.tar.gz" -C "$dir"
fi
TABELLION_XMLCONF="$PWD/$dir/xmlconf" cargo test -p nebu-docx --test xmlconf -- --ignored --nocapture
