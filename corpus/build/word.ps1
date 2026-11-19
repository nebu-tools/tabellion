# Builds the Word-made documents of the test corpus in a Word instance of
# its own.
#
#   powershell.exe -NoProfile -ExecutionPolicy Bypass -File word.ps1 -Dir DIR
#
# DIR holds the pictures (bird.png, heron.png) and receives the documents.
# -Only names the documents to make, by file name without .docx; without it
# the script makes them all.
#
# Safety: the script starts its Word through tools/word/Word.psm1, which
# learns the process id of that instance from the instance itself and
# refuses one that was already running. It sets the user name and initials
# to invented values for the documents; Word.psm1 records each option before
# the first change and puts every one back when the script ends, however
# it ends.
param([Parameter(Mandatory)][string]$Dir, [string[]]$Only)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'Word.psm1') -Force

$AUTHOR_A = 'Wren Ashby'
$AUTHOR_B = 'Pip Tanager'

function As-Author($name, $initials) {
  Set-WordOption $session 'UserName' $name
  Set-WordOption $session 'UserInitials' $initials
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

function Cascade {
  $doc = $word.Documents.Add()
  # Toggle properties through the styles (ECMA-376 part 1, 17.7.3): one
  # paragraph for each case, its whole text in one formatting.
  $s = $doc.Styles.Add('Hidden Note', 1); $s.Font.Hidden = $true
  $s = $doc.Styles.Add('Bold Note', 1); $s.Font.Bold = $true
  $s = $doc.Styles.Add('Bird Name', 2); $s.Font.Color = 2263842
  $s = $doc.Styles.Add('Secret', 2); $s.Font.Hidden = $true
  $s = $doc.Styles.Add('Strong Note', 2); $s.Font.Bold = $true
  $s = $doc.Styles.Add('Slanted', 2); $s.Font.Italic = $true
  Para $doc 'Toggle properties through the styles' $TITLE | Out-Null
  $cases = @(
    @('A wren sang in the reeds.', 'Hidden Note', 'Bird Name', $null, $null),
    @('A robin sang on the fence.', 'Hidden Note', 'Secret', $null, $null),
    @('A dunnock fed under the hedge.', 'Bold Note', 'Strong Note', $null, $null),
    @('A teal rested on the pool.', 'Bold Note', 'Slanted', $null, $null),
    @('A heron stood in the shallows.', 'Hidden Note', $null, 'Hidden', $false),
    @('A swift flew over the marsh.', $NORMAL, 'Secret', 'Hidden', $false),
    @('A coot dived by the reeds.', 'Bold Note', 'Strong Note', 'Bold', $true),
    @('A moorhen crossed the path.', 'Bold Note', $null, 'Bold', $false),
    @('A kingfisher flashed past.', $NORMAL, 'Strong Note', 'Bold', $false),
    @('A snipe rose from the grass.', $NORMAL, 'Strong Note', $null, $null),
    @('The survey ends here.', $NORMAL, $null, $null, $null)
  )
  # The text first, then the formatting, by character positions taken
  # before any text is hidden: hidden text moves the place where Para
  # appends and the paragraphs Word counts.
  $at = @()
  foreach ($c in $cases) {
    $r = Para $doc $c[0] $NORMAL
    $at += ,@($r.Start, $r.End)
  }
  for ($i = $cases.Count - 1; $i -ge 0; $i--) {
    $c = $cases[$i]
    $r = $doc.Range($at[$i][0], $at[$i][1])
    $r.Paragraphs.Item(1).Style = $doc.Styles.Item($c[1])
    $r = $doc.Range($at[$i][0], $at[$i][1])
    if ($c[2]) { $r.Style = $doc.Styles.Item($c[2]) }
    if ($c[3]) { $r.Font.($c[3]) = $c[4] }
  }
  Scrub $doc 'Toggle properties'
  Save $doc 'cascade.docx'
}

function Changes {
  $doc = $word.Documents.Add()
  Para $doc 'Field notes with changes' $TITLE | Out-Null
  Para $doc 'The heron fed at the east pool.' $NORMAL | Out-Null
  Para $doc 'Sightings' $H1 | Out-Null
  $tp = Para $doc 'x' $NORMAL
  $t = $doc.Tables.Add($tp, 3, 3)
  $t.Borders.Enable = $true
  $rows = @(@('Bird', 'Site', 'Count'), @('Heron', 'East pool', '2'), @('Coot', 'Reed beds', '5'))
  for ($i = 0; $i -lt 3; $i++) { for ($j = 0; $j -lt 3; $j++) { $t.Cell($i + 1, $j + 1).Range.Text = $rows[$i][$j] } }
  # Content controls around a paragraph, around a row and around words.
  $p = Para $doc 'The weir was in flood on the first visit.' $NORMAL
  $cc = $doc.ContentControls.Add(0, $p.Paragraphs.Item(1).Range)
  $cc.Tag = 'note:block'; $cc.Title = 'Paragraph note'
  $rep = $doc.ContentControls.Add(9, $t.Rows.Item(3).Range)
  $rep.Tag = 'note:row'; $rep.Title = 'Sighting row'
  $p = Para $doc 'Observer: Wren Ashby.' $NORMAL
  $cc = $doc.ContentControls.Add(1, $doc.Range($p.Start + 10, $p.Start + 20))
  $cc.Tag = 'note:words'; $cc.Title = 'Observer'
  $last = Para $doc 'Counts are pairs.' $NORMAL
  $doc.Footnotes.Add((At-End $doc $last), [Type]::Missing, 'Seen from the hide.') | Out-Null
  $s1 = $doc.Sections.Item(1)
  $s1.Headers.Item(1).Range.Text = 'Field notes, spring'
  $s1.Footers.Item(1).Range.Text = 'Draft for the group'

  # A comment on two cells of a row.
  As-Author $AUTHOR_B 'PT'
  $doc.Comments.Add($doc.Range($t.Cell(2, 1).Range.Start, $t.Cell(2, 2).Range.End - 1), 'Both from the dawn visit.') | Out-Null

  $doc.TrackRevisions = $true
  As-Author $AUTHOR_A 'WA'
  $a = $doc.Paragraphs.Item(2).Range
  # Inserted by one author, then deleted in part by the other: a deletion
  # over an insertion.
  $doc.Range($a.End - 2, $a.End - 2).InsertBefore(' and the reed beds')
  $h = $s1.Headers.Item(1).Range
  $x = $h.Duplicate
  $x.SetRange($h.Start + 13, $h.Start + 19)
  $x.Text = 'summer'
  $f = $s1.Footers.Item(1).Range
  $y = $f.Duplicate
  $y.SetRange($f.Start + 6, $f.Start + 6)
  $y.InsertAfter('copy ')
  $n = $doc.Footnotes.Item(1).Range
  $z = $n.Duplicate
  $z.SetRange($n.End - 1, $n.End - 1)
  $z.InsertAfter(' at dawn')
  As-Author $AUTHOR_B 'PT'
  $a = $doc.Paragraphs.Item(2).Range
  $doc.Range($a.Start + 21, $a.Start + 39).Delete() | Out-Null
  $n = $doc.Footnotes.Item(1).Range
  $w = $n.Duplicate
  $w.SetRange($n.Start, $n.Start + 5)
  $w.Delete() | Out-Null
  $doc.TrackRevisions = $false
  As-Author $AUTHOR_A 'WA'
  Scrub $doc 'Field notes with changes'
  Save $doc 'changes.docx'
}

function Defaults {
  # Toggle properties in the document defaults. Word's own "Set As
  # Default" puts bold and italic into the Normal style and leaves
  # w:docDefaults alone, so the script makes the document in Word, adds bold and
  # italic to the defaults of its styles part, and has Word open the file
  # and save it again: the file is Word's, its defaults are not ones Word's
  # interface makes.
  $doc = $word.Documents.Add()
  $s = $doc.Styles.Add('Bold Note', 1); $s.Font.Bold = $true
  $s = $doc.Styles.Add('Plain Note', 1); $s.Font.Bold = $false
  $s = $doc.Styles.Add('Based Bold', 1); $s.BaseStyle = 'Bold Note'
  $s = $doc.Styles.Add('Strong Note', 2); $s.Font.Bold = $true
  $s = $doc.Styles.Add('Slanted', 2); $s.Font.Italic = $true
  $s = $doc.Styles.Add('Bold Grid', 3); $s.Font.Bold = $true
  $cases = @(
    @('A wren sang in the reeds.', $NORMAL, $null, $null, $null),
    @('A robin sang on the fence.', 'Bold Note', $null, $null, $null),
    @('A dunnock fed under the hedge.', $NORMAL, 'Strong Note', $null, $null),
    @('A teal rested on the pool.', 'Plain Note', $null, $null, $null),
    @('A heron stood in the shallows.', $NORMAL, $null, $null, $null),
    @('A swift flew over the marsh.', 'Based Bold', $null, $null, $null),
    @('A coot dived by the reeds.', 'Bold Note', 'Strong Note', $null, $null),
    @('A snipe rose from the grass.', $NORMAL, 'Slanted', $null, $null)
  )
  Para $doc 'Document defaults' $TITLE | Out-Null
  $at = @()
  foreach ($c in $cases) {
    $r = Para $doc $c[0] $NORMAL
    $at += ,@($r.Start, $r.End)
  }
  for ($i = $cases.Count - 1; $i -ge 0; $i--) {
    $c = $cases[$i]
    $r = $doc.Range($at[$i][0], $at[$i][1])
    $r.Paragraphs.Item(1).Style = $doc.Styles.Item($c[1])
    $r = $doc.Range($at[$i][0], $at[$i][1])
    if ($c[2]) { $r.Style = $doc.Styles.Item($c[2]) }
    if ($c[3]) { $r.Font.($c[3]) = $c[4] }
  }
  # A table in a bold table style.
  $end = $doc.Content.End - 1
  $tbl = $doc.Tables.Add($doc.Range($end, $end), 1, 2)
  $tbl.Style = 'Bold Grid'
  $tbl.Cell(1, 1).Range.Text = 'Wren'
  $tbl.Cell(1, 2).Range.Text = 'Robin'
  # A group content control around two paragraphs with a control inside,
  # made as Word's Developer tab makes one: select, then group. Adding it
  # to a range that is not selected gives "This is not a valid selection".
  $g = Para $doc 'Delivery to the north shed.' $NORMAL
  $g2 = Para $doc 'Payment on delivery.' $NORMAL
  $pay = $doc.ContentControls.Add(1, $doc.Range($g2.Start, $g2.Start + 7))
  $pay.Tag = 'delivery:payment'; $pay.Title = 'Payment'
  $doc.Range($g.Start, $g2.End).Select()
  $grp = $word.Selection.Range.ContentControls.Add(7)
  $grp.Tag = 'delivery:group'; $grp.Title = 'Delivery'
  Para $doc 'The survey ends here.' $NORMAL | Out-Null
  Scrub $doc 'Document defaults'
  Save $doc 'defaults-base.docx'
  # Bold and italic into the defaults.
  $base = Join-Path $Dir 'defaults-base.docx'
  Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
  $zip = [IO.Compression.ZipFile]::Open($base, 'Update')
  try {
    $e = $zip.GetEntry('word/styles.xml')
    $rd = New-Object IO.StreamReader($e.Open())
    try { $xml = $rd.ReadToEnd() } finally { $rd.Close() }
    $patched = [regex]::Replace($xml, '(<w:rPrDefault><w:rPr><w:rFonts[^>]*/>)', '$1<w:b/><w:bCs/><w:i/><w:iCs/>')
    if ($patched -eq $xml) { throw 'the styles part has no rPrDefault to add bold to' }
    $e.Delete()
    $n = $zip.CreateEntry('word/styles.xml')
    $wr = New-Object IO.StreamWriter($n.Open(), (New-Object Text.UTF8Encoding($false)))
    try { $wr.Write($patched) } finally { $wr.Close() }
  } finally { $zip.Dispose() }
  # Bold taken off again, now that it is the default: Word writes the
  # property as off only where it differs from what the text would have.
  $doc = $word.Documents.Open($base, $false, $false, $false)
  $doc.Styles.Item('Plain Note').Font.Bold = $false
  $heron = $doc.Paragraphs.Item(6).Range
  if (-not $heron.Text.StartsWith('A heron')) { throw "paragraph 6 is not the heron's: $($heron.Text)" }
  $doc.Range($heron.Start, $heron.End - 1).Font.Bold = $false
  Scrub $doc 'Document defaults'
  Save $doc 'defaults.docx'
  Remove-Item $base
}

function Wanted($name) { -not $Only -or (@($Only | ForEach-Object { $_ -split ',' }) -contains $name) }

$session = Start-OwnWord -PidFile (Join-Path $Dir 'word.pid')
$word = $session.Word
try {
  $word.Visible = $false
  $word.DisplayAlerts = 0
  Set-WordOption $session 'Options.UseLocalUserInfo' $true
  As-Author $AUTHOR_A 'WA'
  if (Wanted 'birds') { Birds }
  Test-ForcedFailure 'word.ps1, after the first document'
  if (Wanted 'parts') { Parts }
  if (Wanted 'recipes-de') { Recipes 'recipes-de.docx' 1031 'Rezepte aus dem Obstgarten' 'Apfelkuchen' `
    'Ein einfacher Kuchen mit Äpfeln aus dem eigenen Garten.' `
    @('Mehl', 'Eier', 'Butter') @('Den Teig rühren.', 'Die Äpfel schneiden.', 'Eine Stunde backen.') `
    'Ein Kuchen ist erst fertig, wenn die Küche danach duftet.' 'Tabelle' 'Zutaten für einen Kuchen' `
    'Säuerliche Sorten eignen sich am besten.' }
  if (Wanted 'recettes-fr') { Recipes 'recettes-fr.docx' 1036 'Recettes du verger' 'Tarte aux pommes' `
    'Une tarte simple avec des pommes du jardin.' `
    @('Farine', 'Œufs', 'Beurre') @('Préparer la pâte.', 'Couper les pommes.', 'Cuire une heure.') `
    'Une tarte est prête quand la cuisine sent bon.' 'Tableau' 'Ingrédients pour une tarte' `
    'Les variétés acidulées conviennent le mieux.' }
  if (Wanted 'cascade') { Cascade }
  if (Wanted 'changes') { Changes }
  if (Wanted 'defaults') { Defaults }
} catch {
  [Console]::Error.WriteLine("word.ps1, line $($_.InvocationInfo.ScriptLineNumber): $($_.Exception.Message)")
  throw
} finally {
  Stop-OwnWord $session
}
