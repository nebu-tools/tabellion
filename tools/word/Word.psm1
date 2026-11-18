# A Word instance of our own, and the options we change in it put back.
#
# Used by tools/word/check.ps1, tools/word/style-names.ps1,
# tools/word/settings.ps1 and corpus/build/word.ps1:
#
#   Import-Module (Join-Path $PSScriptRoot 'Word.psm1')
#   $session = Start-OwnWord -PidFile word.pid
#   try {
#     Set-WordOption $session 'UserName' 'Wren Ashby'
#     ... work with $session.Word ...
#   } finally {
#     Stop-OwnWord $session
#   }
#
# Start-OwnWord starts a new, invisible Word.Application and learns the
# process id of exactly that instance from the instance itself: it opens a
# blank document, takes the handle of its window and asks Windows which
# process owns that window. It refuses an instance that has documents or is
# visible, and one whose process ran before it started Word, and then
# touches nothing. It changes no option.
#
# Set-WordOption records the value an option has before it changes it, the
# first time it changes that option. Word also writes some of its view
# state to the registry when it quits (VisiFlm under Word\Options after a
# field update, for one), so Start-OwnWord records the values under
# HKCU\Software\Microsoft\Office\16.0\Word\Options before it starts
# Word, and Stop-OwnWord puts back, after its Word has quit, each value that
# differs, and says which. Stop-OwnWord puts back every recorded
# option, each in its own try, quits only this instance, and stops only its
# process if it does not end. If the instance is gone before the options
# are back, Stop-OwnWord starts a second instance of its own to put them
# back there. A caller sets up the try/finally right after Start-OwnWord,
# before its first change.
Set-StrictMode -Version 2

if (-not ('Tabellion.Win32' -as [type])) {
  Add-Type -Namespace Tabellion -Name Win32 -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern uint GetWindowThreadProcessId(System.IntPtr hWnd, out uint processId);
'@
}

function Get-WordIds { @(Get-Process WINWORD -ErrorAction SilentlyContinue | ForEach-Object { $_.Id }) }

$OptionsKey = 'HKCU:\Software\Microsoft\Office\16.0\Word\Options'

# The values under a registry key: name to kind and data.
function Get-RegistryValues([string]$key) {
  $values = @{}
  if (Test-Path $key) {
    $item = Get-Item $key
    foreach ($v in $item.GetValueNames()) {
      $values[$v] = @($item.GetValueKind($v), $item.GetValue($v, $null, 'DoNotExpandEnvironmentNames'))
    }
  }
  $values
}

function Same-Data($a, $b) {
  if ($a -is [byte[]] -or $b -is [byte[]]) {
    return ($a -is [byte[]]) -and ($b -is [byte[]]) -and ([Convert]::ToBase64String($a) -eq [Convert]::ToBase64String($b))
  }
  "$a" -eq "$b"
}

# Puts the values of a key back as recorded, each in its own try: changed
# and removed ones set again, added ones removed.
function Restore-RegistryValues([string]$key, $before) {
  $now = Get-RegistryValues $key
  foreach ($v in $now.Keys) {
    if ($before.ContainsKey($v)) { continue }
    try { Remove-ItemProperty -Path $key -Name $v; [Console]::Error.WriteLine("Word.psm1: removed $v, which Word added under $key") }
    catch { [Console]::Error.WriteLine("Word.psm1: could not remove $v under $key") }
  }
  foreach ($v in $before.Keys) {
    $kind, $data = $before[$v]
    if ($now.ContainsKey($v) -and (Same-Data $now[$v][1] $data)) { continue }
    try {
      New-ItemProperty -Path $key -Name $v -PropertyType $kind -Value $data -Force | Out-Null
      [Console]::Error.WriteLine("Word.psm1: put back $v under $key, which Word had changed")
    } catch { [Console]::Error.WriteLine("Word.psm1: could not put back $v under $key") }
  }
}

# Starts a new Word and returns a session: the application, its process id
# and the options changed so far.
function Start-OwnWord {
  param([string]$PidFile)
  $registry = Get-RegistryValues $OptionsKey
  $before = Get-WordIds
  $word = New-Object -ComObject Word.Application
  $wordPid = 0
  try {
    if ($word.Documents.Count -ne 0 -or $word.Visible) {
      throw 'COM handed back a Word that has documents or is visible, so not one of our own; not touching it.'
    }
    $doc = $word.Documents.Add()
    $hwnd = [IntPtr][long]$doc.ActiveWindow.Hwnd
    $doc.Close([ref]0)
    [uint32]$id = 0
    [void][Tabellion.Win32]::GetWindowThreadProcessId($hwnd, [ref]$id)
    if ($id -eq 0) { throw 'Word gave no window whose process could be found; not touching it.' }
    if ($before -contains [int]$id) {
      throw "The Word of process $id ran before this script started Word; not touching it."
    }
    $wordPid = [int]$id
  } catch {
    [void][Runtime.InteropServices.Marshal]::ReleaseComObject($word)
    throw
  }
  if ($PidFile) { Set-Content -Path $PidFile -Value $wordPid }
  [pscustomobject]@{
    Word = $word
    Pid = $wordPid
    Saved = [ordered]@{}
    Registry = $registry
  }
}

# An option is a property of the application (UserName) or of its Options
# object (Options.UseLocalUserInfo).
function Get-WordValue($word, [string]$name) {
  if ($name.StartsWith('Options.')) { return $word.Options.($name.Substring(8)) }
  $word.$name
}

function Set-WordValue($word, [string]$name, $value) {
  if ($name.StartsWith('Options.')) { $word.Options.($name.Substring(8)) = $value }
  else { $word.$name = $value }
}

# Changes an option that Word keeps from one run to the next, after
# recording the value it had.
function Set-WordOption {
  param([Parameter(Mandatory)]$Session, [Parameter(Mandatory)][string]$Name, $Value)
  if (-not $Session.Saved.Contains($Name)) {
    $Session.Saved[$Name] = Get-WordValue $Session.Word $Name
  }
  Set-WordValue $Session.Word $Name $Value
}

# Puts back the recorded options in the given Word; returns the names of
# those it could not put back.
function Restore-Options($word, $saved) {
  $failed = @()
  foreach ($name in @($saved.Keys)) {
    try { Set-WordValue $word $name $saved[$name] }
    catch { $failed += $name }
  }
  ,$failed
}

# Puts back the options, quits this instance and makes sure its process is
# gone. Never stops another process.
function Stop-OwnWord {
  param([Parameter(Mandatory)]$Session)
  $failed = Restore-Options $Session.Word $Session.Saved
  try { $Session.Word.NormalTemplate.Saved = $true } catch {}
  try { $Session.Word.Quit([ref]0) } catch {}
  try { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($Session.Word) } catch {}
  [GC]::Collect()
  [GC]::WaitForPendingFinalizers()
  foreach ($i in 1..40) {
    if (-not (Get-Process -Id $Session.Pid -ErrorAction SilentlyContinue)) { break }
    Start-Sleep -Milliseconds 250
  }
  if (Get-Process -Id $Session.Pid -ErrorAction SilentlyContinue) {
    Stop-Process -Id $Session.Pid -Force -ErrorAction SilentlyContinue
    [Console]::Error.WriteLine("Word.psm1: stopped Word process $($Session.Pid), which did not quit")
  }
  Restore-RegistryValues $OptionsKey $Session.Registry
  if ($failed.Count -gt 0) {
    # The instance died before its options were back: put them back in a
    # second instance of our own.
    $retry = [ordered]@{}
    foreach ($n in $failed) { $retry[$n] = $Session.Saved[$n] }
    $second = Start-OwnWord
    try {
      $left = Restore-Options $second.Word $retry
      foreach ($n in $left) { [Console]::Error.WriteLine("Word.psm1: could not put back $n; it was $($retry[$n])") }
    } finally {
      Stop-OwnWord $second
    }
  }
}

# Throws half-way through a script when TABELLION_WORD_FAIL is set, to show
# that the options come back on every way out.
function Test-ForcedFailure([string]$where) {
  if ($env:TABELLION_WORD_FAIL) { throw "forced failure at $where (TABELLION_WORD_FAIL is set)" }
}

Export-ModuleMember -Function Start-OwnWord, Stop-OwnWord, Set-WordOption, Get-WordValue, Test-ForcedFailure
