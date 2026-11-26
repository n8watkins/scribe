# Watchdog for checks.ps1: after $Seconds, kills everything the checks started,
# releases Shift, Ctrl and both mouse buttons (never Alt), and restores the
# owner's clipboard unless the checks already did.
param([int]$Seconds = 780, [string]$Dir)
Start-Sleep -Seconds $Seconds
if (Test-Path (Join-Path $Dir "done.txt")) { exit }
Add-Content (Join-Path $Dir "watchdog.txt") "fired $(Get-Date -Format s)"
Get-Content (Join-Path $Dir "pids.txt") -ErrorAction SilentlyContinue | Where-Object { $_ } | ForEach-Object { Stop-Process -Id ([int]$_) -Force -ErrorAction SilentlyContinue }
Get-Process scribe-slint-trial -ErrorAction SilentlyContinue | Stop-Process -Force
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class R {
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, int dx, int dy, uint data, UIntPtr extra);
}
"@
foreach ($vk in 0x10, 0x11, 0xA0, 0xA1, 0xA2, 0xA3) { [R]::keybd_event([byte]$vk, 0, 2, [UIntPtr]::Zero) }
[R]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
[R]::mouse_event(0x0010, 0, 0, 0, [UIntPtr]::Zero)
if (-not (Test-Path (Join-Path $Dir "clipboard-restored.txt"))) {
  Add-Type -AssemblyName System.Windows.Forms
  $t = Join-Path $Dir "clipboard.txt"; $i = Join-Path $Dir "clipboard.png"; $f = Join-Path $Dir "clipboard-files.txt"
  if (Test-Path $i) { [Windows.Forms.Clipboard]::SetImage([Drawing.Image]::FromFile($i)) }
  elseif (Test-Path $f) { $c = New-Object Collections.Specialized.StringCollection; Get-Content $f | ForEach-Object { [void]$c.Add($_) }; [Windows.Forms.Clipboard]::SetFileDropList($c) }
  elseif (Test-Path $t) { Set-Clipboard -Value ([IO.File]::ReadAllText($t)) }
  else { [Windows.Forms.Clipboard]::Clear() }
  Add-Content (Join-Path $Dir "watchdog.txt") "clipboard restored by watchdog"
}
