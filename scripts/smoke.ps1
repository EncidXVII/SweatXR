# AUTO GATE 7: SteamVR smoke test with the null driver, no headset needed.
#   powershell -ExecutionPolicy Bypass -File scripts\smoke.ps1 -Seconds 1800
#   powershell -ExecutionPolicy Bypass -File scripts\smoke.ps1 -Register
# -Register checks the install path instead: with SteamVR running, `fitvr --register` must make
# SteamVR start the overlay, which then exits cleanly when SteamVR quits. That overlay uses the
# real data directory, since SteamVR starts it without this script's environment.
# Backs up steamvr.vrsettings and appconfig.json, enables driver_null, runs the overlay with a
# fake strap and the watch pinned in view (no controllers) against a throwaway database,
# samples memory and CPU, then restores everything. If this script is killed (not Ctrl+C'd)
# the restore does not run: copy the files back from %TEMP%itvr-smoke-<pid>.
param(
    [int]$Seconds = 120,
    # Caches (glyphs, SQLite pages, first panel render) fill early; growth is measured after this.
    [int]$Warmup = 60,
    [string]$Exe = "$PSScriptRoot\..\target\release\fitvr.exe",
    [string]$Steam = "${env:ProgramFiles(x86)}\Steam",
    [switch]$Register
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
if ($hadAutolaunch) { Copy-Item $autolaunch $backup }
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
    if ($Register) {
        # Start as a first-time user: SteamVR running, FitnessVR not registered yet.
        Remove-Item $autolaunch -ErrorAction SilentlyContinue
        $apps = Get-Content $appconfig -Raw | ConvertFrom-Json
        $apps.manifest_paths = @($apps.manifest_paths | Where-Object { $_ -notmatch 'fitvr\.vrmanifest$' })
        [IO.File]::WriteAllText($appconfig, ($apps | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    }

    Start-Process (Join-Path $Steam 'steamapps\common\SteamVR\bin\win64\vrstartup.exe')
    $deadline = (Get-Date).AddSeconds(90)
    while (-not (Get-Process vrserver -ErrorAction SilentlyContinue)) {
        if ((Get-Date) -gt $deadline) { throw 'vrserver did not start' }
        Start-Sleep 2
    }
    Start-Sleep 15

    if ($Register) {
        "SteamVR processes: $((Get-Process $vrNames -ErrorAction SilentlyContinue).Name -join ', ')"
        $reg = Start-Process $Exe -ArgumentList '--register' -Wait -PassThru
        $realData = Join-Path $env:APPDATA 'FitnessVR\data'
        Get-Content (Join-Path $realData 'register.log')
        $deadline = (Get-Date).AddSeconds(30)
        do {
            Start-Sleep 2
            $ov = Get-CimInstance Win32_Process -Filter "Name = 'fitvr.exe'" | Where-Object CommandLine -match '--overlay'
        } while (-not $ov -and (Get-Date) -lt $deadline)
        Start-Sleep 20
        # Closing the SteamVR window from a script does not quit SteamVR, so stop it; the
        # overlay must notice and exit on its own.
        Get-Process $vrNames -ErrorAction SilentlyContinue | Stop-Process -Force
        $deadline = (Get-Date).AddSeconds(30)
        while ($ov -and (Get-Process -Id $ov.ProcessId -ErrorAction SilentlyContinue) -and (Get-Date) -lt $deadline) { Start-Sleep 2 }
        $log = Get-Content (Join-Path $realData 'overlay.log') -ErrorAction SilentlyContinue
        "register exit  $($reg.ExitCode)"
        "overlay began  $([bool]$ov)"
        $log | Select-Object -Last 10
        $failed = $reg.ExitCode -ne 0 -or -not $ov -or -not ($log -match 'overlay stopped') -or ($log -match ' ERROR ')
    } else {
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
}
finally {
    Get-Process $vrNames -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep 3
    Copy-Item (Join-Path $backup 'steamvr.vrsettings') $settings -Force
    if (Test-Path (Join-Path $backup 'appconfig.json')) { Copy-Item (Join-Path $backup 'appconfig.json') $appconfig -Force }
    if ($hadAutolaunch) { Copy-Item (Join-Path $backup (Split-Path $autolaunch -Leaf)) $autolaunch -Force }
    else { Remove-Item $autolaunch -ErrorAction SilentlyContinue }
    "restored SteamVR settings from $backup"
}
if ($failed) { "SMOKE FAILED"; exit 1 } else { "SMOKE PASSED" }
