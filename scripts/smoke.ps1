# AUTO GATE 7: SteamVR smoke test with the null driver, no headset needed.
#   powershell -ExecutionPolicy Bypass -File scripts\smoke.ps1 -Seconds 1800
# Backs up steamvr.vrsettings and appconfig.json, enables driver_null, runs the overlay with a
# fake strap and the watch pinned in view (no controllers) against a throwaway database,
# samples memory and CPU, then restores everything. If this script is killed (not Ctrl+C'd)
# the restore does not run: copy the files back from %TEMP%itvr-smoke-<pid>.
param(
    [int]$Seconds = 120,
    # Caches (glyphs, SQLite pages, first panel render) fill early; growth is measured after this.
    [int]$Warmup = 60,
    [string]$Exe = "$PSScriptRoot\..\target\release\fitvr.exe",
    [string]$Steam = "${env:ProgramFiles(x86)}\Steam"
)
$ErrorActionPreference = 'Stop'
$vrNames = 'vrserver', 'vrmonitor', 'vrcompositor', 'vrdashboard', 'vrwebhelper', 'vrstartup'
if (Get-Process $vrNames -ErrorAction SilentlyContinue) { throw 'SteamVR is running; close it first.' }

$config = Join-Path $Steam 'config'
$settings = Join-Path $config 'steamvr.vrsettings'
$appconfig = Join-Path $config 'appconfig.json'
$autolaunch = Join-Path $config 'vrappconfig\local.fitnessvr.overlay.vrappconfig'
$hadAutolaunch = Test-Path $autolaunch
$backup = Join-Path $env:TEMP "fitvr-smoke-$PID"
New-Item -ItemType Directory -Force $backup | Out-Null
Copy-Item $settings, $appconfig $backup -ErrorAction SilentlyContinue
$data = Join-Path $backup 'data'
$failed = $false
try {
    $json = Get-Content $settings -Raw | ConvertFrom-Json
    $json | Add-Member -Force driver_null ([pscustomobject]@{ enable = $true })
    if (-not $json.steamvr) { $json | Add-Member steamvr ([pscustomobject]@{}) }
    foreach ($kv in @{ requireHmd = $false; forcedDriver = 'null'; activateMultipleDrivers = $true }.GetEnumerator()) {
        $json.steamvr | Add-Member -Force $kv.Key $kv.Value
    }
    $json | ConvertTo-Json -Depth 32 | Set-Content $settings -Encoding utf8

    Start-Process (Join-Path $Steam 'steamapps\common\SteamVR\bin\win64\vrstartup.exe')
    $deadline = (Get-Date).AddSeconds(90)
    while (-not (Get-Process vrserver -ErrorAction SilentlyContinue)) {
        if ((Get-Date) -gt $deadline) { throw 'vrserver did not start' }
        Start-Sleep 2
    }
    Start-Sleep 15

    $env:FITVR_DATA_DIR = $data
    $p = Start-Process $Exe -ArgumentList '--overlay', '--fake-hr', '--pin-watch', '--exit-after', $Seconds -PassThru
    Start-Sleep $Warmup
    $p.Refresh()
    $mem0 = $p.WorkingSet64; $cpu0 = $p.TotalProcessorTime; $t0 = Get-Date
    $mem1 = $mem0; $cpu1 = $cpu0; $t1 = $t0
    while (-not $p.HasExited) {
        Start-Sleep 5
        $p.Refresh()
        if (-not $p.HasExited) {
            $mem1 = $p.WorkingSet64; $cpu1 = $p.TotalProcessorTime; $t1 = Get-Date
            if ([int]($t1 - $t0).TotalSeconds % 60 -lt 5) { "  +{0:N0} s  {1:N1} MB" -f ($t1 - $t0).TotalSeconds, ($mem1 / 1MB) }
        }
    }
    $cpuPct = 100 * ($cpu1 - $cpu0).TotalSeconds / [math]::Max(1, ($t1 - $t0).TotalSeconds)
    $growMb = ($mem1 - $mem0) / 1MB
    $log = Get-Content (Join-Path $data 'overlay.log') -ErrorAction SilentlyContinue
    $errors = @($log | Select-String -Pattern ' (ERROR|WARN) ')
    "exit code      $($p.ExitCode)"
    "memory growth  {0:N1} MB (limit 10)" -f $growMb
    "cpu            {0:N2} % of one core (limit 2)" -f $cpuPct
    "log warnings   $($errors.Count)"
    $log | Select-Object -Last 10
    $failed = $p.ExitCode -ne 0 -or $growMb -ge 10 -or $cpuPct -ge 2 -or $errors.Count -gt 0
}
finally {
    Get-Process $vrNames -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep 3
    Copy-Item (Join-Path $backup 'steamvr.vrsettings') $settings -Force
    if (Test-Path (Join-Path $backup 'appconfig.json')) { Copy-Item (Join-Path $backup 'appconfig.json') $appconfig -Force }
    if (-not $hadAutolaunch) { Remove-Item $autolaunch -ErrorAction SilentlyContinue }
    "restored SteamVR settings from $backup"
}
if ($failed) { "SMOKE FAILED"; exit 1 } else { "SMOKE PASSED" }
