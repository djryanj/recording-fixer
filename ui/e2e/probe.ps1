# Used by smoke.ps1: launch the app with the browser arguments msedgedriver would set, and write to -Out
# whether Chromium produced DevToolsActivePort, which arguments the browser process got, and its log.
param([string]$Exe, [string]$Out)
$ErrorActionPreference = 'Continue'
$lines = New-Object System.Collections.Generic.List[string]
function Say($t) { $lines.Add([string]$t) }
function Descendants($p) { Get-CimInstance Win32_Process | Where-Object { $_.ParentProcessId -eq $p } | ForEach-Object { $_; Descendants $_.ProcessId } }

$id = [Security.Principal.WindowsIdentity]::GetCurrent()
$admin = ([Security.Principal.WindowsPrincipal]$id).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
Say "running as $($id.Name), elevated: $admin"

$work = Join-Path ([IO.Path]::GetTempPath()) ("srf-probe-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory $work | Out-Null
$env:RECORDING_FIXER_FOLDERS = $work
$env:WEBVIEW2_USER_DATA_FOLDER = Join-Path $work '.webview'
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--enable-automation --enable-logging --log-level=0 --remote-debugging-port=0 --test-type=webdriver'
$app = Start-Process $Exe -PassThru
$found = $null
for ($i = 0; $i -lt 60 -and -not $found; $i++) {
  Start-Sleep -Milliseconds 500
  $found = Get-ChildItem $work -Recurse -Filter DevToolsActivePort -Force -ErrorAction SilentlyContinue | Select-Object -First 1
}
Say "DevToolsActivePort $(if ($found) { 'appeared' } else { 'did NOT appear' })"
$log = Get-ChildItem $work -Recurse -Force -Filter chrome_debug.log -ErrorAction SilentlyContinue | Select-Object -First 1
Say "chrome_debug.log (written only when the arguments were applied): $(if ($log) { 'present' } else { 'absent' })"
foreach ($p in (Descendants $app.Id | Where-Object { $_.Name -eq 'msedgewebview2.exe' -and $_.CommandLine -notmatch '--type=' })) {
  Say "browser process args: $($p.CommandLine)"
}
if ($log) { Say '--- chrome_debug.log (last 15 lines)'; Get-Content $log.FullName -Tail 15 | ForEach-Object { Say $_ } }
# Is the app itself alive and showing a window, and what does the browser side look like?
$app.Refresh()
Say "app: exited=$($app.HasExited) responding=$($app.Responding) window='$($app.MainWindowTitle)' cpu=$([math]::Round($app.CPU, 1))s"
Say '--- every process under the app (type, pid, cpu seconds, working set MB)'
foreach ($p in (Descendants $app.Id)) {
  $type = if ($p.CommandLine -match '--type=([w-]+)') { $Matches[1] } else { 'browser' }
  $gp = Get-Process -Id $p.ProcessId -ErrorAction SilentlyContinue
  Say "  $($p.Name) $type pid=$($p.ProcessId) cpu=$([math]::Round($gp.CPU, 1)) ws=$([math]::Round($gp.WorkingSet64 / 1MB))"
}
$dumps = Get-ChildItem $work -Recurse -Force -Include *.dmp -ErrorAction SilentlyContinue
Say "crash dumps under the user data folder: $(@($dumps).Count)"
$events = Get-WinEvent -FilterHashtable @{ LogName = 'Application'; StartTime = (Get-Date).AddMinutes(-3) } -ErrorAction SilentlyContinue | Where-Object { $_.LevelDisplayName -in 'Error', 'Warning' -and $_.Message -match 'recording-fixer|msedgewebview2|WebView2' } | Select-Object -First 5
foreach ($e in $events) { Say "event $($e.Id) $($e.ProviderName): $(($e.Message -split "`n")[0])" }
if (-not $app.HasExited) { Stop-Process -Id $app.Id -Force }
Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
Set-Content -Path $Out -Value $lines
