#!/usr/bin/env bash
# Regenerates every spike output into spike/out/ and times the engine.
#
#   spike/run.sh [REAL.docx]
#
# With REAL.docx, also time a tracked replacement in that file: set
# REAL_BLOCK (a block id from `tabellion inspect REAL.docx blocks`),
# REAL_FIND and REAL_WITH.
#
# Needs cargo, and R with the package installed for the R part
# (R CMD INSTALL r/tabellion). The optional template is a real-world file to
# time and edit; it is read, never committed.
set -euo pipefail
cd "$(dirname "$0")/.."
out=spike/out
mkdir -p "$out"
cargo build --release -q -p tabellion -p nebu-docx --examples --bins
T=target/release/tabellion

samples/build.sh
cargo run --release -q -p nebu-docx --example generate -- samples/report-template.docx 300 "$out/large.docx"

echo "== large generated document (300 chapters)"
$T bench "$out/large.docx" --block '#1801' --find "Region 7" --with "Region seven" --runs 7 --out "$out/large.tracked.docx"
$T roundtrip "$out/large.docx" "$out/large.roundtrip.docx"

if [ $# -ge 1 ]; then
  echo "== $(basename "$1")"
  cp "$1" "$out/real.docx"
  $T bench "$out/real.docx" --block "$REAL_BLOCK" --find "$REAL_FIND" --with "$REAL_WITH" \
    --runs 7 --out "$out/real.tracked.docx"
  $T roundtrip "$out/real.docx" "$out/real.roundtrip.docx"
fi

echo "== Typst fragment"
$T insert samples/report-template.docx "$out/typst-fragment.docx" --at '{{results}}' --typst spike/typst/fragment.typ

echo "== R report"
# --no-init-file: the legacy rv profile in the repository root would replace
# the library path.
Rscript --no-init-file spike/r/report.R
