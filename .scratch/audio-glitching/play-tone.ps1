Add-Type -AssemblyName System.Windows.Forms
$player = New-Object System.Media.SoundPlayer 'C:\Users\huangcy2\Documents\workspaces\audio-share\.scratch\audio-glitching\tone-48k-stereo.wav'
$player.PlayLooping()
Start-Sleep -Seconds 7200
