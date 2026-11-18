#!/usr/bin/env bash
# Writes the table of built-in style names for a display language of the
# Word on the Windows host, from WSL.
#
#   tools/word/style-names.sh LANG
#
# LANG is the language tag of the display language Word runs in, such as
# en; the script checks it against Word's. The table goes to crates/nebu-docx/src/view/names/LANG.tsv. The script
# works in a fresh directory under the Windows %TEMP% and removes it after.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
PS=/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe
lang=$1
wintemp=$("$PS" -NoProfile -Command '[IO.Path]::GetTempPath()' | tr -d '\r')
dir="$(wslpath -u "$wintemp")tabellion-names-$(date +%s%N)"
mkdir -p "$dir"
trap 'rm -rf "$dir"' EXIT
# Windows PowerShell reads a script without a byte order mark as ANSI.
for f in Word.psm1 style-names.ps1; do
  { printf '\xEF\xBB\xBF'; cat "$here/$f"; } > "$dir/$f"
done
args=(-Out "$(wslpath -w "$dir/names.tsv")" -Dir "$(wslpath -w "$dir")")
timeout "${TABELLION_WORD_TIMEOUT:-600}" "$PS" -NoProfile -ExecutionPolicy Bypass \
  -File "$(wslpath -w "$dir/style-names.ps1")" "${args[@]}" | tr -d '\r'
shown=$(sed -n '1s/.*display language \([A-Za-z-]*\),.*/\1/p' "$dir/names.tsv")
if [ "${shown%%-*}" != "$lang" ]; then
  echo "style-names.sh: Word runs in $shown, not $lang" >&2
  exit 1
fi
cp "$dir/names.tsv" "$here/../../crates/nebu-docx/src/view/names/$lang.tsv"
