# Measure Audio Share playback on the real phone.
#
# Starts a server, drives the released v0.3.4 Android client, samples
# AudioFlinger underrun frames and wlan0 receive bytes, then reports.
# The phone must be connected by adb and reachable at the Hotspot address.
param(
    [Parameter(Mandatory = $true)][string]$ServerExe,
    [string[]]$ServerArgs = @(),
    [int]$MeasureSeconds = 40,
    [int]$WarmupSeconds = 4,
    [string]$OutDir = "$PSScriptRoot\measure",
    [string]$PhoneHost = "192.168.137.1",
    [int]$Port = 65530,
    [int]$PlayX = 538,
    [int]$PlayY = 1150
)

$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force $OutDir | Out-Null
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$runDir = Join-Path $OutDir $stamp
New-Item -ItemType Directory -Force $runDir | Out-Null

function Invoke-Adb([string[]]$AdbArgs) {
    $out = & adb @AdbArgs 2>$null
    if ($LASTEXITCODE -ne 0) { throw "adb $($AdbArgs -join ' ') failed" }
    return $out
}

function Get-AppPid {
    $pidText = (Invoke-Adb @('shell', 'pidof io.github.mkckr0.audio_share_app')) -join ''
    if ([string]::IsNullOrWhiteSpace($pidText)) { return $null }
    return [int]$pidText.Trim()
}

function Get-ActiveTrack([int]$AppPid) {
    $fl = (Invoke-Adb @('shell', 'dumpsys media.audio_flinger')) -join "`n"
    $best = $null
    foreach ($line in ($fl -split "`n")) {
        $t = $line.Trim() -split '\s+'
        if ($t.Count -ge 24 -and $t[2] -eq "$AppPid" -and $t[0] -match '^\d+$' -and $t[1] -eq 'yes') {
            $candidate = [pscustomobject]@{
                TrackId   = [int]$t[0]
                SessionId = [int]$t[3]
                Underruns = [int64]$t[21]
                Frames    = [int]$t[18]
            }
            if ($null -eq $best -or $candidate.Underruns -gt $best.Underruns) { $best = $candidate }
        }
    }
    return $best
}

function Get-WlanRxBytes {
    $dev = (Invoke-Adb @('shell', 'cat /proc/net/dev')) | Select-String 'wlan0'
    return [int64](($dev -split '\s+')[2])
}

function Wait-Listener([int]$Seconds) {
    $deadline = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $deadline) {
        $conn = Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue
        if ($conn) { return $true }
        Start-Sleep -Milliseconds 250
    }
    return $false
}

# Clean client start: force-stop then launch the released app.
Invoke-Adb @('shell', 'am force-stop io.github.mkckr0.audio_share_app') | Out-Null
Start-Sleep -Seconds 1
Invoke-Adb @('shell', 'am start -n io.github.mkckr0.audio_share_app/.ui.MainActivity') | Out-Null
Start-Sleep -Seconds 2

adb logcat -c

$stdout = Join-Path $runDir 'server-stdout.log'
$stderr = Join-Path $runDir 'server-stderr.log'
$server = Start-Process -FilePath $ServerExe -ArgumentList $ServerArgs `
    -RedirectStandardOutput $stdout -RedirectStandardError $stderr -WindowStyle Hidden -PassThru
Set-Content (Join-Path $runDir 'server.pid') $server.Id

try {
    if (-not (Wait-Listener 15)) { throw "server did not listen on $Port" }

    # Tap the play button, then wait for the client to create an AudioTrack.
    Invoke-Adb @('shell', 'input', 'tap', "$PlayX", "$PlayY") | Out-Null
    Start-Sleep -Seconds $WarmupSeconds

    $appPid = Get-AppPid
    if ($null -eq $appPid) { throw "Android client process is not running" }
    Set-Content (Join-Path $runDir 'app-pid.txt') $appPid

    $first = Get-ActiveTrack $appPid
    if ($null -eq $first) { throw "no active AudioTrack for the Android client" }

    $rx0 = Get-WlanRxBytes
    $t0 = Get-Date
    $rows = [System.Collections.Generic.List[object]]::new()
    for ($i = 0; $i -lt $MeasureSeconds; $i++) {
        $track = Get-ActiveTrack $appPid
        $rx = Get-WlanRxBytes
        $now = Get-Date
        $rows.Add([pscustomobject]@{
            Elapsed        = [math]::Round(($now - $t0).TotalSeconds, 3)
            WlanRxBytes    = $rx
            UnderrunFrames = if ($track) { $track.Underruns } else { $null }
            TrackId        = if ($track) { $track.TrackId } else { $null }
            SessionId      = if ($track) { $track.SessionId } else { $null }
        })
        Start-Sleep -Milliseconds 250
    }
    $t1 = Get-Date
    $rx1 = Get-WlanRxBytes
    $last = Get-ActiveTrack $appPid
    $rows | Export-Csv (Join-Path $runDir 'monitor.csv') -NoTypeInformation

    $elapsed = ($t1 - $t0).TotalSeconds
    $bytes = $rx1 - $rx0
    $underrunStart = $first.Underruns
    $underrunEnd = if ($last) { $last.Underruns } else { $underrunStart }
    $underrunFrames = $underrunEnd - $underrunStart
    $sampleRate = 48000
    $blockAlign = 8      # f32 stereo
    $payloadRate = $sampleRate * $blockAlign
    # v0.3.4 splits a 10 ms frame (3840 B) into 1464+1464+912 and adds 28 B per
    # datagram (20 B IPv4 + 8 B UDP); Ethernet/Wi-Fi adds ~14 B per datagram.
    $wireRate = $payloadRate + (300 * (3 * 28 + 3 * 14))
    $summary = [pscustomobject]@{
        Server          = $ServerExe
        ServerArgs      = ($ServerArgs -join ' ')
        Seconds         = [math]::Round($elapsed, 3)
        TrackId         = $first.TrackId
        SessionId       = $first.SessionId
        BufferFrames    = $first.Frames
        UnderrunStart   = $underrunStart
        UnderrunEnd     = $underrunEnd
        UnderrunFrames  = $underrunFrames
        UnderrunSeconds = [math]::Round($underrunFrames / $sampleRate, 3)
        UnderrunPercent = [math]::Round(100.0 * $underrunFrames / ($elapsed * $sampleRate), 2)
        WlanRxBytes     = $bytes
        WlanRateBps     = [math]::Round($bytes / $elapsed, 0)
        ExpectedWireBps = $wireRate
        RxToExpectedPct = [math]::Round(100.0 * ($bytes / $elapsed) / $wireRate, 1)
    }
    $summary | ConvertTo-Json | Set-Content (Join-Path $runDir 'summary.json')
    $summary | Format-List
}
finally {
    # Stop sampling before killing the server: the client keeps its AudioTrack
    # running while reconnecting, which would inflate the underrun counter.
    Get-Process -Id $server.Id -ErrorAction SilentlyContinue | Stop-Process -Force
    Invoke-Adb @('shell', 'am force-stop io.github.mkckr0.audio_share_app') | Out-Null
    adb logcat -d -v time > (Join-Path $runDir 'logcat.txt') 2>$null
    Write-Output "evidence: $runDir"
}
