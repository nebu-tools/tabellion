# Opens .docx files in a Word instance of its own and reports, as JSON, what
# Word makes of them: whether they open without repair, the fields before and
# after an update, revisions, tables and their styles, and the text after
# accepting and after rejecting all revisions.
#
#   powershell.exe -NoProfile -ExecutionPolicy Bypass -File check.ps1 -Out report.json -PidFile word.pid a.docx
#
# A file that needs repair makes Word show a modal dialog even with alerts
# off, and the script then hangs. The process id goes to -PidFile first, so a
# caller with a timeout (check.sh) can stop exactly this Word and no other.
#
# Safety: the script starts a new, invisible Word.Application and checks by
# process id that it is new. If COM hands back a Word that was already
# running, it stops without touching it. It only opens the files it is given,
# saves only into a directory next to them, and quits only its own instance.
param(
  [Parameter(Mandatory)][string]$Out,
  [string]$PidFile,
  [switch]$Pdf,
  [Parameter(Mandatory, ValueFromRemainingArguments)][string[]]$Files
)
$ErrorActionPreference = 'Stop'

function Step($m) { Add-Content -Path "$Out.log" -Value ("{0:HH:mm:ss.fff} {1}" -f (Get-Date), $m) }

function Get-WordIds { @(Get-Process WINWORD -ErrorAction SilentlyContinue | ForEach-Object { $_.Id }) }

$before = Get-WordIds
$word = New-Object -ComObject Word.Application
Start-Sleep -Milliseconds 500
$mine = @(Get-WordIds | Where-Object { $before -notcontains $_ })
if ($mine.Count -ne 1) {
  [void][Runtime.InteropServices.Marshal]::ReleaseComObject($word)
  throw "Could not identify a Word process of our own (new: $($mine -join ',')); not touching Word."
}
$wordPid = $mine[0]
if ($PidFile) { Set-Content -Path $PidFile -Value $wordPid }

function Field-Info($range) {
  $list = @()
  foreach ($f in $range.Fields) {
    $list += [ordered]@{ type = [int]$f.Type; code = $f.Code.Text.Trim(); result = $f.Result.Text }
  }
  ,$list
}

function Story-Text($doc) {
  $doc.Content.Text -replace "`r", "`n"
}

$report = [ordered]@{ word_version = $null; word_pid = $wordPid; files = @() }
try {
  $word.Visible = $false
  $word.DisplayAlerts = 0
  $word.Options.CheckGrammarAsYouType = $false
  $word.Options.CheckSpellingAsYouType = $false
  $word.Options.SaveNormalPrompt = $false
  $report.word_version = $word.Version + ' build ' + $word.Build
  foreach ($file in $Files) {
    $path = (Resolve-Path $file).Path
    $r = [ordered]@{ file = $path; opened = $false; error = $null }
    $dir = Join-Path (Split-Path $path) 'word-output'
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $stem = [IO.Path]::GetFileNameWithoutExtension($path)
    $doc = $null
    try {
      $sw = [Diagnostics.Stopwatch]::StartNew()
      # FileName, ConfirmConversions, ReadOnly, AddToRecentFiles. More
      # positional arguments (Encoding in particular) can make Word wait on a
      # conversion dialog.
      $doc = $word.Documents.Open($path, $false, $false, $false)
      $r.open_ms = $sw.ElapsedMilliseconds
      Step 'opened'
      $r.opened = $true
      $r.name = $doc.Name
      $r.pages = $doc.ComputeStatistics(2)
      $r.paragraphs = $doc.Paragraphs.Count
      $r.tables = @()
      $i = 0
      foreach ($t in $doc.Tables) {
        $i++
        if ($i -gt 20) { break }
        $st = $t.Style
        $r.tables += [ordered]@{ rows = $t.Rows.Count; columns = $t.Columns.Count; uniform = $t.Uniform; style = $(if ($st) { $st.NameLocal } else { $null }) }
      }
      $r.table_count = $doc.Tables.Count
      $r.table_styles = @()
      foreach ($s in $doc.Styles) {
        # Built-in ones too when in use, to compare Visibility with a style
        # the gallery shows (Table Grid) and one it hides (Normal Table).
        if ($s.Type -eq 3 -and (-not $s.BuiltIn -or $s.InUse)) {
          $r.table_styles += [ordered]@{ name = $s.NameLocal; builtin = $s.BuiltIn; in_use = $s.InUse; visibility = $s.Visibility; priority = $s.Priority; quick = $s.QuickStyle }
        }
      }
      $r.revisions = @()
      foreach ($rv in $doc.Revisions) {
        $r.revisions += [ordered]@{ type = [int]$rv.Type; author = $rv.Author; text = $rv.Range.Text }
      }
      $r.fields_before = Field-Info $doc.Content
      Step 'fields before'
      $sw.Restart()
      [void]$doc.Fields.Update()
      foreach ($s in $doc.StoryRanges) { [void]$s.Fields.Update() }
      $r.update_ms = $sw.ElapsedMilliseconds
      Step 'updated'
      $r.fields_after = Field-Info $doc.Content
      $r.footnotes = @()
      foreach ($fn in $doc.Footnotes) { $r.footnotes += [ordered]@{ index = $fn.Index; text = $fn.Range.Text } }
      Step 'texts'
      $r.text_head = (Story-Text $doc).Substring(0, [Math]::Min(1500, $doc.Content.Text.Length))
      $doc.SaveAs2([ref](Join-Path $dir "$stem.updated.docx"), [ref]16)
      Step 'saved updated'
      if ($doc.Revisions.Count -gt 0) {
        $doc.Revisions.AcceptAll()
        $r.accepted_text = (Story-Text $doc).Substring(0, [Math]::Min(1500, $doc.Content.Text.Length))
        $doc.SaveAs2([ref](Join-Path $dir "$stem.accepted.docx"), [ref]16)
        $doc.Close([ref]0)
        $doc = $word.Documents.Open($path, $false, $false, $false)
        $doc.Revisions.RejectAll()
        $r.rejected_text = (Story-Text $doc).Substring(0, [Math]::Min(1500, $doc.Content.Text.Length))
        $doc.SaveAs2([ref](Join-Path $dir "$stem.rejected.docx"), [ref]16)
      }
      if ($Pdf) { Step 'pdf'; $doc.SaveAs2([ref](Join-Path $dir "$stem.pdf"), [ref]17) }
    } catch {
      $r.error = $_.Exception.Message
    } finally {
      if ($doc) { try { $doc.Close([ref]0) } catch {} }
    }
    $report.files += $r
  }
} finally {
  try { $word.Quit([ref]0) } catch {}
  [void][Runtime.InteropServices.Marshal]::ReleaseComObject($word)
  [GC]::Collect()
  foreach ($i in 1..20) {
    if (-not (Get-Process -Id $wordPid -ErrorAction SilentlyContinue)) { break }
    Start-Sleep -Milliseconds 500
  }
  if (Get-Process -Id $wordPid -ErrorAction SilentlyContinue) {
    Stop-Process -Id $wordPid -Force -ErrorAction SilentlyContinue
    $report.killed_own_word = $true
  }
}
Step 'writing report'
$report | ConvertTo-Json -Depth 6 | Out-File -Encoding utf8 $Out
