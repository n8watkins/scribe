# Hands-on checks of the Slint trial with real input (SendInput), during the
# owner's hands-off window. Saves and restores his clipboard, runs under a
# watchdog, never presses Alt, stops on the first surprise.
param([string]$Dir = "C:\Users\natha\AppData\Local\Temp\slint-trial\checks", [string]$Exe = "C:\Users\natha\AppData\Local\Temp\slint-trial\scribe-slint-trial.exe", [string]$FramesExe = "C:\Users\natha\AppData\Local\Temp\slint-trial\target\release\scribe-slint-trial.exe", [string]$Only = "")
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Windows.Forms, System.Drawing, UIAutomationClient, UIAutomationTypes
Add-Type @"
using System; using System.Runtime.InteropServices; using System.Text; using System.Collections.Generic;
public static class U {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int dx, dy; public uint data, flags, time; public IntPtr extra; }
  [StructLayout(LayoutKind.Sequential)] struct KEYBDINPUT { public ushort vk, scan; public uint flags, time; public IntPtr extra; }
  [StructLayout(LayoutKind.Explicit)] struct INPUT { [FieldOffset(0)] public uint type; [FieldOffset(8)] public MOUSEINPUT mi; [FieldOffset(8)] public KEYBDINPUT ki; }
  [DllImport("user32.dll")] static extern uint SendInput(uint n, INPUT[] i, int size);
  [DllImport("user32.dll")] public static extern int GetSystemMetrics(int i);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string cls, string title);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr SendMessage(IntPtr h, uint msg, IntPtr w, StringBuilder l);
  [DllImport("user32.dll")] static extern IntPtr SendMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
  // The text of the form's only child (the TextBox), by WM_GETTEXT.
  public static string ChildText(IntPtr form) {
    IntPtr c = GetWindow(form, 5);
    int n = SendMessage(c, 0x000E, IntPtr.Zero, IntPtr.Zero).ToInt32();
    var s = new StringBuilder(n + 1); SendMessage(c, 0x000D, (IntPtr)(n + 1), s); return s.ToString();
  }
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
  [DllImport("user32.dll")] public static extern IntPtr GetWindowLongPtrW(IntPtr h, int i);
  [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr h, uint cmd);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint f);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr p);
  public delegate bool EnumProc(IntPtr h, IntPtr p);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out int v, int n);
  static int vx, vy, vw, vh;
  static void Desk() { vx = GetSystemMetrics(76); vy = GetSystemMetrics(77); vw = GetSystemMetrics(78); vh = GetSystemMetrics(79); }
  static INPUT M(uint flags, int x, int y, uint data) {
    Desk();
    var i = new INPUT(); i.type = 0; i.mi.flags = flags; i.mi.data = data;
    if ((flags & 0x8000) != 0) { i.mi.dx = (int)Math.Round((x - vx) * 65535.0 / (vw - 1)); i.mi.dy = (int)Math.Round((y - vy) * 65535.0 / (vh - 1)); }
    return i;
  }
  static void Send(params INPUT[] a) { SendInput((uint)a.Length, a, Marshal.SizeOf(typeof(INPUT))); }
  public static void Move(int x, int y) { Send(M(0x8000 | 0x4000 | 0x0001, x, y, 0)); }
  public static void Down() { Send(M(0x0002, 0, 0, 0)); }
  public static void Up() { Send(M(0x0004, 0, 0, 0)); }
  public static void RDown() { Send(M(0x0008, 0, 0, 0)); }
  public static void RUp() { Send(M(0x0010, 0, 0, 0)); }
  public static void Wheel(int delta) { Send(M(0x0800, 0, 0, (uint)delta)); }
  static INPUT K(ushort vk, ushort scan, uint flags) { var i = new INPUT(); i.type = 1; i.ki.vk = vk; i.ki.scan = scan; i.ki.flags = flags; return i; }
  public static void Char(char c) { Send(K(0, c, 0x0004), K(0, c, 0x0004 | 0x0002)); }
  public static void Key(ushort vk) { Send(K(vk, 0, 0), K(vk, 0, 0x0002)); }
  public static void Combo(ushort mod, ushort vk) { Send(K(mod, 0, 0), K(vk, 0, 0), K(vk, 0, 0x0002), K(mod, 0, 0x0002)); }
  public static uint FgPid() { uint p; GetWindowThreadProcessId(GetForegroundWindow(), out p); return p; }
  public static string Title(IntPtr h) { var s = new StringBuilder(256); GetWindowText(h, s, 256); return s.ToString(); }
  public static List<IntPtr> Windows(uint pid) {
    var l = new List<IntPtr>();
    EnumWindows((h, p) => { uint q; GetWindowThreadProcessId(h, out q); if (q == pid) l.Add(h); return true; }, IntPtr.Zero);
    return l;
  }
  // Alt+Tab and taskbar rule: visible, unowned, not cloaked, and either an app window or not a tool window.
  public static bool InAltTab(IntPtr h) {
    if (!IsWindowVisible(h)) return false;
    long ex = GetWindowLongPtrW(h, -20).ToInt64();
    int cloaked; DwmGetWindowAttribute(h, 14, out cloaked, 4);
    if (cloaked != 0) return false;
    if ((ex & 0x40000) != 0) return true;
    if (GetWindow(h, 4) != IntPtr.Zero) return false;
    if ((ex & 0x80) != 0) return false;
    if ((ex & 0x08000000) != 0) return false;
    return true;
  }
}
"@
[void][U]::SetProcessDPIAware()
New-Item -ItemType Directory -Force $Dir | Out-Null
$shots = Join-Path $Dir "shots"; New-Item -ItemType Directory -Force $shots | Out-Null
$R = [ordered]@{ started = (Get-Date -Format s); steps = [ordered]@{}; surprises = @() }
$pidFile = Join-Path $Dir "pids.txt"; Set-Content $pidFile ""
function Log($t) { $l = "{0:HH:mm:ss} {1}" -f (Get-Date), $t; Write-Output $l; Add-Content (Join-Path $Dir "checks.log") $l }
function Pause-Ms([int]$ms) { Start-Sleep -Milliseconds $ms }
function Surprise($what) { $R.surprises += $what; Log "SURPRISE: $what"; throw "stopped: $what" }

# ----- clipboard: save first -----
$clipKind = "empty"
if ([Windows.Forms.Clipboard]::ContainsImage()) { [Windows.Forms.Clipboard]::GetImage().Save((Join-Path $Dir "clipboard.png")); $clipKind = "image" }
elseif ([Windows.Forms.Clipboard]::ContainsFileDropList()) { Set-Content (Join-Path $Dir "clipboard-files.txt") @([Windows.Forms.Clipboard]::GetFileDropList()); $clipKind = "files" }
elseif ([Windows.Forms.Clipboard]::ContainsText()) { [IO.File]::WriteAllText((Join-Path $Dir "clipboard.txt"), [Windows.Forms.Clipboard]::GetText()); $clipKind = "text" }
$R.clipboardSaved = $clipKind
Log "clipboard saved ($clipKind)"
$watch = Start-Process powershell -WindowStyle Hidden -PassThru -ArgumentList "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", (Join-Path $Dir "watchdog.ps1"), "-Seconds", "780", "-Dir", $Dir
$cursor0 = New-Object U+POINT; [void][U]::GetCursorPos([ref]$cursor0)

# ----- helpers -----
$trial = $null; $sink = $null
function Start-Trial([string[]]$a, [string]$name) {
  $st = Join-Path $Dir "status-$name.jsonl"; Remove-Item $st -ErrorAction SilentlyContinue
  $p = Start-Process $Exe -ArgumentList (@("--status=$st", "--exit-after=240000") + $a) -PassThru
  Add-Content $pidFile $p.Id
  $script:trial = $p; $script:status = $st
  return $p
}
function Stop-Trial { if ($script:trial -and -not $script:trial.HasExited) { Stop-Process -Id $script:trial.Id -Force; [void]$script:trial.WaitForExit(5000) }; $script:trial = $null }
function Events { if (Test-Path $script:status) { @(Get-Content $script:status | Where-Object { $_ } | ForEach-Object { $_ | ConvertFrom-Json }) } else { @() } }
function Has-Event($n) { [bool](Events | Where-Object { $_.event -eq $n }) }
function Wait-For([scriptblock]$c, [int]$ms, [string]$what) { $end = (Get-Date).AddMilliseconds($ms); while ((Get-Date) -lt $end) { if (& $c) { return }; Pause-Ms 50 }; Surprise "timed out waiting for $what" }
function Find-Win([uint32]$procId, [string]$title) { foreach ($h in [U]::Windows($procId)) { if ([U]::IsWindowVisible($h) -and [U]::Title($h) -eq $title) { return $h } }; return [IntPtr]::Zero }
function Rect($h) { $r = New-Object U+RECT; [void][U]::GetWindowRect($h, [ref]$r); return $r }
function Fg { $h = [U]::GetForegroundWindow(); return [ordered]@{ pid = [U]::FgPid(); title = [U]::Title($h) } }
function Sink-Text { return [U]::ChildText($script:sinkH) }
function Click([int]$x, [int]$y) { [U]::Move($x, $y); Pause-Ms 60; [U]::Down(); Pause-Ms 60; [U]::Up(); Pause-Ms 80 }
$typed = New-Object Text.StringBuilder
function Type-Chunk([string]$s) { foreach ($c in $s.ToCharArray()) { [U]::Char($c); Pause-Ms 12 }; [void]$typed.Append($s) }
function Check-Fg([string]$step) {
  $f = Fg
  if ($f.pid -ne $script:sinkProc.Id) { Surprise "after '$step' the foreground is '$($f.title)' (pid $($f.pid)), not the sink" }
  return $f
}
function Capture-Screen($r, [string]$name) {
  $w = $r.R - $r.L; $h = $r.B - $r.T
  $b = New-Object Drawing.Bitmap $w, $h; $g = [Drawing.Graphics]::FromImage($b); $g.CopyFromScreen($r.L, $r.T, 0, 0, (New-Object Drawing.Size $w, $h)); $g.Dispose()
  if ($name) { $b.Save((Join-Path $shots "$name.png")) }
  return $b
}
function Shot-Window($h, [string]$name) {
  $r = Rect $h; $w = $r.R - $r.L; $hh = $r.B - $r.T
  $b = New-Object Drawing.Bitmap $w, $hh; $g = [Drawing.Graphics]::FromImage($b); $dc = $g.GetHdc(); [void][U]::PrintWindow($h, $dc, 2); $g.ReleaseHdc($dc); $g.Dispose()
  $b.Save((Join-Path $shots "$name.png")); $b.Dispose()
}
function Pill { $h = Find-Win $script:trial.Id "Scribe Pill (trial)"; return $h }

try {
  # ----- the sink window, focused by a click inside it -----
  $script:sinkProc = Start-Process powershell -WindowStyle Hidden -PassThru -ArgumentList "-NoProfile", "-STA", "-ExecutionPolicy", "Bypass", "-File", (Join-Path $Dir "sink.ps1")
  Add-Content $pidFile $script:sinkProc.Id
  Wait-For { $script:sinkH = Find-Win $script:sinkProc.Id "Trial input sink"; $script:sinkH -ne [IntPtr]::Zero } 15000 "the sink window"
  $sr = Rect $script:sinkH
  Click ($sr.L + 200) ($sr.T + 150)
  Pause-Ms 300
  $f = Check-Fg "focus the sink"
  Log "sink focused"

  # ===== A1: show and hide while typing =====
  if (-not $Only -or $Only -match "A") {
    Start-Trial @("--renderer=software", "--pill=cycle", "--no-clipboard=1") "A1-cycle" | Out-Null
    $fgLog = @()
    $end = (Get-Date).AddSeconds(10)
    $n = 0
    while ((Get-Date) -lt $end) { Type-Chunk ("show-hide {0:00} the quick brown fox. " -f $n); $n++; $fgLog += (Check-Fg "typing during show/hide").title }
    $ev = Events
    $R.steps.A1 = [ordered]@{ shown = @($ev | Where-Object event -eq "pill-shown").Count; hidden = @($ev | Where-Object event -eq "pill-hidden").Count; chunks = $n; foregroundAlwaysSink = $true }
    Stop-Trial
    Log "A1 shown $($R.steps.A1.shown) hidden $($R.steps.A1.hidden)"

    # ===== A2: drag, click to stop, Copy, paste =====
    Start-Trial @("--renderer=software", "--pill=text") "A2" | Out-Null
    Wait-For { Has-Event "pill-shown" } 10000 "the pill"
    Pause-Ms 400
    $ph = Pill; $r0 = Rect $ph
    $R.steps.A2 = [ordered]@{ pillStart = "$($r0.L),$($r0.T) to $($r0.R),$($r0.B)"; dpiStart = [U]::GetDpiForWindow($ph) }
    Type-Chunk "before drag. "; Check-Fg "pill shown" | Out-Null
    # Taskbar and Alt+Tab, by style and the shell's own rule
    $ex = [int64][U]::GetWindowLongPtrW($ph, -20)
    $R.steps.A2.exStyle = ("0x{0:X}" -f $ex)
    $R.steps.A2.inAltTabByRule = [U]::InAltTab($ph)
    $tb = [Windows.Automation.AutomationElement]::FromHandle([U]::FindWindow("Shell_TrayWnd", $null))
    $names = @($tb.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition) | ForEach-Object { $_.Current.Name } | Where-Object { $_ })
    $R.steps.A2.taskbarNamesWithScribe = @($names | Where-Object { $_ -match "Scribe" })
    # Real drag: press in the middle, move 120 px left and 200 px up in steps, release.
    $cx = [int](($r0.L + $r0.R) / 2); $cy = [int](($r0.T + $r0.B) / 2)
    [U]::Move($cx, $cy); Pause-Ms 100; [U]::Down(); Pause-Ms 80
    for ($i = 1; $i -le 20; $i++) { [U]::Move($cx - 6 * $i, $cy - 10 * $i); Pause-Ms 20 }
    [U]::Up(); Pause-Ms 300
    $r1 = Rect $ph
    $R.steps.A2.afterDrag = "$($r1.L),$($r1.T)"
    $R.steps.A2.dragged = Has-Event "pill-dragged"
    $R.steps.A2.stoppedByDrag = Has-Event "pill-stop"
    Type-Chunk "after drag. "; Check-Fg "drag" | Out-Null
    if (-not $R.steps.A2.dragged -or $R.steps.A2.stoppedByDrag) { Surprise "drag: moved=$($R.steps.A2.dragged) stopped=$($R.steps.A2.stoppedByDrag)" }

    # ===== D: onto the other monitor and back =====
    $screens = [Windows.Forms.Screen]::AllScreens
    $R.steps.D = [ordered]@{ monitors = @($screens | ForEach-Object { "$($_.Bounds.X),$($_.Bounds.Y) $($_.Bounds.Width)x$($_.Bounds.Height) primary=$($_.Primary)" }) }
    $other = $screens | Where-Object { -not $_.Primary } | Select-Object -First 1
    if ($other) {
      $r1 = Rect $ph; $sx = [int](($r1.L + $r1.R) / 2); $sy = [int](($r1.T + $r1.B) / 2)
      $tx = $other.Bounds.X + [int]($other.Bounds.Width / 2); $ty = $other.Bounds.Y + [int]($other.Bounds.Height / 2)
      [U]::Move($sx, $sy); Pause-Ms 100; [U]::Down(); Pause-Ms 80
      for ($i = 1; $i -le 30; $i++) { [U]::Move([int]($sx + ($tx - $sx) * $i / 30), [int]($sy + ($ty - $sy) * $i / 30)); Pause-Ms 20 }
      [U]::Up(); Pause-Ms 400
      $r2 = Rect $ph
      $R.steps.D.onOther = "$($r2.L),$($r2.T) to $($r2.R),$($r2.B)"; $R.steps.D.dpiOther = [U]::GetDpiForWindow($ph); $R.steps.D.sizeOther = "$($r2.R - $r2.L)x$($r2.B - $r2.T)"
      Type-Chunk "on the other monitor. "; Check-Fg "drag to other monitor" | Out-Null
      $cx2 = [int](($r2.L + $r2.R) / 2); $cy2 = [int](($r2.T + $r2.B) / 2)
      [U]::Move($cx2, $cy2); Pause-Ms 100; [U]::Down(); Pause-Ms 80
      for ($i = 1; $i -le 30; $i++) { [U]::Move([int]($cx2 + ($sx - $cx2) * $i / 30), [int]($cy2 + ($sy - $cy2) * $i / 30)); Pause-Ms 20 }
      [U]::Up(); Pause-Ms 400
      $r3 = Rect $ph
      $R.steps.D.back = "$($r3.L),$($r3.T) to $($r3.R),$($r3.B)"; $R.steps.D.dpiBack = [U]::GetDpiForWindow($ph)
      $R.steps.D.stillRecording = -not (Has-Event "pill-stop")
      Type-Chunk "back again. "; Check-Fg "drag back" | Out-Null
    } else { $R.steps.D.note = "one monitor" }

    # ===== A2: real click stops recording =====
    $r4 = Rect $ph
    Click ([int](($r4.L + $r4.R) / 2)) ([int](($r4.T + $r4.B) / 2))
    Type-Chunk "after click. "; Check-Fg "click to stop" | Out-Null
    Wait-For { @(Events | Where-Object { $_.event -eq "pill-status" -and $_.status -eq "Ready" }).Count -gt 0 } 5000 "the confirmation"
    $R.steps.A2.clickStopped = Has-Event "pill-stop"
    Pause-Ms 400
    # ===== A2 / C: Copy =====
    $pe = [Windows.Automation.AutomationElement]::FromHandle($ph)
    $btn = $pe.FindFirst([Windows.Automation.TreeScope]::Descendants, (New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty, "Copy")))
    if (-not $btn) { Surprise "no Copy button in the pill's automation tree" }
    $br = $btn.Current.BoundingRectangle
    $R.steps.C = [ordered]@{ copyButton = "$([int]$br.X),$([int]$br.Y) $([int]$br.Width)x$([int]$br.Height)" }
    Click ([int]($br.X + $br.Width / 2)) ([int]($br.Y + $br.Height / 2))
    Pause-Ms 300
    $R.steps.C.copyEvent = (Events | Where-Object event -eq "pill-copy" | Select-Object -First 1)
    $clip = [Windows.Forms.Clipboard]::GetText()
    $R.steps.C.clipboard = $clip
    $live = "So the plan for tomorrow is to finish the assessment and send the notes to the team, then look at the installer again because the upgrade test failed twice last week and I want to know why before Friday"
    $R.steps.C.clipboardIsTheTranscript = ($clip.Length -gt 0 -and $live.StartsWith($clip)) -or $clip -eq "So the plan for tomorrow is to finish the assessment and send the notes to the team."
    $f = Check-Fg "Copy click"
    $sinkBefore = Sink-Text
    $R.steps.A2.typedArrivedInOrder = ($sinkBefore -eq $typed.ToString())
    if (-not $R.steps.A2.typedArrivedInOrder) { $R.steps.A2.typed = $typed.ToString(); $R.steps.A2.sink = $sinkBefore; Surprise "typed text does not match what the sink holds" }
    [U]::Combo(0x11, 0x56); Pause-Ms 300
    $sinkAfter = Sink-Text
    $R.steps.C.pastedIntoSink = $sinkAfter.EndsWith($clip) -and $sinkAfter.Length -eq ($sinkBefore.Length + $clip.Length)
    Log ("A2/C/D done: " + ($R.steps | ConvertTo-Json -Depth 4 -Compress))
    Stop-Trial
  }

  # ===== B: corners over light and dark, both renderers =====
  if (-not $Only -or $Only -match "B") {
    $R.steps.B = [ordered]@{}
    foreach ($rend in @("software", "skia-software")) {
      Start-Trial @("--renderer=$rend", "--pill=bar", "--hold=1", "--no-clipboard=1") "B-$rend" | Out-Null
      Wait-For { Has-Event "pill-shown" } 10000 "the pill"
      Pause-Ms 500
      $ph = Pill; $pr = Rect $ph
      $res = [ordered]@{}
      foreach ($bgc in @(@("light", [Drawing.Color]::FromArgb(240, 240, 240)), @("dark", [Drawing.Color]::FromArgb(16, 16, 16)))) {
        # A plain backdrop window of our own behind the pill (the pill is topmost).
        $bgf = New-Object Windows.Forms.Form
        $bgf.FormBorderStyle = "None"; $bgf.StartPosition = "Manual"; $bgf.ShowInTaskbar = $false
        $bgf.Location = New-Object Drawing.Point ($pr.L - 30), ($pr.T - 30); $bgf.Size = New-Object Drawing.Size ($pr.R - $pr.L + 60), ($pr.B - $pr.T + 60)
        $bgf.BackColor = $bgc[1]; $bgf.Show(); [Windows.Forms.Application]::DoEvents(); Pause-Ms 400; [Windows.Forms.Application]::DoEvents()
        $b = Capture-Screen $pr "B-$rend-$($bgc[0])"
        $corner = $b.GetPixel(1, 1); $corner2 = $b.GetPixel($b.Width - 2, $b.Height - 2); $mid = $b.GetPixel([int]($b.Width / 2), 2)
        $want = $bgc[1]
        $ok = ([math]::Abs($corner.R - $want.R) -le 3) -and ([math]::Abs($corner2.R - $want.R) -le 3)
        $res[$bgc[0]] = [ordered]@{ cornerTopLeft = "$($corner.R),$($corner.G),$($corner.B)"; cornerBottomRight = "$($corner2.R),$($corner2.G),$($corner2.B)"; topMiddle = "$($mid.R),$($mid.G),$($mid.B)"; backdrop = "$($want.R),$($want.G),$($want.B)"; cornersShowBackdrop = $ok }
        $b.Dispose(); $bgf.Close(); $bgf.Dispose()
      }
      $R.steps.B[$rend] = $res
      Stop-Trial
    }
    Log ("B: " + ($R.steps.B | ConvertTo-Json -Depth 4 -Compress))
    Click ($sr.L + 200) ($sr.T + 150); Pause-Ms 200
  }

  # ===== E: tray =====
  if (-not $Only -or $Only -match "E") {
    Start-Trial @("--renderer=software", "--no-clipboard=1") "E" | Out-Null
    Wait-For { Has-Event "start" } 10000 "start"
    Pause-Ms 1500
    $root = [Windows.Automation.AutomationElement]::RootElement
    $tb = [Windows.Automation.AutomationElement]::FromHandle([U]::FindWindow("Shell_TrayWnd", $null))
    $icon = @($tb.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition) | Where-Object { $_.Current.Name -match "Scribe Slint trial" }) | Select-Object -First 1
    $R.steps.E = [ordered]@{}
    if (-not $icon -or $icon.Current.IsOffscreen -or $icon.Current.BoundingRectangle.Width -le 0) {
      $R.steps.E.note = "the tray icon is not on the taskbar (hidden overflow area); not rearranged, not clicked"
    } else {
      $ir = $icon.Current.BoundingRectangle; $ix = [int]($ir.X + $ir.Width / 2); $iy = [int]($ir.Y + $ir.Height / 2)
      $R.steps.E.icon = "$ix,$iy"
      [U]::Move($ix, $iy); Pause-Ms 80; [U]::RDown(); Pause-Ms 60; [U]::RUp(); Pause-Ms 700
      $item = $root.FindFirst([Windows.Automation.TreeScope]::Descendants, (New-Object Windows.Automation.AndCondition((New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty, "Show floating pill")), (New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::MenuItem)))))
      if (-not $item) { [U]::Key(0x1B); Surprise "the tray menu did not show 'Show floating pill'" }
      try { $R.steps.E.checkBefore = $item.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Current.ToggleState.ToString() } catch { $R.steps.E.checkBefore = "no toggle pattern" }
      $mr = $item.Current.BoundingRectangle
      Click ([int]($mr.X + $mr.Width / 2)) ([int]($mr.Y + $mr.Height / 2)); Pause-Ms 500
      $R.steps.E.pillEnabledEvent = (Events | Where-Object event -eq "pill-enabled" | Select-Object -Last 1)
      [U]::Move($ix, $iy); Pause-Ms 80; [U]::RDown(); Pause-Ms 60; [U]::RUp(); Pause-Ms 700
      $item = $root.FindFirst([Windows.Automation.TreeScope]::Descendants, (New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty, "Show floating pill")))
      try { $R.steps.E.checkAfter = $item.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Current.ToggleState.ToString() } catch { $R.steps.E.checkAfter = "no toggle pattern" }
      [U]::Key(0x1B); Pause-Ms 300
      # Left click opens the main window.
      Click $ix $iy; Pause-Ms 1200
      $R.steps.E.mainOpened = (Events | Where-Object event -eq "main-open" | Select-Object -Last 1)
      $mh = Find-Win $script:trial.Id "Scribe Slint trial"
      $R.steps.E.mainForeground = ([U]::FgPid() -eq $script:trial.Id)
      if ($mh -ne [IntPtr]::Zero) { Shot-Window $mh "E-main-from-tray" }
    }
    Stop-Trial
    Log ("E: " + ($R.steps.E | ConvertTo-Json -Depth 4 -Compress))
  }

  # ===== F: editor typing, latency, select, copy, undo =====
  if (-not $Only -or $Only -match "F") {
    Start-Trial @("--renderer=software", "--open=editor", "--main-onscreen=1") "F" | Out-Null
    Wait-For { Has-Event "main-first-frame" } 10000 "the main window"
    Pause-Ms 800
    $mh = Find-Win $script:trial.Id "Scribe Slint trial"; $mr = Rect $mh
    $ed = [Windows.Automation.AutomationElement]::FromHandle($mh).FindFirst([Windows.Automation.TreeScope]::Descendants, (New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty, "Transcript editor")))
    $er = $ed.Current.BoundingRectangle
    # Click at the start of the first line (focuses the window and the editor).
    $kx = [int]$er.X + 14; $ky = [int]$er.Y + 14
    Click $kx $ky; Pause-Ms 300
    [U]::Key(0x24); Pause-Ms 100   # Home
    $R.steps.F = [ordered]@{ load = (Get-CimInstance Win32_Processor | Measure-Object -Property LoadPercentage -Average).Average }
    $area = New-Object U+RECT; $area.L = $kx - 8; $area.T = $ky - 9; $area.R = $kx + 400; $area.B = $ky + 9
    $lat = @(); $pattern = "abcdefghij"
    for ($i = 0; $i -lt 220; $i++) {
      $before = Capture-Screen $area $null
      $bh = $before.GetPixel(10, 9).ToArgb() ; $bs = New-Object IO.MemoryStream; $before.Save($bs, [Drawing.Imaging.ImageFormat]::Bmp); $bb = $bs.ToArray(); $before.Dispose()
      $sw = [Diagnostics.Stopwatch]::StartNew()
      [U]::Char($pattern[$i % 10])
      $changed = $false
      while ($sw.ElapsedMilliseconds -lt 500) {
        $now = Capture-Screen $area $null; $ns = New-Object IO.MemoryStream; $now.Save($ns, [Drawing.Imaging.ImageFormat]::Bmp); $nb = $ns.ToArray(); $now.Dispose()
        if (-not [Linq.Enumerable]::SequenceEqual([byte[]]$bb, [byte[]]$nb)) { $changed = $true; break }
      }
      $lat += $(if ($changed) { $sw.Elapsed.TotalMilliseconds } else { 500 })
    }
    $s = @($lat | Sort-Object)
    $R.steps.F.keys = $lat.Count
    $R.steps.F.medianMs = [math]::Round($s[[int]($s.Count / 2)], 1)
    $R.steps.F.p95Ms = [math]::Round($s[[int]($s.Count * 0.95)], 1)
    $R.steps.F.maxMs = [math]::Round($s[-1], 1)
    $R.steps.F.missed = @($lat | Where-Object { $_ -ge 500 }).Count
    $R.steps.F.loadAfter = (Get-CimInstance Win32_Processor | Measure-Object -Property LoadPercentage -Average).Average
    $vp = $null
    try { $vp = $ed.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern); $val = $vp.Current.Value; $R.steps.F.typedInText = $val.StartsWith(("abcdefghij" * 22)) } catch { $R.steps.F.typedInText = "editor has no Value pattern: $($_.Exception.Message)" }
    # Select to the end of the line and copy.
    [U]::Key(0x24); Pause-Ms 100
    [U]::Combo(0x10, 0x23); Pause-Ms 200
    [U]::Combo(0x11, 0x43); Pause-Ms 300
    $cl = [Windows.Forms.Clipboard]::GetText()
    $R.steps.F.copiedChars = $cl.Length
    $R.steps.F.copiedStartsWithTyped = $cl.StartsWith("abcdefghij")
    [U]::Key(0x23); Pause-Ms 100
    for ($u = 0; $u -lt 30; $u++) { [U]::Combo(0x11, 0x5A); Pause-Ms 40 }
    Pause-Ms 300
    if ($vp) { $val2 = $vp.Current.Value; $R.steps.F.undoRemovedTyping = -not $val2.Contains("abcdefghijabcdefghij"); $R.steps.F.lengthAfterUndo = $val2.Length }
    Shot-Window $mh "F-editor-after"
    Stop-Trial
    Click ($sr.L + 200) ($sr.T + 150); Pause-Ms 200
    Log ("F: " + ($R.steps.F | ConvertTo-Json -Depth 4 -Compress))
  }

  # ===== G: real wheel on the history list =====
  if ((-not $Only -or $Only -match "G") -and (Test-Path $FramesExe)) {
    $Exe = $FramesExe
    Start-Trial @("--renderer=software", "--open=history", "--main-onscreen=1", "--probe=frames") "G" | Out-Null
    Wait-For { Has-Event "main-first-frame" } 10000 "the main window"
    $mh = Find-Win $script:trial.Id "Scribe Slint trial"
    $li = [Windows.Automation.AutomationElement]::FromHandle($mh).FindFirst([Windows.Automation.TreeScope]::Descendants, (New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty, "Transcript history")))
    $lr = $li.Current.BoundingRectangle
    Click ([int]($lr.X + $lr.Width / 2)) ([int]($lr.Y + 30)); Pause-Ms 200
    Pause-Ms 2500
    $t0 = Get-Date
    while (((Get-Date) - $t0).TotalSeconds -lt 16) { $d = if (((Get-Date) - $t0).TotalSeconds -lt 8) { -120 } else { 120 }; [U]::Wheel($d); Pause-Ms 16 }
    Wait-For { Has-Event "probe-frames" } 10000 "frame stats"
    $R.steps.G = (Events | Where-Object event -eq "probe-frames" | Select-Object -First 1)
    $R.steps.G | Add-Member -NotePropertyName cpuLoadPercent -NotePropertyValue ((Get-CimInstance Win32_Processor | Measure-Object -Property LoadPercentage -Average).Average) -Force
    Shot-Window $mh "G-history-after-wheel"
    Stop-Trial
    Log ("G: " + ($R.steps.G | ConvertTo-Json -Depth 4 -Compress))
  }

  # ===== H: Tab and Shift+Tab, what a screen reader would be told =====
  if (-not $Only -or $Only -match "H") {
    $R.steps.H = [ordered]@{}
    foreach ($page in @("controls", "history", "editor")) {
      Start-Trial @("--renderer=software", "--open=$page", "--main-onscreen=1") "H-$page" | Out-Null
      Wait-For { Has-Event "main-first-frame" } 10000 "the main window"
      Pause-Ms 600
      $mh = Find-Win $script:trial.Id "Scribe Slint trial"; $mr = Rect $mh
      # Focus the window by clicking its blank status line (bottom left), not a control.
      Click ($mr.L + 420) ($mr.B - 22); Pause-Ms 300
      $stops = @()
      $keys = @(1..14 | ForEach-Object { "Tab" }) + @(1..4 | ForEach-Object { "Shift+Tab" })
      $k = 0
      foreach ($key in $keys) {
        if ($key -eq "Tab") { [U]::Key(0x09) } else { [U]::Combo(0x10, 0x09) }
        Pause-Ms 250
        $fe = [Windows.Automation.AutomationElement]::FocusedElement
        $c = $fe.Current
        $state = ""
        try { $state += " toggle=" + $fe.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Current.ToggleState } catch {}
        try { $state += " selected=" + $fe.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Current.IsSelected } catch {}
        try { $state += " value=" + $fe.GetCurrentPattern([Windows.Automation.RangeValuePattern]::Pattern).Current.Value } catch {}
        $nm = $c.Name; if ($nm.Length -gt 50) { $nm = $nm.Substring(0, 50) + "..." }
        $stops += ("{0} -> {1} '{2}' enabled={3}{4} pid={5}" -f $key, $c.ControlType.ProgrammaticName.Replace("ControlType.", ""), $nm, $c.IsEnabled, $state, $c.ProcessId)
        $k++
        Shot-Window $mh ("H-{0}-{1:00}" -f $page, $k)
      }
      $R.steps.H[$page] = $stops
      Stop-Trial
    }
    Log ("H: " + ($R.steps.H | ConvertTo-Json -Depth 4 -Compress))
    Click ($sr.L + 200) ($sr.T + 150); Pause-Ms 200
  }
  Log "ALL DONE"
} catch {
  $R.stoppedEarly = $_.Exception.Message
  Log "STOPPED: $($_.Exception.Message) at $($_.InvocationInfo.ScriptLineNumber): $($_.InvocationInfo.Line.Trim())"
} finally {
  Stop-Trial
  [U]::Up()
  foreach ($vk in 0x10, 0x11) { } # keys are always sent as pairs; nothing to release
  if ($script:sinkProc) { Stop-Process -Id $script:sinkProc.Id -Force -ErrorAction SilentlyContinue }
  Get-Process scribe-slint-trial -ErrorAction SilentlyContinue | Stop-Process -Force
  # Restore his clipboard.
  if ($clipKind -eq "image") { [Windows.Forms.Clipboard]::SetImage([Drawing.Image]::FromFile((Join-Path $Dir "clipboard.png"))) }
  elseif ($clipKind -eq "files") { $c = New-Object Collections.Specialized.StringCollection; Get-Content (Join-Path $Dir "clipboard-files.txt") | ForEach-Object { [void]$c.Add($_) }; [Windows.Forms.Clipboard]::SetFileDropList($c) }
  elseif ($clipKind -eq "text") { [Windows.Forms.Clipboard]::SetText([IO.File]::ReadAllText((Join-Path $Dir "clipboard.txt"))) }
  else { [Windows.Forms.Clipboard]::Clear() }
  $R.clipboardRestored = $(if ($clipKind -eq "text") { ([Windows.Forms.Clipboard]::GetText() -eq [IO.File]::ReadAllText((Join-Path $Dir "clipboard.txt"))) } else { $clipKind })
  Set-Content (Join-Path $Dir "clipboard-restored.txt") "yes"
  [U]::Move($cursor0.X, $cursor0.Y)
  Set-Content (Join-Path $Dir "done.txt") "done"
  if ($watch) { Stop-Process -Id $watch.Id -Force -ErrorAction SilentlyContinue }
  $R.leftRunning = @(Get-Process scribe-slint-trial -ErrorAction SilentlyContinue).Count
  $R.finished = (Get-Date -Format s)
  $R | ConvertTo-Json -Depth 8 | Set-Content -Encoding UTF8 (Join-Path $Dir "results.json")
  Log "clipboard restored: $($R.clipboardRestored); trial processes left: $($R.leftRunning)"
}
