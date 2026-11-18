#!/usr/bin/env bash
# Writes crates/nebu-docx/src/writer/builtin.xml: the built-in styles the
# writer adds to a document that lacks them, as the Word on the Windows host
# writes them, from WSL.
#
#   tools/word/builtin-styles.sh
#
# The script works in a fresh directory under the Windows %TEMP% and removes
# it after.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
PS=/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe
wintemp=$("$PS" -NoProfile -Command '[IO.Path]::GetTempPath()' | tr -d '\r')
dir="$(wslpath -u "$wintemp")tabellion-builtin-$(date +%s%N)"
mkdir -p "$dir"
trap 'rm -rf "$dir"' EXIT
# Windows PowerShell reads a script without a byte order mark as ANSI.
for f in Word.psm1 builtin-styles.ps1; do
  { printf '\xEF\xBB\xBF'; cat "$here/$f"; } > "$dir/$f"
done
timeout "${TABELLION_WORD_TIMEOUT:-600}" "$PS" -NoProfile -ExecutionPolicy Bypass \
  -File "$(wslpath -w "$dir/builtin-styles.ps1")" -Dir "$(wslpath -w "$dir")" | tr -d '\r'
python3 -I "$here/builtin-styles.py" "$dir/builtin-styles.docx" "$here/../../crates/nebu-docx/src/writer/builtin.xml"
