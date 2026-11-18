#!/usr/bin/env bash
# Builds the test corpus.
#
#   corpus/build.sh word     # the Word-made documents, in a Word of its own
#   corpus/build.sh pandoc   # the pandoc-made document
#   corpus/build.sh edge     # the hand-written document of edge cases
#   corpus/build.sh read [FILE...]  # Word's readings of every document, or of some
#
# The Word steps run from WSL against the Word on the Windows host. They
# work in a fresh directory under the Windows %TEMP% and remove it after.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
PS=/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe
limit=${TABELLION_WORD_TIMEOUT:-600}

word() {
  local wintemp dir
  wintemp=$("$PS" -NoProfile -Command '[IO.Path]::GetTempPath()' | tr -d '\r')
  dir="$(wslpath -u "$wintemp")tabellion-corpus-$(date +%s%N)"
  mkdir -p "$dir"
  # Windows PowerShell reads a script without a byte order mark as ANSI.
  { printf '\xEF\xBB\xBF'; cat "$here/build/word.ps1"; } > "$dir/word.ps1"
  python3 "$here/build/png.py" "$dir/bird.png" 120 80 200 225 190
  python3 "$here/build/png.py" "$dir/heron.png" 96 64 190 210 235
  local status=0
  timeout "$limit" "$PS" -NoProfile -ExecutionPolicy Bypass -File "$(wslpath -w "$dir/word.ps1")" \
    -Dir "$(wslpath -w "$dir")" || status=$?
  if [ $status -eq 124 ] && [ -f "$dir/word.pid" ]; then
    local pid
    pid=$(tr -dc 0-9 < "$dir/word.pid")
    "$PS" -NoProfile -Command "Stop-Process -Id $pid -Force" >/dev/null 2>&1 || true
    echo "corpus/build.sh: Word took longer than ${limit}s; stopped its process $pid" >&2
  fi
  for f in birds parts recipes-de recettes-fr; do
    [ -f "$dir/$f.docx" ] && cp "$dir/$f.docx" "$here/$f.docx"
  done
  rm -rf "$dir"
  return $status
}

pandoc_doc() {
  local dir
  dir=$(mktemp -d)
  python3 "$here/build/png.py" "$dir/bird.png" 120 80 200 225 190
  pandoc --from markdown --to docx --resource-path "$dir" --output "$here/pandoc.docx" "$here/build/pandoc.md"
  rm -rf "$dir"
}

edge_doc() {
  local dir
  dir=$(mktemp -d)
  python3 "$here/build/png.py" "$dir/bird.png" 120 80 200 225 190
  python3 "$here/build/edge.py" "$here/edge.docx" "$dir/bird.png"
  rm -rf "$dir"
}

read_all() {
  local out files
  out=$(mktemp -d)
  if [ $# -gt 0 ]; then files=("$@"); else files=("$here"/*.docx); fi
  TABELLION_WORD_READ=1 "$here/../tools/word/check.sh" "$out" "${files[@]}" >/dev/null
  for f in "${files[@]}"; do
    local stem
    stem=$(basename "$f" .docx)
    python3 "$here/build/reading.py" "$out/$stem.json" > "$here/$stem.word.json"
  done
  rm -rf "$out"
}

case ${1:-} in
  word) word ;;
  pandoc) pandoc_doc ;;
  edge) edge_doc ;;
  read) shift; read_all "$@" ;;
  *) echo "usage: corpus/build.sh word|pandoc|edge|read" >&2; exit 2 ;;
esac
