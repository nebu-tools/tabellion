# Builds the Word-made documents of the test corpus in a Word instance of
# its own.
#
#   powershell.exe -NoProfile -ExecutionPolicy Bypass -File word.ps1 -Dir DIR
#
# DIR holds the pictures (bird.png, heron.png) and receives the documents.
#
# Safety: the script starts a new, invisible Word.Application and checks by
# process id that it is new; if not, it stops without touching Word. It sets
# the user name and initials to invented values for the documents, and puts
# back every option it changed before it quits its own instance.
param([Parameter(Mandatory)][string]$Dir)
$ErrorActionPreference = 'Stop'

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
Set-Content -Path (Join-Path $Dir 'word.pid') -Value $wordPid

# Options that Word keeps in the registry. They are read now and put back in
# the finally block.
$saved = [ordered]@{
  UserName = $word.UserName
  UserInitials = $word.UserInitials
  UseLocalUserInfo = $word.Options.UseLocalUserInfo
  CheckSpellingAsYouType = $word.Options.CheckSpellingAsYouType
  CheckGrammarAsYouType = $word.Options.CheckGrammarAsYouType
}

$AUTHOR_A = 'Wren Ashby'
$AUTHOR_B = 'Pip Tanager'

function As-Author($name, $initials) {
  $word.UserName = $name
  $word.UserInitials = $initials
}

# Appends a paragraph with text and a built-in style; returns the range of
# its text.
function Para($doc, $text, $style) {
  $end = $doc.Content.End - 1
  $r = $doc.Range($end, $end)
  $r.InsertAfter($text)
  try { $r.Style = $doc.Styles.Item($style) } catch { throw "style $style`: $($_.Exception.Message)" }
  $r.InsertParagraphAfter()
  $doc.Range($end, $end + $text.Length)
}

# The range at the end of the paragraph that holds $r, before its mark.
function At-End($doc, $r) {
  $p = $r.Paragraphs.Item(1).Range
  $doc.Range($p.End - 1, $p.End - 1)
}

function Text-After($doc, $r, $text) {
  $r.InsertAfter($text)
  $doc.Range($r.End, $r.End)
}

function Field($doc, $r, $code) {
  $doc.Fields.Add($r, -1, $code, $false)
}

# Document properties are late bound, so PowerShell reaches them by name.
function Set-Property($doc, $name, $value) {
  $props = $doc.BuiltInDocumentProperties
  $p = [System.__ComObject].InvokeMember('Item', 'GetProperty', $null, $props, @($name))
  [void][System.__ComObject].InvokeMember('Value', 'SetProperty', $null, $p, @($value))
}

function Scrub($doc, $title) {
  Set-Property $doc 'Title' $title
  Set-Property $doc 'Company' ''
  Set-Property $doc 'Manager' ''
  Set-Property $doc 'Author' $AUTHOR_A
}

function Save($doc, $name) {
  [object]$path = [string](Join-Path $Dir $name)
  [object]$format = 16
  [object]$no = $false
  [object]$none = ''
  # FileName, FileFormat (16: .docx), LockComments, Password, AddToRecentFiles.
  $doc.SaveAs2([ref]$path, [ref]$format, [ref]$no, [ref]$none, [ref]$no)
  $doc.Close([ref]0)
}

# Built-in styles by WdBuiltinStyle, so the script works in any interface
# language.
$NORMAL = -1; $H1 = -2; $H2 = -3; $TITLE = -63; $CAPTION = -35
$LIST_BULLET = -49; $LIST_NUMBER = -50; $QUOTE = 'Quote'; $BODY = -67; $SUBTITLE = -75

function Birds {
  $doc = $word.Documents.Add()
  Para $doc 'Breeding bird survey of the Alder Marshes' $TITLE | Out-Null
  $toc = Para $doc 'Contents' $NORMAL
  $h = Para $doc 'Survey sites' $H1
  $intro = Para $doc 'The survey covered four sites along the river. ' $NORMAL
  $cap = Para $doc 'Table ' $CAPTION
  $r = Field $doc (At-End $doc $cap) 'SEQ Table \* ARABIC'
  $cap = $doc.Paragraphs.Last.Previous(1).Range
  $r = Text-After $doc (At-End $doc $cap) ': Sites, visits and counts'

  $tp = Para $doc 'x' $NORMAL
  $t = $doc.Tables.Add($tp, 5, 4)
  $t.Borders.Enable = $true
  $cells = @(
    @('Site', 'Counts', '', 'Notes'),
    @('', 'Visits', 'Pairs', ''),
    @('North', '12', '41', 'Reed beds'),
    @('', '3', '9', 'Second count'),
    @('East', '9', '33', 'Willow carr')
  )
  for ($i = 0; $i -lt 5; $i++) { for ($j = 0; $j -lt 4; $j++) { $t.Cell($i + 1, $j + 1).Range.Text = $cells[$i][$j] } }
  # A nested table in the notes of the east site.
  $inner = $doc.Tables.Add($t.Cell(5, 4).Range, 2, 2)
  $inner.Borders.Enable = $true
  $inner.Cell(1, 1).Range.Text = 'Wren'; $inner.Cell(1, 2).Range.Text = '4'
  $inner.Cell(2, 1).Range.Text = 'Reed warbler'; $inner.Cell(2, 2).Range.Text = '7'
  $t.Rows.Item(1).HeadingFormat = $true
  # Merges: across in the header, down in the first column.
  $t.Cell(3, 1).Merge($t.Cell(4, 1))
  $t.Cell(1, 1).Merge($t.Cell(2, 1))
  $t.Cell(1, 2).Merge($t.Cell(1, 3))

  $ref = Para $doc 'As ' $NORMAL
  $r = At-End $doc $ref
  $r.InsertCrossReference('Table', 3, 1, $true)
  $ref = $doc.Paragraphs.Last.Previous(1).Range
  Text-After $doc (At-End $doc $ref) ' shows, the north site held the most pairs.' | Out-Null

  $doc.Bookmarks.Add('site_north', (Para $doc 'The north site lies in reed beds below the weir.' $NORMAL)) | Out-Null

  Para $doc 'Species' $H1 | Out-Null
  $s = Para $doc 'Wrens sang at every site' $NORMAL
  $doc.Footnotes.Add((At-End $doc $s), [Type]::Missing, 'Counted from song between 05:00 and 08:00.') | Out-Null
  $s = $doc.Paragraphs.Last.Previous(1).Range
  $r = Text-After $doc (At-End $doc $s) ', and reed warblers at two'
  $doc.Endnotes.Add($r, [Type]::Missing, 'Reed warblers arrive in late April.') | Out-Null
  $s = $doc.Paragraphs.Last.Previous(1).Range
  Text-After $doc (At-End $doc $s) '.' | Out-Null

  $fig = Para $doc '' $NORMAL
  $pic = $doc.InlineShapes.AddPicture((Join-Path $Dir 'bird.png'), $false, $true, $fig)
  $pic.AlternativeText = 'A dark bird shape on a pale green ground'
  $pic.Title = 'Wren'
  $cap = Para $doc 'Figure ' $CAPTION
  Field $doc (At-End $doc $cap) 'SEQ Figure \* ARABIC' | Out-Null
  $cap = $doc.Paragraphs.Last.Previous(1).Range
  Text-After $doc (At-End $doc $cap) ': A wren at the north site' | Out-Null

  $float = Para $doc 'Grey herons fed on the east site at dawn and flew to the heronry at dusk.' $NORMAL
  $shape = $doc.Shapes.AddPicture((Join-Path $Dir 'heron.png'), $false, $true, 300, 20, 90, 60, $float)
  $shape.AlternativeText = 'A dark bird shape on a pale blue ground'
  $shape.Title = 'Heron'
  $shape.WrapFormat.Type = 0

  $boxp = Para $doc 'A field note stands beside this paragraph.' $NORMAL
  $box = $doc.Shapes.AddTextbox(1, 300, 10, 140, 50, $boxp)
  $box.TextFrame.TextRange.Text = 'Field note: the weir was in flood on 3 May.'

  # Fields: a page number, a nested IF, a code split over runs, a field in a
  # hyperlink.
  $fp = Para $doc 'This paragraph is on page ' $NORMAL
  $r = Field $doc (At-End $doc $fp) 'PAGE'
  $fp = $doc.Paragraphs.Last.Previous(1).Range
  $r = Text-After $doc (At-End $doc $fp) '. '
  $if = Field $doc $r 'IF  > 0 "The report has tables." "The report has no tables."'
  $code = $if.Code
  $inner = $doc.Range($code.Start + 4, $code.Start + 4)
  Field $doc $inner 'SEQ Table \c' | Out-Null
  $if.Update() | Out-Null

  $sp = Para $doc 'The north site is described in ' $NORMAL
  $f = Field $doc (At-End $doc $sp) 'REF site_north \h'
  $f.Code.Characters.Item(3).Font.Bold = $true
  $f.Code.Characters.Item(4).Font.Italic = $true
  $f.Update() | Out-Null

  $hp = Para $doc 'Data: ' $NORMAL
  $link = $doc.Hyperlinks.Add((At-End $doc $hp), 'https://example.org/alder-marshes', '', '', 'count sheet ')
  $hr = $link.Range
  $in = $doc.Range($hr.End - 1, $hr.End - 1)
  Field $doc $in 'SEQ Sheet \* ARABIC' | Out-Null

  # A second section with its own header and footer.
  $last = Para $doc 'End of the first section.' $NORMAL
  $br = At-End $doc $last
  $br.InsertBreak(2)
  Para $doc 'Methods' $H1 | Out-Null
  Para $doc 'Each site was visited at dawn by two observers.' $NORMAL | Out-Null

  $s1 = $doc.Sections.Item(1)
  $s1.PageSetup.DifferentFirstPageHeaderFooter = -1
  $s1.Headers.Item(1).Range.Text = 'Alder Marshes survey'
  $s1.Headers.Item(2).Range.Text = 'Breeding bird survey, first page'
  $s1.Footers.Item(1).Range.Text = 'Page '
  $at = $s1.Footers.Item(1).Range
  $at.MoveEnd(1, -1) | Out-Null
  $at.Collapse(0)
  $at.Fields.Add($at, -1, 'PAGE', $false) | Out-Null
  $s2 = $doc.Sections.Item(2)
  $s2.Headers.Item(1).LinkToPrevious = $false
  $s2.Headers.Item(1).Range.Text = 'Alder Marshes survey: methods'
  $s2.Footers.Item(1).LinkToPrevious = $false
  $s2.Footers.Item(1).Range.Text = 'Methods section'

  $doc.TablesOfContents.Add($toc, $true, 1, 3) | Out-Null
  $doc.Fields.Update() | Out-Null
  $doc.TablesOfContents.Item(1).Update()
  Scrub $doc 'Breeding bird survey'
  Save $doc 'birds.docx'
}

function Parts {
  $doc = $word.Documents.Add()
  Para $doc 'Parts catalogue: garden pumps' $TITLE | Out-Null
  Para $doc 'Pumps' $H1 | Out-Null
  $a = Para $doc 'The P-100 pump moves 40 litres a minute.' $NORMAL
  $b = Para $doc 'The P-200 pump moves 65 litres a minute.' $NORMAL
  $c = Para $doc 'Spare seals fit both pumps.' $NORMAL
  Para $doc 'Clamps are sold in pairs.' $NORMAL | Out-Null
  Para $doc 'Hoses come in two lengths.' $NORMAL | Out-Null
  $tp = Para $doc 'x' $NORMAL
  $t = $doc.Tables.Add($tp, 4, 3)
  $t.Borders.Enable = $true
  $rows = @(@('Part', 'Name', 'Price'), @('P-100', 'Small pump', '120.00'), @('P-200', 'Large pump', '180.00'), @('S-1', 'Seal kit', '9.50'))
  for ($i = 0; $i -lt 4; $i++) { for ($j = 0; $j -lt 3; $j++) { $t.Cell($i + 1, $j + 1).Range.Text = $rows[$i][$j] } }

  Para $doc 'Order form' $H1 | Out-Null
  $kinds = @(
    @(0, 'rich', 'Rich text', 'Notes for the order'),
    @(1, 'plain', 'Plain text', 'Order number'),
    @(6, 'date', 'Date', $null),
    @(3, 'combo', 'Combo box', $null),
    @(4, 'dropdown', 'Drop-down list', $null),
    @(8, 'check', 'Check box', $null),
    @(2, 'picture', 'Picture', $null),
    @(5, 'gallery', 'Building block gallery', $null)
  )
  foreach ($k in $kinds) {
    $p = Para $doc ($k[2] + ': ') $NORMAL
    $at = At-End $doc $p
    $cc = $doc.ContentControls.Add($k[0], $at)
    $cc.Tag = 'order:' + $k[1]
    $cc.Title = $k[2]
    if ($k[0] -eq 3 -or $k[0] -eq 4) {
      $cc.DropdownListEntries.Add('Pump', 'pump') | Out-Null
      $cc.DropdownListEntries.Add('Seal kit', 'seal') | Out-Null
      $cc.DropdownListEntries.Item(2).Select()
    }
    if ($k[0] -eq 6) { $cc.DateDisplayFormat = 'yyyy-MM-dd'; $cc.Range.Text = '2026-05-03' }
    if ($k[0] -eq 8) { $cc.Checked = $true }
    if ($k[3]) { $cc.Range.Text = $k[3] }
  }
  $g = Para $doc 'Delivery to the north shed.' $NORMAL
  $g2 = Para $doc 'Payment on delivery.' $NORMAL
  try {
    $grp = $doc.ContentControls.Add(7, $doc.Range($g.Start, $g2.End + 1))
    $grp.Tag = 'order:group'; $grp.Title = 'Group'
  } catch { [Console]::Error.WriteLine("group content control: $($_.Exception.Message)") }
  $rp = Para $doc 'Line item: one pump.' $NORMAL
  try {
    $rep = $doc.ContentControls.Add(9, $rp.Paragraphs.Item(1).Range)
    $rep.Tag = 'order:repeat'; $rep.Title = 'Repeating section'
  } catch { [Console]::Error.WriteLine("repeating section content control: $($_.Exception.Message)") }

  # Comments with replies, by two authors.
  As-Author $AUTHOR_A 'WA'
  $a = $doc.Paragraphs.Item(3).Range
  $cm = $doc.Comments.Add($doc.Range($a.Start + 4, $a.Start + 9), 'Is the P-100 still sold?')
  As-Author $AUTHOR_B 'PT'
  $cm.Replies.Add($doc.Range($a.Start + 4, $a.Start + 9), 'Yes, until the end of the year.') | Out-Null
  $c = $doc.Paragraphs.Item(5).Range
  $doc.Comments.Add($doc.Range($c.Start, $c.End - 1), 'Check the seal sizes.') | Out-Null

  # Tracked changes by two authors.
  $doc.TrackRevisions = $true
  $doc.TrackMoves = $true
  $doc.TrackFormatting = $true
  As-Author $AUTHOR_A 'WA'
  $a = $doc.Paragraphs.Item(3).Range
  $doc.Range($a.Start + 15, $a.Start + 15).InsertAfter('quietly ')
  $b = $doc.Paragraphs.Item(4).Range
  $doc.Range($b.Start + 10, $b.Start + 15).Delete() | Out-Null
  $a = $doc.Paragraphs.Item(3).Range
  $doc.Range($a.Start + 29, $a.Start + 38).Font.Bold = $true
  $doc.Paragraphs.Item(5).Alignment = 1
  As-Author $AUTHOR_B 'PT'
  # Moves the hoses paragraph above the clamps.
  $doc.Paragraphs.Item(7).Range.Relocate(0)
  $t = $doc.Tables.Item(1)
  $t.Rows.Item(4).Delete()
  $t.Rows.Add() | Out-Null
  $t.Cell(5, 1).Range.Text = 'H-1'; $t.Cell(5, 2).Range.Text = 'Hose'; $t.Cell(5, 3).Range.Text = '14.00'
  $p = $doc.Paragraphs.Item(2).Range
  $doc.Range($p.End, $p.End).InsertBefore("All prices include delivery.`r")
  $doc.TrackRevisions = $false
  As-Author $AUTHOR_A 'WA'

  Scrub $doc 'Parts catalogue'
  Save $doc 'parts.docx'
}

function Recipes($name, $lang, $heading, $dish, $intro, $items, $steps, $saying, $label, $legend, $note) {
  $doc = $word.Documents.Add()
  Para $doc $heading $TITLE | Out-Null
  Para $doc $dish $H1 | Out-Null
  $p = Para $doc $intro $NORMAL
  $doc.Footnotes.Add((At-End $doc $p), [Type]::Missing, $note) | Out-Null
  foreach ($i in $items) { Para $doc $i $LIST_BULLET | Out-Null }
  foreach ($s in $steps) { Para $doc $s $LIST_NUMBER | Out-Null }
  Para $doc $saying $QUOTE | Out-Null
  $cap = Para $doc ($label + ' ') $CAPTION
  Field $doc (At-End $doc $cap) ('SEQ ' + $label + ' \* ARABIC') | Out-Null
  $cap = $doc.Paragraphs.Last.Previous(1).Range
  Text-After $doc (At-End $doc $cap) (": " + $legend) | Out-Null
  $tp = Para $doc 'x' $NORMAL
  $t = $doc.Tables.Add($tp, 3, 2)
  $t.Style = 'Table Grid'
  $t.Cell(1, 1).Range.Text = $items[0]; $t.Cell(1, 2).Range.Text = '200 g'
  $t.Cell(2, 1).Range.Text = $items[1]; $t.Cell(2, 2).Range.Text = '3'
  $t.Cell(3, 1).Range.Text = $items[2]; $t.Cell(3, 2).Range.Text = '100 g'
  Para $doc $dish $H2 | Out-Null
  $doc.Content.LanguageID = $lang
  Scrub $doc $heading
  Save $doc $name
}

try {
  $word.Visible = $false
  $word.DisplayAlerts = 0
  $word.Options.CheckGrammarAsYouType = $false
  $word.Options.CheckSpellingAsYouType = $false
  $word.Options.SaveNormalPrompt = $false
  $word.Options.UseLocalUserInfo = $true
  As-Author $AUTHOR_A 'WA'
  Birds
  Parts
  Recipes 'recipes-de.docx' 1031 'Rezepte aus dem Obstgarten' 'Apfelkuchen' `
    'Ein einfacher Kuchen mit Äpfeln aus dem eigenen Garten.' `
    @('Mehl', 'Eier', 'Butter') @('Den Teig rühren.', 'Die Äpfel schneiden.', 'Eine Stunde backen.') `
    'Ein Kuchen ist erst fertig, wenn die Küche danach duftet.' 'Tabelle' 'Zutaten für einen Kuchen' `
    'Säuerliche Sorten eignen sich am besten.'
  Recipes 'recettes-fr.docx' 1036 'Recettes du verger' 'Tarte aux pommes' `
    'Une tarte simple avec des pommes du jardin.' `
    @('Farine', 'Œufs', 'Beurre') @('Préparer la pâte.', 'Couper les pommes.', 'Cuire une heure.') `
    'Une tarte est prête quand la cuisine sent bon.' 'Tableau' 'Ingrédients pour une tarte' `
    'Les variétés acidulées conviennent le mieux.'
} catch {
  [Console]::Error.WriteLine("word.ps1, line $($_.InvocationInfo.ScriptLineNumber): $($_.Exception.Message)")
  throw
} finally {
  foreach ($k in 'UseLocalUserInfo', 'CheckSpellingAsYouType', 'CheckGrammarAsYouType') {
    try { $word.Options.$k = $saved[$k] } catch {}
  }
  try { $word.UserName = $saved.UserName; $word.UserInitials = $saved.UserInitials } catch {}
  try { $word.Quit([ref]0) } catch {}
  [void][Runtime.InteropServices.Marshal]::ReleaseComObject($word)
  [GC]::Collect()
  foreach ($i in 1..20) {
    if (-not (Get-Process -Id $wordPid -ErrorAction SilentlyContinue)) { break }
    Start-Sleep -Milliseconds 500
  }
  if (Get-Process -Id $wordPid -ErrorAction SilentlyContinue) { Stop-Process -Id $wordPid -Force -ErrorAction SilentlyContinue }
}
