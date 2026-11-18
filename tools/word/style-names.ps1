# Asks Word for the name it shows (Style.NameLocal) of every built-in style,
# and the name it stores for it, and writes the table that
# crates/nebu-docx/src/view/names/<language>.tsv holds.
#
#   powershell.exe -NoProfile -ExecutionPolicy Bypass -File style-names.ps1 -Out names.tsv [-Dir DIR]
#
# The names Word shows are those of the display language it runs in, which
# is set in Word (File, Options, Language) and which the script does not
# change. It changes no setting.
#
# The stored name comes from a saved file: the script gives every built-in
# style a font size of its own, from 500 points, saves the document in DIR
# and reads each style's w:name with its size from word/styles.xml. The
# default styles of each kind are found as the defaults of a blank
# document, and a style without a font in a document of its own.
param(
  [Parameter(Mandatory)][string]$Out,
  [string]$Dir = $env:TEMP
)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'Word.psm1') -Force
Add-Type -AssemblyName System.IO.Compression.FileSystem

function Styles-Xml($path) {
  $zip = [IO.Compression.ZipFile]::OpenRead($path)
  try {
    $entry = $zip.GetEntry('word/styles.xml')
    $r = New-Object IO.StreamReader($entry.Open())
    try { [xml]$r.ReadToEnd() } finally { $r.Close() }
  } finally { $zip.Dispose() }
}

$W = 'http://schemas.openxmlformats.org/wordprocessingml/2006/main'

function Save-Doc($doc, $path) {
  [object]$p = [string]$path
  [object]$f = 16
  $doc.SaveAs2([ref]$p, [ref]$f)
}

$session = Start-OwnWord
try {
  $word = $session.Word
  $word.Visible = $false
  $word.DisplayAlerts = 0
  $lcid = $word.LanguageSettings.LanguageID(2)
  $culture = [Globalization.CultureInfo]::GetCultureInfo([int]$lcid)
  Test-ForcedFailure 'style-names.ps1, after starting Word'

  # Every built-in style, by WdBuiltinStyle, with a font size of its own.
  $doc = $word.Documents.Add()
  $rows = New-Object System.Collections.ArrayList
  $byMark = @{}
  $k = 0
  for ($v = -1; $v -ge -400; $v--) {
    try { $st = $doc.Styles.Item($v) } catch { continue }
    $row = [pscustomobject]@{ Value = $v; Shown = $st.NameLocal; Type = [int]$st.Type; Stored = $null }
    [void]$rows.Add($row)
    $k++
    try {
      # Sizes from 500 points, which no built-in style has. A whole number
      # would reach Word as an integer, which it ignores.
      $st.Font.Size = [single](500 + $k / 2)
      $byMark["$(1000 + $k)"] = $row
    } catch {}
  }
  $path = Join-Path $Dir "style-names-$PID.docx"
  Save-Doc $doc $path
  $doc.Close([ref]0)
  $xml = Styles-Xml $path
  Remove-Item $path
  $ns = New-Object Xml.XmlNamespaceManager($xml.NameTable)
  $ns.AddNamespace('w', $W)
  # Word makes a linked character style ("Heading 1 Char") with the font
  # of its paragraph style; such a style is not built in.
  foreach ($s in $xml.SelectNodes('//w:style[not(@w:customStyle="1")]', $ns)) {
    $sz = $s.SelectSingleNode('w:rPr/w:sz/@w:val', $ns)
    $name = $s.SelectSingleNode('w:name/@w:val', $ns)
    if ($sz -and $name -and $byMark.ContainsKey($sz.Value)) {
      $byMark[$sz.Value].Stored = $name.Value
    }
  }
  # Styles without a font, one document each: the style the blank
  # document did not have.
  $blank = $word.Documents.Add()
  Save-Doc $blank $path
  $blank.Close([ref]0)
  $xml = Styles-Xml $path
  Remove-Item $path
  $ns = New-Object Xml.XmlNamespaceManager($xml.NameTable)
  $ns.AddNamespace('w', $W)
  $known = @{}
  foreach ($n in $xml.SelectNodes('//w:style/w:name/@w:val', $ns)) { $known[$n.Value] = $true }
  # Every document has the default style of each kind, which takes no font
  # size: wdStyleNormal, wdStyleDefaultParagraphFont, wdStyleNormalTable
  # and the list style No List (-108).
  $defaults = @{ 'paragraph' = -1; 'character' = -66; 'table' = -106; 'numbering' = -108 }
  foreach ($s in $xml.SelectNodes('//w:style[@w:default="1"]', $ns)) {
    $v = $defaults[$s.GetAttribute('type', $W)]
    $row = $rows | Where-Object { $_.Value -eq $v } | Select-Object -First 1
    if ($row -and -not $row.Stored) { $row.Stored = $s.SelectSingleNode('w:name/@w:val', $ns).Value }
  }
  foreach ($row in @($rows | Where-Object { -not $_.Stored })) {
    $d = $word.Documents.Add()
    try {
      $st = $d.Styles.Item($row.Value)
      try { $st.Priority = $st.Priority } catch {}
      try { $st.QuickStyle = -not $st.QuickStyle; $st.QuickStyle = -not $st.QuickStyle } catch {}
      Save-Doc $d $path
    } finally { $d.Close([ref]0) }
    $xml = Styles-Xml $path
    Remove-Item $path
    $ns = New-Object Xml.XmlNamespaceManager($xml.NameTable)
    $ns.AddNamespace('w', $W)
    $new = @($xml.SelectNodes('//w:style[not(@w:customStyle="1")]/w:name/@w:val', $ns) | Where-Object { -not $known.ContainsKey($_.Value) })
    if ($new.Count -eq 1) { $row.Stored = $new[0].Value }
  }
  $lines = New-Object System.Collections.ArrayList
  [void]$lines.Add("# The names Word shows for its built-in styles in display language $($culture.Name),")
  [void]$lines.Add("# written by tools/word/style-names.ps1 from Word $($word.Version). Each line is")
  [void]$lines.Add('# the name Word stores, a tab, and the name it shows.')
  $missing = @()
  $seen = @{}
  foreach ($row in $rows) {
    if (-not $row.Stored) { $missing += "$($row.Value) $($row.Shown)"; continue }
    if ($seen.ContainsKey($row.Stored.ToLowerInvariant())) {
      throw "two built-in styles came out stored as $($row.Stored)"
    }
    $seen[$row.Stored.ToLowerInvariant()] = $true
    [void]$lines.Add("$($row.Stored)`t$($row.Shown)")
  }
  $utf8 = New-Object Text.UTF8Encoding($false)
  [IO.File]::WriteAllText($Out, (($lines -join "`n") + "`n"), $utf8)
  foreach ($m in $missing) { [Console]::Error.WriteLine("style-names.ps1: no stored name for built-in style $m") }
  "$($rows.Count) built-in styles, $($lines.Count - 3) written, $($missing.Count) without a stored name"
} finally {
  Stop-OwnWord $session
}
