# Opens a copy in Word, updates every field, saves it and exports the PDF.
param([Parameter(Mandatory)][string]$Dir)
$ErrorActionPreference = 'Stop'
$word = New-Object -ComObject Word.Application
try {
  $word.Visible = $false
  $word.DisplayAlerts = 0
  $word.Options.CheckGrammarAsYouType = $false
  $word.Options.CheckSpellingAsYouType = $false
  $word.ScreenUpdating = $false
  # Background pagination re-runs after every field update and dominates runtime.
  $word.Options.Pagination = $false
  $doc = $word.Documents.Open("$Dir\in.docx", $false, $false, $false)
  # TOC switches name "Ueberschrift 1" literally; an English Word UI knows that
  # style as "Heading 1" and silently drops the entry. Swap for the update only.
  $de = [string][char]0xDC + 'berschrift 1'
  $local = $doc.Styles.Item(-2).NameLocal
  $tocs = @($doc.Fields | Where-Object { $_.Type -eq 13 -and $_.Code.Text.Contains($de) })
  if ($local -ne $de) { foreach ($f in $tocs) { $f.Code.Text = $f.Code.Text.Replace($de, $local) } }
  # Two passes: the first changes length, the second gets final page numbers.
  foreach ($pass in 1..2) {
    # Not Fields.Update(): that also touches HYPERLINK fields, and at least one
    # G-BA template has a link Word then replaces with "Error! Hyperlink ...".
    # Types: REF=3, STYLEREF=10, SEQ=12, NOTEREF=72. PAGEREF sits inside the TOCs.
    foreach ($f in $doc.Fields) {
      if (@(3, 10, 12, 72) -contains [int]$f.Type) { [void]$f.Update() }
    }
    foreach ($s in $doc.Sections) {
      foreach ($h in $s.Headers) { [void]$h.Range.Fields.Update() }
      foreach ($f in $s.Footers) { [void]$f.Range.Fields.Update() }
    }
    $doc.Repaginate()
    foreach ($t in $doc.TablesOfContents) { $t.Update() }
    foreach ($t in $doc.TablesOfFigures) { $t.Update() }
  }
  if ($local -ne $de) { foreach ($f in $tocs) { $f.Code.Text = $f.Code.Text.Replace($local, $de) } }
  $doc.SaveAs2([ref]"$Dir\out.docx", [ref]16)
  # wdExportFormatPDF=17, print quality=0, whole doc=0, content=0, bookmarks from headings=1
  $doc.ExportAsFixedFormat("$Dir\out.pdf", 17, $false, 0, 0, 1, 1, 0, $true, $true, 1, $true, $true, $false)
  $doc.Close([ref]0)
} finally {
  $word.Quit([ref]0)
  [void][Runtime.InteropServices.Marshal]::ReleaseComObject($word)
}
