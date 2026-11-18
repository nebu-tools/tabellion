# Prints, as JSON, the Word settings that the scripts of this repository
# change or could change: the options as a Word instance of our own reads
# them, and the registry values behind them. Run it before and after a
# script and compare the two outputs.
#
#   powershell.exe -NoProfile -ExecutionPolicy Bypass -File settings.ps1
#
# It changes nothing.
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'Word.psm1') -Force

$names = @(
  'UserName', 'UserInitials', 'UserAddress',
  'Options.UseLocalUserInfo', 'Options.SaveNormalPrompt',
  'Options.CheckSpellingAsYouType', 'Options.CheckGrammarAsYouType',
  'Options.ConfirmConversions', 'Options.UpdateFieldsAtPrint',
  'Options.UpdateLinksAtOpen', 'Options.BackgroundSave'
)
$out = [ordered]@{ options = [ordered]@{}; registry = [ordered]@{} }
$session = Start-OwnWord
try {
  foreach ($n in $names) {
    try { $out.options[$n] = Get-WordValue $session.Word $n } catch { $out.options[$n] = "error: $($_.Exception.Message)" }
  }
} finally {
  Stop-OwnWord $session
}
foreach ($key in @(
  'HKCU:\Software\Microsoft\Office\Common\UserInfo',
  'HKCU:\Software\Microsoft\Office\16.0\Common\UserInfo',
  'HKCU:\Software\Microsoft\Office\16.0\Word\Options',
  'HKCU:\Software\Microsoft\Office\16.0\Common\LanguageResources'
)) {
  $values = [ordered]@{}
  if (Test-Path $key) {
    $item = Get-Item $key
    foreach ($v in ($item.GetValueNames() | Sort-Object)) {
      $data = $item.GetValue($v, $null, 'DoNotExpandEnvironmentNames')
      if ($data -is [byte[]]) { $data = [BitConverter]::ToString($data) }
      $values[$v] = "$data"
    }
  }
  $out.registry[$key] = $values
}
$out | ConvertTo-Json -Depth 5
