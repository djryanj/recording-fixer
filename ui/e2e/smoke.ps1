# Pre-flight for the end-to-end suite: does the built app start, and does WebView2 come up under it?
# Run before the WebDriver tests so a CI failure says "WebView2 never started" (with the environment
# printed) instead of the driver's opaque "DevToolsActivePort file doesn't exist".
#   pwsh ui/e2e/smoke.ps1            (APP_EXE and EDGEDRIVER override the defaults)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$exe = if ($env:APP_EXE) { $env:APP_EXE } else { Join-Path $here '../../target/debug/recording-fixer.exe' }
$driver = if ($env:EDGEDRIVER) { $env:EDGEDRIVER } else { Join-Path $here '.tools/edgedriver/msedgedriver.exe' }
$logs = Join-Path $here '.tools/logs'
New-Item -ItemType Directory -Force $logs | Out-Null

$id = [Security.Principal.WindowsIdentity]::GetCurrent()
$admin = ([Security.Principal.WindowsPrincipal]$id).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
$runtime = (Get-ItemProperty 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}' -ErrorAction SilentlyContinue).pv
Write-Host "user: $($id.Name)  elevated: $admin  session: $([Diagnostics.Process]::GetCurrentProcess().SessionId)"
Write-Host "WebView2 runtime: $runtime"
$loaded = Get-ChildItem 'C:\Program Files (x86)\Microsoft\EdgeWebView\Application' -Directory -ErrorAction SilentlyContinue | Where-Object { $_.Name -match '^\d+\.' } | Sort-Object { [version]$_.Name } | Select-Object -Last 1
Write-Host "WebView2 on disk:  $($loaded.Name)"
Write-Host "msedgedriver:     $(& $driver --version)"
Write-Host "app:              $exe"
if (-not (Test-Path $exe)) { throw "app not built: $exe" }

$work = Join-Path ([IO.Path]::GetTempPath()) ("srf-smoke-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory $work | Out-Null
$env:RECORDING_FIXER_FOLDERS = $work
$env:WEBVIEW2_USER_DATA_FOLDER = Join-Path $work '.webview'
$out = Join-Path $logs 'smoke-stdout.log'
$err = Join-Path $logs 'smoke-stderr.log'
$app = Start-Process $exe -PassThru -RedirectStandardOutput $out -RedirectStandardError $err

function Descendants($pid_) {
  Get-CimInstance Win32_Process | Where-Object { $_.ParentProcessId -eq $pid_ } | ForEach-Object { $_; Descendants $_.ProcessId }
}

$ok = $false
for ($i = 0; $i -lt 40; $i++) {
  Start-Sleep -Milliseconds 500
  if ($app.HasExited) { break }
  if (Descendants $app.Id | Where-Object { $_.Name -eq 'msedgewebview2.exe' }) { $ok = $true; break }
}

if ($app.HasExited) { Write-Host "app exited early with code $($app.ExitCode)" }
Write-Host "--- processes under the app"
Descendants $app.Id | ForEach-Object { Write-Host "  $($_.Name) ($($_.ProcessId))" }
foreach ($f in $out, $err) { if ((Get-Item $f).Length) { Write-Host "--- $(Split-Path -Leaf $f)"; Get-Content $f -Tail 40 } }

if (-not $app.HasExited) { Stop-Process -Id $app.Id -Force }
Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue

if (-not $ok) { Write-Host "::error::WebView2 did not start under the app"; exit 1 }
Write-Host "smoke ok: WebView2 started under the app"
