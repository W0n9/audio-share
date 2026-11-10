# Generate a deterministic 10 s 440 Hz stereo 48 kHz 16-bit WAV.
param(
    [string]$Path = "$PSScriptRoot\tone-48k-stereo.wav",
    [double]$Frequency = 440.0,
    [int]$SampleRate = 48000,
    [int]$Channels = 2,
    [double]$Seconds = 10.0,
    [double]$Amplitude = 0.25
)

$frames = [int]($SampleRate * $Seconds)
$peak = [int](32767 * $Amplitude)
$dataBytes = $frames * $Channels * 2

$stream = [System.IO.File]::Create($Path)
$writer = [System.IO.BinaryWriter]::new($stream)
try {
    $writer.Write([System.Text.Encoding]::ASCII.GetBytes("RIFF"))
    $writer.Write([int](36 + $dataBytes))
    $writer.Write([System.Text.Encoding]::ASCII.GetBytes("WAVE"))
    $writer.Write([System.Text.Encoding]::ASCII.GetBytes("fmt "))
    $writer.Write([int]16)
    $writer.Write([int16]1)                       # PCM
    $writer.Write([int16]$Channels)
    $writer.Write([int]$SampleRate)
    $writer.Write([int]($SampleRate * $Channels * 2))
    $writer.Write([int16]($Channels * 2))
    $writer.Write([int16]16)
    $writer.Write([System.Text.Encoding]::ASCII.GetBytes("data"))
    $writer.Write([int]$dataBytes)
    for ($i = 0; $i -lt $frames; $i++) {
        $value = [int16]([math]::Round($peak * [math]::Sin(2.0 * [math]::PI * $Frequency * $i / $SampleRate)))
        for ($c = 0; $c -lt $Channels; $c++) {
            $writer.Write($value)
        }
    }
}
finally {
    $writer.Dispose()
    $stream.Dispose()
}
Write-Output $Path
