# Renders speech clips for the end-to-end test with the Windows speech
# synthesizer (no audio device needed): 16 kHz, 16-bit mono WAV files.
# -Manifest is a JSON array of { "text": ..., "path": ... }.
param([Parameter(Mandatory = $true)][string]$Manifest)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Speech

$clips = Get-Content $Manifest -Raw | ConvertFrom-Json
$synth = New-Object System.Speech.Synthesis.SpeechSynthesizer
$voices = @($synth.GetInstalledVoices() | Where-Object { $_.Enabled })
if ($voices.Count -eq 0) { throw "No speech synthesis voices are installed" }
Write-Host "Voice: $($voices[0].VoiceInfo.Name)"
$synth.SelectVoice($voices[0].VoiceInfo.Name)
$synth.Rate = -1
$format = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(
  16000,
  [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen,
  [System.Speech.AudioFormat.AudioChannel]::Mono
)
foreach ($clip in $clips) {
  $synth.SetOutputToWaveFile($clip.path, $format)
  $synth.Speak($clip.text)
  $synth.SetOutputToNull()
  if ((Get-Item $clip.path).Length -lt 8000) { throw "The clip for '$($clip.text)' came out empty" }
}
$synth.Dispose()
