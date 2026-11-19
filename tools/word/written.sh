#!/usr/bin/env bash
# Has Word read the documents the writer tests make, from WSL, and writes
# what it reports next to them (corpus/written/NAME.word.json) with the
# file Word saved (NAME.saved.docx).
#
#   tools/word/written.sh [NAME...]
#
# Each file is read in a Word instance of its own, started by written.ps1
# through Word.psm1, in a fresh directory under the Windows %TEMP% that the
# script removes after. A run longer than TABELLION_WORD_TIMEOUT seconds
# (default 180) almost always waits on a repair dialog: the script stops
# that one Word, by the process id written.ps1 recorded, and reports the
# file as not opened.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
dir_out="$here/../../corpus/written"
PS=/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe
limit=${TABELLION_WORD_TIMEOUT:-180}
if [ $# -gt 0 ]; then names=("$@"); else names=(); for f in "$dir_out"/*.docx; do case $f in *.saved.docx) ;; *) names+=("$(basename "$f" .docx)") ;; esac; done; fi
wintemp=$("$PS" -NoProfile -Command '[IO.Path]::GetTempPath()' | tr -d '\r')
for name in "${names[@]}"; do
  dir="$(wslpath -u "$wintemp")tabellion-written-$(date +%s%N)"
  mkdir -p "$dir"
  # Windows PowerShell reads a script without a byte order mark as ANSI.
  for f in Word.psm1 written.ps1; do { printf '\xEF\xBB\xBF'; cat "$here/$f"; } > "$dir/$f"; done
  cp "$dir_out/$name.docx" "$dir/"
  w() { wslpath -w "$dir/$1"; }
  timeout "$limit" "$PS" -NoProfile -ExecutionPolicy Bypass -File "$(w written.ps1)" \
    -Out "$(w report.json)" -PidFile "$(w word.pid)" -Saved "$(w saved.docx)" -File "$(w "$name.docx")" >/dev/null 2>"$dir/stderr"
  status=$?
  if [ $status -eq 124 ] && [ -f "$dir/word.pid" ]; then
    pid=$(tr -dc 0-9 < "$dir/word.pid")
    "$PS" -NoProfile -Command "Stop-Process -Id $pid -Force" >/dev/null 2>&1
    sleep 2
    printf '{"file":"%s.docx","opened":false,"error":"timeout after %ss, probably a repair dialog"}\n' "$name" "$limit" > "$dir_out/$name.word.json"
  elif [ -f "$dir/report.json" ]; then
    sed '1s/^\xEF\xBB\xBF//' "$dir/report.json" | tr -d '\r' > "$dir_out/$name.word.json"
    [ -f "$dir/saved.docx" ] && cp "$dir/saved.docx" "$dir_out/$name.saved.docx"
  else
    printf '{"file":"%s.docx","opened":false,"error":"written.ps1 failed: %s"}\n' "$name" "$(tr -d '\r\n"' < "$dir/stderr" | cut -c1-200)" > "$dir_out/$name.word.json"
  fi
  cat "$dir/stderr" >&2
  rm -rf "$dir"
  echo "$dir_out/$name.word.json"
done
