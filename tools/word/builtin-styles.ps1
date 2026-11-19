# Has Word write the built-in styles that tabellion adds to a document that
# lacks them: one paragraph or run in each, a table in Table Grid, saved
# into DIR as builtin-styles.docx. builtin-styles.py then copies the
# definitions out of its styles part.
#
#   powershell.exe -NoProfile -ExecutionPolicy Bypass -File builtin-styles.ps1 -Dir DIR
#
# It changes no setting of Word: the document's author is set to the
# invented Wren Ashby on the document itself.
param([Parameter(Mandatory)][string]$Dir)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'Word.psm1') -Force

function Para($doc, $text, $style) {
  $end = $doc.Content.End - 1
  $r = $doc.Range($end, $end)
  $r.InsertAfter($text)
  $r.Style = $doc.Styles.Item($style)
  $r.InsertParagraphAfter()
  $doc.Range($end, $end + $text.Length)
}

$session = Start-OwnWord -PidFile (Join-Path $Dir 'word.pid')
try {
  $word = $session.Word
  $word.Visible = $false
  $word.DisplayAlerts = 0
  $doc = $word.Documents.Add()
  # By the names Word stores, which Styles.Item takes in any display
  # language for these.
  foreach ($s in @('heading 1', 'heading 2', 'heading 3', 'heading 4', 'heading 5', 'heading 6', 'heading 7',
                   'heading 8', 'heading 9', 'caption', 'List Paragraph', 'footnote text')) {
    Para $doc "Style $s" $s | Out-Null
  }
  $r = Para $doc 'Code: x' (-1)
  $doc.Range($r.End - 1, $r.End).Style = $doc.Styles.Item('HTML Code')
  $r = Para $doc 'A link' (-1)
  $doc.Hyperlinks.Add($doc.Range($r.Start, $r.End), 'https://example.org/birds') | Out-Null
  $r = Para $doc 'A note' (-1)
  $doc.Footnotes.Add($doc.Range($r.End, $r.End)) | Out-Null
  $end = $doc.Content.End - 1
  $t = $doc.Tables.Add($doc.Range($end, $end), 1, 1)
  $t.Style = 'Table Grid'
  $props = $doc.BuiltInDocumentProperties
  $p = [System.__ComObject].InvokeMember('Item', 'GetProperty', $null, $props, @('Author'))
  [void][System.__ComObject].InvokeMember('Value', 'SetProperty', $null, $p, @('Wren Ashby'))
  [object]$path = [string](Join-Path $Dir 'builtin-styles.docx')
  [object]$format = 16
  $doc.SaveAs2([ref]$path, [ref]$format)
  $doc.Close([ref]0)
  "Word $($word.Version)"
} finally {
  Stop-OwnWord $session
}
