# Opens a docx in a new Word instance with the add-in sideloaded, waits until the
# e2e page reports done (a file named "done" appears), saves and quits.
param(
  [Parameter(Mandatory)][string]$Dir,
  [Parameter(Mandatory)][string]$AddinId,
  [int]$Timeout = 300
)
$ErrorActionPreference = 'Stop'
$key = 'HKCU:\Software\Microsoft\Office\16.0\WEF\Developer'
if (-not (Test-Path $key)) { New-Item -Path $key -Force | Out-Null }
New-ItemProperty -Path $key -Name $AddinId -Value "$Dir\manifest.xml" -PropertyType String -Force | Out-Null
$word = $null
try {
  $word = New-Object -ComObject Word.Application
  $word.Visible = $true
  $word.DisplayAlerts = 0
  $doc = $word.Documents.Open("$Dir\in.docx", $false, $false, $false)
  $t = 0
  while (-not (Test-Path "$Dir\done") -and $t -lt $Timeout) { Start-Sleep -Milliseconds 500; $t += 0.5 }
  if (-not (Test-Path "$Dir\done")) { Write-Error "Timeout: the add-in did not report back." }
  $doc.SaveAs2([ref]"$Dir\out.docx", [ref]16)
  $doc.Close([ref]0)
} finally {
  if ($word) {
    $word.Quit([ref]0)
    [void][Runtime.InteropServices.Marshal]::ReleaseComObject($word)
  }
  Remove-ItemProperty -Path $key -Name $AddinId -ErrorAction SilentlyContinue
}
