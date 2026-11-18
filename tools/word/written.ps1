# Opens documents that tabellion wrote in a Word instance of its own and
# reports, as JSON, what Word makes of them; then saves each as a new file.
#
#   powershell.exe -NoProfile -ExecutionPolicy Bypass -File written.ps1 -Out report.json -PidFile word.pid -Saved saved.docx file.docx
#
# The report holds: whether Word opened the file without repair and in its
# current mode (no conversion), the paragraphs of the body with their
# style, text and list number as Word shows it (ListFormat.ListString),
# the tables with their style, header rows and the style options for
# header row, banded rows and first column, the table styles of the
# document, the captions Word lists for cross-references
# (GetCrossReferenceItems) for the labels Table, Figure and Equation, every
# field's code and result before and after Word updates all fields of all
# stories, each REF field with the text of the bookmark it names, the
# footnotes, and the inline pictures with their sizes in points.
#
# Safety: the script starts its Word through Word.psm1, which learns the
# process id of that instance from the instance itself and refuses one that
# was already running. It sets the user name and initials to the invented
# Wren Ashby for the file it saves, and Word.psm1 puts them back however the
# script ends. It opens only the file it is given and saves only -Saved.
param(
  [Parameter(Mandatory)][string]$Out,
  [string]$PidFile,
  [Parameter(Mandatory)][string]$Saved,
  [Parameter(Mandatory)][string]$File
)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'Word.psm1') -Force

function Text-Of($range) {
  $range.TextRetrievalMode.IncludeFieldCodes = $false
  $range.TextRetrievalMode.IncludeHiddenText = $false
  $range.Text
}

function Fields-Of($doc) {
  $list = @()
  foreach ($s in $doc.StoryRanges) {
    $r = $s
    $n = 0
    while ($r -ne $null -and $n -lt 200) {
      foreach ($f in $r.Fields) { $list += [ordered]@{ story = [int]$r.StoryType; code = $f.Code.Text.Trim(); result = $f.Result.Text } }
      $r = $r.NextStoryRange
      $n++
    }
  }
  ,$list
}

function Set-Property($doc, $name, $value) {
  $props = $doc.BuiltInDocumentProperties
  $p = [System.__ComObject].InvokeMember('Item', 'GetProperty', $null, $props, @($name))
  [void][System.__ComObject].InvokeMember('Value', 'SetProperty', $null, $p, @($value))
}

$session = Start-OwnWord -PidFile $PidFile
$word = $session.Word
$r = [ordered]@{ file = [IO.Path]::GetFileName($File); opened = $false; error = $null }
try {
  $word.Visible = $false
  $word.DisplayAlerts = 0
  Set-WordOption $session 'Options.UseLocalUserInfo' $true
  Set-WordOption $session 'UserName' 'Wren Ashby'
  Set-WordOption $session 'UserInitials' 'WA'
  $r.word_version = $word.Version + ' build ' + $word.Build
  $path = (Resolve-Path $File).Path
  # FileName, ConfirmConversions, ReadOnly, AddToRecentFiles.
  $doc = $word.Documents.Open($path, $false, $false, $false)
  try {
    $r.opened = $true
    $r.compatibility_mode = $doc.CompatibilityMode
    $r.converted = ($doc.CompatibilityMode -ne 15)
    $r.paragraphs = @()
    foreach ($p in $doc.Paragraphs) {
      $st = $null
      try { $st = $p.Style.NameLocal } catch {}
      $r.paragraphs += [ordered]@{ style = $st; text = (Text-Of $p.Range); list = $p.Range.ListFormat.ListString; in_table = [bool]$p.Range.Information(12) }
    }
    $r.table_count = $doc.Tables.Count
    $r.tables = @()
    foreach ($t in $doc.Tables) {
      # Rows of a table with vertically merged cells cannot be reached one
      # by one, and a cell's Row is then empty; a cell selected gives the
      # format of its row through the selection.
      $heads = 0
      $seen = @{}
      foreach ($c in $t.Range.Cells) {
        if ($c.NestingLevel -ne $t.NestingLevel -or $seen.ContainsKey($c.RowIndex)) { continue }
        $seen[$c.RowIndex] = $true
        $c.Select()
        if ($word.Selection.Rows.HeadingFormat -eq -1) { $heads++ }
      }
      $st = $null
      try { $st = $t.Style.NameLocal } catch {}
      $r.tables += [ordered]@{
        style = $st; rows = $t.Rows.Count; columns = $t.Columns.Count; header_rows = $heads
        heading_rows = [bool]$t.ApplyStyleHeadingRows; row_bands = [bool]$t.ApplyStyleRowBands; first_column = [bool]$t.ApplyStyleFirstColumn
        nested = $t.Tables.Count
      }
    }
    $r.table_styles = @()
    foreach ($s in $doc.Styles) {
      if ($s.Type -eq 3 -and $s.InUse) { $r.table_styles += [ordered]@{ name = $s.NameLocal; builtin = $s.BuiltIn } }
    }
    $r.captions = [ordered]@{}
    foreach ($label in @('Table', 'Figure', 'Equation')) {
      $items = @()
      try { foreach ($i in $doc.GetCrossReferenceItems($label)) { $items += $i } } catch {}
      $r.captions[$label] = $items
    }
    $doc.Bookmarks.ShowHidden = $true
    $r.bookmarks = @()
    foreach ($b in $doc.Bookmarks) { $r.bookmarks += $b.Name }
    $r.fields_before = Fields-Of $doc
    foreach ($s in $doc.StoryRanges) {
      $x = $s
      $n = 0
      while ($x -ne $null -and $n -lt 200) { [void]$x.Fields.Update(); $x = $x.NextStoryRange; $n++ }
    }
    $r.fields_after = Fields-Of $doc
    $r.references = @()
    foreach ($f in $doc.Content.Fields) {
      $code = $f.Code.Text.Trim()
      if ($code -match '^REF\s+(\S+)') {
        $name = $Matches[1]
        $target = $null
        if ($doc.Bookmarks.Exists($name)) { $target = Text-Of $doc.Bookmarks.Item($name).Range }
        $r.references += [ordered]@{ code = $code; result = $f.Result.Text; target = $target }
      }
    }
    $r.footnotes = @()
    foreach ($fn in $doc.Footnotes) { $r.footnotes += [ordered]@{ index = $fn.Index; text = (Text-Of $fn.Range) } }
    $r.inline_shapes = @()
    foreach ($s in $doc.InlineShapes) { $r.inline_shapes += [ordered]@{ type = [int]$s.Type; width = $s.Width; height = $s.Height; alt = $s.AlternativeText } }
    Set-Property $doc 'Company' ''
    Set-Property $doc 'Manager' ''
    Set-Property $doc 'Author' 'Wren Ashby'
    [object]$sp = [string]$Saved
    [object]$fmt = 16
    [object]$no = $false
    [object]$none = ''
    $doc.SaveAs2([ref]$sp, [ref]$fmt, [ref]$no, [ref]$none, [ref]$no)
  } finally {
    $doc.Close([ref]0)
  }
} catch {
  $r.error = $_.Exception.Message
} finally {
  Stop-OwnWord $session
}
$r | ConvertTo-Json -Depth 30 | Out-File -Encoding utf8 $Out
