#requires -Version 5.1
<#
.SYNOPSIS
    A/B the old v0.3.4 `as-cmd.exe` against the rewritten Server on Windows.

.DESCRIPTION
    Records the frozen v0.3.4 Wire Protocol from both servers with
    `protocol-harness record`, then compares:

      * the exact GET_FORMAT response bytes and the advertised AudioFormat
        (encoding, channels, sample rate, block align);
      * PCM level statistics (RMS/peak dBFS) and byte counts of the captured
        datagrams;
      * UDP datagram size distributions (min/max/full-size count);
      * optionally, capture-side glitch counters from `wasapi-spike measure`
        for the new WASAPI backend (the old server has no counter output).

    Both servers must capture the same endpoint and format while the same
    continuous tone (or other known source) plays into that endpoint. The
    script does not start or select a tone source; do that manually and keep it
    running for both recordings.

    PCM content is NOT byte-compared: the two recordings are sequential. Levels
    are compared within -LevelToleranceDb (default 3 dB).

    NOT EXECUTED in the Linux implementation environment; see
    docs/server-rewrite/capture-windows.md.

.PARAMETER OldServer
    Path to the old v0.3.4 `as-cmd.exe`.

.PARAMETER NewServer
    Path to the rewritten `as-cmd.exe` (or `AudioShareServer.exe` once it can
    run headless with the same flags).

.PARAMETER Harness
    Path to `protocol-harness.exe` (default: resolved from PATH).

.PARAMETER OldServerArgs
    Extra server arguments for the old binary, for example
    @("-e", "<endpoint-id>", "--encoding", "f32", "--channels", "2",
      "--sample-rate", "48000"). `--bind` is added automatically.

.PARAMETER NewServerArgs
    Extra server arguments for the new binary. Use the same capture selection
    as the old side; the new CLI accepts endpoint/encoding flags from ticket 07
    onward, so pass them here once available.

.PARAMETER SpikeExe
    Optional path to `wasapi-spike.exe` (ticket 02). When given, runs
    `measure` for the new capture path and stores its glitch counters.

.EXAMPLE
    .\scripts\ab-compare-windows.ps1 `
        -OldServer C:\audio-share\old\as-cmd.exe `
        -NewServer .\target\release\as-cmd.exe `
        -Harness .\target\release\protocol-harness.exe `
        -SpikeExe .\target\release\wasapi-spike.exe
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $OldServer,

    [Parameter(Mandatory = $true)]
    [string] $NewServer,

    [string] $Harness = "protocol-harness.exe",

    [string] $WorkDir = "",

    [string[]] $OldServerArgs = @(),

    [string[]] $NewServerArgs = @(),

    [string] $SpikeExe = "",

    [string] $Endpoint = "default",

    [string] $Encoding = "default",

    [int] $Channels = 0,

    [int] $SampleRate = 0,

    [int] $Datagrams = 64,

    [int] $Heartbeats = 3,

    [int] $TimeoutSecs = 45,

    [double] $LevelToleranceDb = 3.0,

    [int] $SpikeSeconds = 60
)

$ErrorActionPreference = "Stop"

function Resolve-ToolPath {
    param(
        [string] $Path,
        [string] $Name
    )
    if ([string]::IsNullOrWhiteSpace($Path)) {
        throw "$Name path is empty"
    }
    if (Test-Path -LiteralPath $Path) {
        return (Resolve-Path -LiteralPath $Path).Path
    }
    $command = Get-Command $Path -ErrorAction SilentlyContinue
    if ($null -ne $command) {
        return $command.Source
    }
    throw "$Name not found: $Path"
}

function Invoke-Record {
    param(
        [string] $Server,
        [string] $OutDir,
        [string[]] $ExtraArgs,
        [string] $Label
    )

    if (Test-Path -LiteralPath $OutDir) {
        Remove-Item -LiteralPath $OutDir -Recurse -Force
    }
    New-Item -ItemType Directory -Path $OutDir -Force | Out-Null

    $arguments = @(
        "record",
        "--server-cmd", $Server,
        "--server-arg=--bind={bind}",
        "--out", $OutDir,
        "--datagrams", "$Datagrams",
        "--heartbeats", "$Heartbeats",
        "--timeout-secs", "$TimeoutSecs",
        "--provenance", "ab-windows-$Label",
        "--server-label", $Label
    )
    foreach ($argument in $ExtraArgs) {
        $arguments += "--server-arg=$argument"
    }

    Write-Host "recording $Label from $Server"
    & $Harness @arguments
    if ($LASTEXITCODE -ne 0) {
        throw "protocol-harness record failed for $Label (exit $LASTEXITCODE)"
    }
}

function Read-FixtureManifest {
    param([string] $FixtureDir)
    $path = Join-Path $FixtureDir "manifest.json"
    if (-not (Test-Path -LiteralPath $path)) {
        throw "missing fixture manifest: $path"
    }
    return (Get-Content -LiteralPath $path -Raw | ConvertFrom-Json)
}

function ConvertTo-Decibels {
    param([double] $Linear)
    if ($Linear -le 0.0) {
        return [double]::NegativeInfinity
    }
    return 20.0 * [Math]::Log10($Linear)
}

# Decode the captured raw PCM and return full-scale normalised level statistics.
function Get-PcmStatistics {
    param(
        [string] $FixtureDir,
        $Manifest
    )

    $pcmPath = Join-Path $FixtureDir $Manifest.pcm.file
    if (-not (Test-Path -LiteralPath $pcmPath)) {
        throw "missing PCM capture: $pcmPath"
    }
    $bytes = [System.IO.File]::ReadAllBytes($pcmPath)
    $encoding = [int] $Manifest.audio_format.encoding
    $channels = [int] $Manifest.audio_format.channels
    if ($channels -le 0) {
        throw "capture has no channels"
    }

    switch ($encoding) {
        1 { $bytesPerSample = 4 } # ENCODING_PCM_FLOAT
        2 { $bytesPerSample = 1 } # ENCODING_PCM_8BIT (unsigned)
        3 { $bytesPerSample = 2 } # ENCODING_PCM_16BIT
        4 { $bytesPerSample = 3 } # ENCODING_PCM_24BIT (packed)
        5 { $bytesPerSample = 4 } # ENCODING_PCM_32BIT
        default { throw "unknown encoding value $encoding" }
    }

    $frameBytes = $bytesPerSample * $channels
    $frames = [Math]::Floor($bytes.Length / $frameBytes)
    $sumSquares = [double] 0.0
    $peak = [double] 0.0
    $samples = [long] 0

    for ($frame = 0; $frame -lt $frames; $frame++) {
        $frameBase = $frame * $frameBytes
        for ($channel = 0; $channel -lt $channels; $channel++) {
            $offset = $frameBase + ($channel * $bytesPerSample)
            $value = [double] 0.0
            switch ($encoding) {
                1 {
                    $value = [BitConverter]::ToSingle($bytes, $offset)
                }
                2 {
                    # WAVE_FORMAT_PCM 8-bit is unsigned, like Android's PCM_8BIT.
                    $value = ([double] $bytes[$offset] - 128.0) / 128.0
                }
                3 {
                    $value = [BitConverter]::ToInt16($bytes, $offset) / 32768.0
                }
                4 {
                    $raw = ([int] $bytes[$offset]) -bor
                           (([int] $bytes[$offset + 1]) -shl 8) -bor
                           (([int] $bytes[$offset + 2]) -shl 16)
                    if (($raw -band 0x800000) -ne 0) {
                        $raw = $raw - 0x1000000
                    }
                    $value = $raw / 8388608.0
                }
                5 {
                    $value = [BitConverter]::ToInt32($bytes, $offset) / 2147483648.0
                }
            }
            $sumSquares += $value * $value
            $magnitude = [Math]::Abs($value)
            if ($magnitude -gt $peak) {
                $peak = $magnitude
            }
            $samples++
        }
    }

    $rms = [double] 0.0
    if ($samples -gt 0) {
        $rms = [Math]::Sqrt($sumSquares / $samples)
    }

    return [pscustomobject] @{
        Bytes = $bytes.Length
        Frames = $frames
        Samples = $samples
        Rms = $rms
        Peak = $peak
        RmsDb = ConvertTo-Decibels $rms
        PeakDb = ConvertTo-Decibels $peak
    }
}

function Get-PacketStatistics {
    param($Manifest)

    $lengths = @($Manifest.pcm.segment_lengths | ForEach-Object { [int] $_ })
    if ($lengths.Count -eq 0) {
        throw "capture contains no UDP datagrams"
    }
    $maximum = ($lengths | Measure-Object -Maximum).Maximum
    $minimum = ($lengths | Measure-Object -Minimum).Minimum
    $alignedPayload = [int] $Manifest.mtu.aligned_payload_bytes
    $fullSize = @($lengths | Where-Object { $_ -eq $alignedPayload }).Count

    return [pscustomobject] @{
        Count = $lengths.Count
        Minimum = [int] $minimum
        Maximum = [int] $maximum
        FullSize = $fullSize
        AlignedPayload = $alignedPayload
        TotalBytes = [int] $Manifest.pcm.total_bytes
    }
}

function Format-Db {
    param([double] $Decibels)
    if ([double]::IsNegativeInfinity($Decibels)) {
        return "-inf"
    }
    return ("{0:F2}" -f $Decibels)
}

function Assert-FormatMatches {
    param($OldManifest, $NewManifest)

    $oldFormat = $OldManifest.audio_format
    $newFormat = $NewManifest.audio_format
    $fields = @("encoding_name", "channels", "sample_rate", "block_align")
    $mismatches = @()
    foreach ($field in $fields) {
        $oldValue = $oldFormat.$field
        $newValue = $newFormat.$field
        if ($oldValue -ne $newValue) {
            $mismatches += "$field old=$oldValue new=$newValue"
        }
    }
    if ($mismatches.Count -gt 0) {
        throw ("AudioFormat mismatch: " + ($mismatches -join "; "))
    }

    if ($OldManifest.control.get_format_response.hex -ne
        $NewManifest.control.get_format_response.hex) {
        throw "GET_FORMAT response bytes differ between the old and new servers"
    }

    Write-Host ("AudioFormat: {0} {1} ch {2} Hz block_align={3} (match)" -f `
        $newFormat.encoding_name, $newFormat.channels, $newFormat.sample_rate, `
        $newFormat.block_align)
    Write-Host "GET_FORMAT response bytes: identical"
}

function Assert-LevelsMatch {
    param($OldStats, $NewStats, [double] $ToleranceDb)

    $oldRms = $OldStats.RmsDb
    $newRms = $NewStats.RmsDb
    $oldPeak = $OldStats.PeakDb
    $newPeak = $NewStats.PeakDb

    Write-Host ("PCM: old {0} bytes RMS={1} dBFS peak={2} dBFS" -f `
        $OldStats.Bytes, (Format-Db $oldRms), (Format-Db $oldPeak))
    Write-Host ("PCM: new {0} bytes RMS={1} dBFS peak={2} dBFS" -f `
        $NewStats.Bytes, (Format-Db $newRms), (Format-Db $newPeak))

    $bothSilent = [double]::IsNegativeInfinity($oldRms) -and
                  [double]::IsNegativeInfinity($newRms)
    if ($bothSilent) {
        Write-Host "PCM levels: both recordings are silent (match)"
    }
    else {
        $rmsDelta = [Math]::Abs($oldRms - $newRms)
        $peakDelta = [Math]::Abs($oldPeak - $newPeak)
        if ([double]::IsInfinity($rmsDelta) -or $rmsDelta -gt $ToleranceDb) {
            throw (("PCM RMS differs by {0} dB (tolerance {1} dB); ensure the same tone " +
                "plays into the same endpoint for both recordings") -f `
                (Format-Db $rmsDelta), $ToleranceDb)
        }
        if ([double]::IsInfinity($peakDelta) -or $peakDelta -gt $ToleranceDb) {
            throw ("PCM peak differs by {0} dB (tolerance {1} dB)" -f `
                (Format-Db $peakDelta), $ToleranceDb)
        }
        Write-Host ("PCM levels: RMS delta {0} dB, peak delta {1} dB (within {2} dB)" -f `
            (Format-Db $rmsDelta), (Format-Db $peakDelta), $ToleranceDb)
    }

    if ($OldStats.Bytes -gt 0) {
        $byteDelta = [Math]::Abs($OldStats.Bytes - $NewStats.Bytes) / $OldStats.Bytes
        Write-Host (("PCM bytes delta: {0:P1} (the harness stops after the same datagram " +
            "count, so a small difference is expected)") -f $byteDelta)
        if ($byteDelta -gt 0.10) {
            Write-Warning ("PCM byte count differs by more than 10%: old={0} new={1}" -f `
                $OldStats.Bytes, $NewStats.Bytes)
        }
    }
}

function Write-PacketStatistics {
    param($OldPackets, $NewPackets)

    Write-Host ("UDP datagrams: old {0} (min {1}, max {2}, full-size {3}/{4})" -f `
        $OldPackets.Count, $OldPackets.Minimum, $OldPackets.Maximum, `
        $OldPackets.FullSize, $OldPackets.Count)
    Write-Host ("UDP datagrams: new {0} (min {1}, max {2}, full-size {3}/{4})" -f `
        $NewPackets.Count, $NewPackets.Minimum, $NewPackets.Maximum, `
        $NewPackets.FullSize, $NewPackets.Count)

    if ($OldPackets.Maximum -gt $OldPackets.AlignedPayload) {
        throw "old server emitted a datagram above the aligned MTU payload"
    }
    if ($NewPackets.Maximum -gt $NewPackets.AlignedPayload) {
        throw "new server emitted a datagram above the aligned MTU payload"
    }
    if ($OldPackets.FullSize -eq 0 -or $NewPackets.FullSize -eq 0) {
        Write-Warning "one side has no full-size datagram; MTU segmentation is unproven"
    }
}

function Invoke-SpikeMeasure {
    param([string] $Spike, [string] $WorkDirectory)

    if ([string]::IsNullOrWhiteSpace($Spike)) {
        return
    }
    $arguments = @(
        "measure",
        "--endpoint", $Endpoint,
        "--encoding", $Encoding,
        "--channels", "$Channels",
        "--sample-rate", "$SampleRate",
        "--seconds", "$SpikeSeconds",
        "--cycles", "10"
    )
    Write-Host "running wasapi-spike measure for the new capture path"
    $output = & $Spike @arguments 2>&1
    $output | Set-Content -LiteralPath (Join-Path $WorkDirectory "new-wasapi-measure.txt")
    $output | ForEach-Object { Write-Host $_ }
    if ($LASTEXITCODE -ne 0) {
        Write-Warning "wasapi-spike measure failed (exit $LASTEXITCODE)"
        return
    }
    $counterLines = @($output | Where-Object {
        $_ -match "glitches:|start-stop:|index gaps:|flags:"
    })
    if ($counterLines.Count -gt 0) {
        Write-Host "capture-side counters (new WASAPI path):"
        foreach ($line in $counterLines) {
            Write-Host ("  {0}" -f $line)
        }
    }
}

try {
    $OldServer = Resolve-ToolPath $OldServer "old server"
    $NewServer = Resolve-ToolPath $NewServer "new server"
    $Harness = Resolve-ToolPath $Harness "protocol harness"

    if ([string]::IsNullOrWhiteSpace($WorkDir)) {
        $WorkDir = Join-Path (Split-Path -Parent $PSScriptRoot) "target\ab-windows"
    }
    New-Item -ItemType Directory -Path $WorkDir -Force | Out-Null

    $oldDir = Join-Path $WorkDir "old"
    $newDir = Join-Path $WorkDir "new"

    Invoke-Record $OldServer $oldDir $OldServerArgs "old-as-cmd"
    Invoke-Record $NewServer $newDir $NewServerArgs "new-as-cmd"

    $oldManifest = Read-FixtureManifest $oldDir
    $newManifest = Read-FixtureManifest $newDir

    Assert-FormatMatches $oldManifest $newManifest

    $oldPcm = Get-PcmStatistics $oldDir $oldManifest
    $newPcm = Get-PcmStatistics $newDir $newManifest
    Assert-LevelsMatch $oldPcm $newPcm $LevelToleranceDb

    $oldPackets = Get-PacketStatistics $oldManifest
    $newPackets = Get-PacketStatistics $newManifest
    Write-PacketStatistics $oldPackets $newPackets

    Invoke-SpikeMeasure $SpikeExe $WorkDir

    Write-Host ""
    Write-Host "A/B result: PASS"
    Write-Host "artifacts: $WorkDir"
    exit 0
}
catch {
    Write-Error $_
    exit 1
}
