#!/usr/bin/env bash
# Runs check.ps1 from WSL against a Word installed on the Windows host, one
# Word instance per file.
#
#   tools/word/check.sh OUTDIR file.docx...
#
# Each file is copied into a fresh directory under the Windows %TEMP% and
# checked in a Word that check.ps1 starts for it. A run that takes longer
# than TABELLION_WORD_TIMEOUT seconds (default 180) almost always waits on a
# repair dialog: the script then stops that Word, by the process id check.ps1
# recorded, and reports the file as failed. OUTDIR receives one JSON report
# per file and the files Word saved (updated, accepted, rejected, PDF).
# Set TABELLION_WORD_READ=1 for a full reading of each file (see check.ps1).
set -uo pipefail
PS=/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe
limit=${TABELLION_WORD_TIMEOUT:-180}
out=$1; shift
mkdir -p "$out"
wintemp=$("$PS" -NoProfile -Command '[IO.Path]::GetTempPath()' | tr -d '\r')
for f in "$@"; do
  stem=$(basename "$f" .docx)
  id="tabellion-check-$(date +%s%N)"
  dir="$(wslpath -u "$wintemp")$id"
  mkdir -p "$dir"
  cp "$(dirname "$0")/check.ps1" "$dir/"
  cp "$f" "$dir/"
  w() { wslpath -w "$dir/$1"; }
  timeout "$limit" "$PS" -NoProfile -ExecutionPolicy Bypass -File "$(w check.ps1)" \
    -Out "$(w report.json)" -PidFile "$(w word.pid)" ${TABELLION_WORD_PDF:+-Pdf} ${TABELLION_WORD_READ:+-Read} "$(w "$(basename "$f")")" >/dev/null 2>"$out/$stem.stderr"
  status=$?
  if [ $status -eq 124 ] && [ -f "$dir/word.pid" ]; then
    pid=$(tr -dc 0-9 < "$dir/word.pid")
    title=$("$PS" -NoProfile -Command "(Get-Process -Id $pid -ErrorAction SilentlyContinue).MainWindowTitle" | tr -d '\r')
    "$PS" -NoProfile -Command "Stop-Process -Id $pid -Force" >/dev/null 2>&1
    sleep 2
    printf '{"files":[{"file":"%s","opened":false,"error":"timeout after %ss; Word window: %s"}]}\n' "$f" "$limit" "$title" > "$out/$stem.json"
  elif [ -f "$dir/report.json" ]; then
    sed '1s/^\xEF\xBB\xBF//' "$dir/report.json" > "$out/$stem.json"
  else
    printf '{"files":[{"file":"%s","opened":false,"error":"check.ps1 failed, see %s.stderr"}]}\n' "$f" "$stem" > "$out/$stem.json"
  fi
  [ -f "$dir/report.json.log" ] && tr -d "\r" < "$dir/report.json.log" > "$out/$stem.log"
  [ -d "$dir/word-output" ] && cp -r "$dir/word-output/." "$out/"
  rm -rf "$dir" 2>/dev/null
  echo "$out/$stem.json"
done
