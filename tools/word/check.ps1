# Opens .docx files in a Word instance of its own and reports, as JSON, what
# Word makes of them: whether they open without repair, the fields before and
# after an update, revisions, tables and their styles, and the text after
# accepting and after rejecting all revisions.
#
#   powershell.exe -NoProfile -ExecutionPolicy Bypass -File check.ps1 -Out report.json -PidFile word.pid a.docx
#
# With -Read, each file's report also holds a reading: the text of every
# story as stored, after accepting and after rejecting all revisions, the
# headers and footers of each section, tables as rows of cells, fields,
# revisions, comments, bookmarks, notes, text boxes, pictures, content
# controls and the names of the styles in use. The test corpus keeps these
# readings next to its documents.
#
# A file that needs repair makes Word show a modal dialog even with alerts
# off, and the script then hangs. The process id goes to -PidFile first, so a
# caller with a timeout (check.sh) can stop exactly this Word and no other.
#
# Safety: the script starts its Word through Word.psm1, which learns the
# process id of that instance from the instance itself and refuses one that
# was already running. It changes no option of Word. It only opens the
# files it is given, saves only into a directory next to them, and quits
# only its own instance.
param(
  [Parameter(Mandatory)][string]$Out,
  [string]$PidFile,
  [switch]$Pdf,
  [switch]$Read,
  [Parameter(Mandatory, ValueFromRemainingArguments)][string[]]$Files
)
$ErrorActionPreference = 'Stop'

function Step($m) { Add-Content -Path "$Out.log" -Value ("{0:HH:mm:ss.fff} {1}" -f (Get-Date), $m) }

Import-Module (Join-Path $PSScriptRoot 'Word.psm1') -Force
$session = Start-OwnWord -PidFile $PidFile
$word = $session.Word

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

function Text-Of($range) {
  $range.TextRetrievalMode.IncludeFieldCodes = $false
  $range.TextRetrievalMode.IncludeHiddenText = $false
  $range.Text
}

# Every story by its WdStoryType, with linked stories (headers of later
# sections, further text boxes) in order.
function Story-Texts($doc) {
  $out = [ordered]@{}
  foreach ($s in $doc.StoryRanges) {
    $list = @()
    $r = $s
    $n = 0
    while ($r -ne $null -and $n -lt 200) {
      $list += (Text-Of $r)
      $r = $r.NextStoryRange
      $n++
    }
    $out["$([int]$s.StoryType)"] = $list
  }
  $out
}

function Table-Reading($t, $depth) {
  $cells = @()
  foreach ($c in $t.Range.Cells) {
    if ($c.NestingLevel -ne $t.NestingLevel) { continue }
    $nested = @()
    if ($depth -lt 8) { foreach ($n in $c.Tables) { $nested += ,(Table-Reading $n ($depth + 1)) } }
    $cells += [ordered]@{ row = $c.RowIndex; column = $c.ColumnIndex; start = $c.Range.Start; end = $c.Range.End; text = (Text-Of $c.Range); tables = $nested }
  }
  [ordered]@{ rows = $t.Rows.Count; columns = $t.Columns.Count; nesting = $t.NestingLevel; start = $t.Range.Start; end = $t.Range.End; cells = $cells }
}

# The paragraphs of the main story: the text a reader sees, where it
# starts, whether it is in a table, and the toggles of its text without
# the mark (-1 on, 0 off, 9999999 mixed).
function Paragraph-Reading($doc) {
  $list = @()
  foreach ($p in $doc.Paragraphs) {
    $r = $p.Range
    $inTable = [bool]$r.Information(12)
    $body = $doc.Range($r.Start, [Math]::Max($r.Start, $r.End - 1))
    $list += [ordered]@{
      start = $r.Start; end = $r.End; text = (Text-Of $r); table = $inTable
      hidden = [int]$body.Font.Hidden; bold = [int]$body.Font.Bold; italic = [int]$body.Font.Italic
    }
  }
  ,$list
}

# Each section's headers and footers by kind, with text and fields.
function Section-Reading($doc) {
  $list = @()
  foreach ($sec in $doc.Sections) {
    $hf = [ordered]@{ index = $sec.Index; headers = @(); footers = @() }
    foreach ($k in 1, 2, 3) {
      $h = $sec.Headers.Item($k)
      $hf.headers += [ordered]@{ kind = $k; exists = $h.Exists; linked = $h.LinkToPrevious; text = (Text-Of $h.Range); fields = (Field-Info $h.Range) }
      $f = $sec.Footers.Item($k)
      $hf.footers += [ordered]@{ kind = $k; exists = $f.Exists; linked = $f.LinkToPrevious; text = (Text-Of $f.Range); fields = (Field-Info $f.Range) }
    }
    $list += $hf
  }
  ,$list
}

function Note-Reading($notes) {
  $list = @()
  foreach ($n in $notes) { $list += [ordered]@{ index = $n.Index; text = (Text-Of $n.Range) } }
  ,$list
}

function Reading($doc) {
  $doc.Bookmarks.ShowHidden = $true
  # All markup, so that the text holds deleted text as well.
  $view = $doc.ActiveWindow.View
  $view.RevisionsFilter.Markup = 2
  $view.RevisionsFilter.View = 0
  $view.ShowRevisionsAndComments = $true
  $x = [ordered]@{}
  $x.stories = Story-Texts $doc
  $x.paragraphs = Paragraph-Reading $doc
  $x.sections = Section-Reading $doc
  $x.tables = @()
  foreach ($t in $doc.Tables) { $x.tables += ,(Table-Reading $t 0) }
  $x.fields = @()
  $x.revisions = @()
  foreach ($s in $doc.StoryRanges) {
    $r = $s
    $n = 0
    while ($r -ne $null -and $n -lt 200) {
      foreach ($f in $r.Fields) {
        $x.fields += [ordered]@{ story = [int]$r.StoryType; type = [int]$f.Type; code = $f.Code.Text; result = $f.Result.Text }
      }
      foreach ($rv in $r.Revisions) {
        $x.revisions += [ordered]@{ story = [int]$r.StoryType; type = [int]$rv.Type; author = $rv.Author; text = $rv.Range.Text; format = $rv.FormatDescription }
      }
      $r = $r.NextStoryRange
      $n++
    }
  }
  $x.comments = @()
  foreach ($c in $doc.Comments) {
    $parent = $null
    try { if ($c.Ancestor) { $parent = $c.Ancestor.Index } } catch {}
    $x.comments += [ordered]@{ index = $c.Index; author = $c.Author; initials = $c.Initial; text = (Text-Of $c.Range); scope = (Text-Of $c.Scope); parent = $parent; done = $c.Done }
  }
  $x.bookmarks = @()
  foreach ($b in $doc.Bookmarks) { $x.bookmarks += [ordered]@{ name = $b.Name; story = [int]$b.StoryType; text = (Text-Of $b.Range) } }
  $x.footnotes = Note-Reading $doc.Footnotes
  $x.endnotes = Note-Reading $doc.Endnotes
  $x.inline_shapes = @()
  foreach ($s in $doc.InlineShapes) { $x.inline_shapes += [ordered]@{ type = [int]$s.Type; alt = $s.AlternativeText; title = $s.Title; width = $s.Width; height = $s.Height } }
  $x.shapes = @()
  foreach ($s in $doc.Shapes) {
    $text = $null
    try { if ($s.TextFrame.HasText) { $text = (Text-Of $s.TextFrame.TextRange) } } catch {}
    $x.shapes += [ordered]@{ type = [int]$s.Type; alt = $s.AlternativeText; title = $s.Title; width = $s.Width; height = $s.Height; text = $text }
  }
  $x.content_controls = @()
  foreach ($cc in $doc.ContentControls) {
    $x.content_controls += [ordered]@{ type = [int]$cc.Type; tag = $cc.Tag; title = $cc.Title; text = (Text-Of $cc.Range); placeholder = $cc.ShowingPlaceholderText }
  }
  $x.styles = @()
  foreach ($s in $doc.Styles) {
    if ($s.InUse) { $x.styles += [ordered]@{ name = $s.NameLocal; builtin = $s.BuiltIn; type = [int]$s.Type } }
  }
  $x
}

$report = [ordered]@{ word_version = $null; word_pid = $session.Pid; files = @() }
try {
  $word.Visible = $false
  $word.DisplayAlerts = 0
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
      if ($Read) {
        Step 'reading'
        $r.reading = Reading $doc
        $doc.AcceptAllRevisions()
        $r.reading.accepted = Story-Texts $doc
        $r.reading.accepted_paragraphs = Paragraph-Reading $doc
        $r.reading.accepted_sections = Section-Reading $doc
        $r.reading.accepted_footnotes = Note-Reading $doc.Footnotes
        $r.reading.accepted_endnotes = Note-Reading $doc.Endnotes
        $r.reading.accepted_tables = @()
        foreach ($t in $doc.Tables) { $r.reading.accepted_tables += ,(Table-Reading $t 0) }
        $doc.Close([ref]0)
        $doc = $word.Documents.Open($path, $false, $false, $false)
        $doc.RejectAllRevisions()
        $r.reading.rejected = Story-Texts $doc
        $r.reading.rejected_paragraphs = Paragraph-Reading $doc
        $r.reading.rejected_tables = @()
        foreach ($t in $doc.Tables) { $r.reading.rejected_tables += ,(Table-Reading $t 0) }
        $doc.Close([ref]0)
        $doc = $word.Documents.Open($path, $false, $false, $false)
        Step 'read'
      }
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
    Test-ForcedFailure 'check.ps1, after the first file'
  }
} finally {
  Stop-OwnWord $session
}
Step 'writing report'
$report | ConvertTo-Json -Depth 30 | Out-File -Encoding utf8 $Out
