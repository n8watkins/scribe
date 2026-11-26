# Measures the Slint trial on Windows without sending keys, clicking, touching
# the clipboard or taking the focus (adapted from Pulse's scripts/measure.ps1).
# Every wait is bounded, there is an overall limit, the exe quits itself
# (--exit-after), and only the processes this script started are stopped.
#
# For each renderer (skia-software, software):
#   idle        tray only, nothing created                     memory, modules
#   pill        the pill animating (bar, text growing)         memory, style, picture
#   window      main window open on the history list           memory, picture, modules
#   closed      10 s after the main window is destroyed        memory (trimmed and not)
#   probes      list scroll frames, editor open, window open   timings
#   cycle       the pill shown, animated and hidden for 30 s   foreground never ours
#   click       a click posted to the pill (our window only)   stops recording, no focus
#   placement   saved position on monitor 2, off-screen        where the pill lands
# plus pictures of every page and a UI Automation dump of the main window.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\measure.ps1 `
#     -Exe target\release\scribe-slint-trial.exe -Out C:\path\to\out
param(
  [Parameter(Mandatory)] [string]$Exe,
  [Parameter(Mandatory)] [string]$Out,
  [string[]]$Renderers = @("skia-software", "software"),
  [int]$SettleSeconds = 15,
  [int]$LimitSeconds = 1500,
  [switch]$Probes
)
$ErrorActionPreference = "Stop"
$exe = (Resolve-Path $Exe).Path
New-Item -ItemType Directory -Force $Out | Out-Null
$out = (Resolve-Path $Out).Path
$started = Get-Date
$report = [ordered]@{ exe = $exe; startedAt = $started.ToString("s"); errors = @(); runs = [ordered]@{} }
$pidFile = Join-Path $out "started-pids.txt"
Set-Content $pidFile ""

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @"
using System; using System.Runtime.InteropServices; using System.Text; using System.Collections.Generic;
public static class W {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  public delegate bool EnumProc(IntPtr h, IntPtr p);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr p);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint f);
  [DllImport("user32.dll")] public static extern IntPtr GetWindowLongPtrW(IntPtr h, int i);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  public static List<string[]> Windows(uint pid) {
    var list = new List<string[]>();
    EnumWindows((h, p) => {
      uint q; GetWindowThreadProcessId(h, out q);
      if (q == pid) { var s = new StringBuilder(256); GetWindowText(h, s, 256); list.Add(new string[] { h.ToInt64().ToString(), s.ToString(), IsWindowVisible(h) ? "1" : "0" }); }
      return true;
    }, IntPtr.Zero);
    return list;
  }
  public static uint ForegroundPid() { uint pid; GetWindowThreadProcessId(GetForegroundWindow(), out pid); return pid; }
}
"@
[void][W]::SetProcessDPIAware()

function Log($text) {
  $line = "{0:HH:mm:ss} {1}" -f (Get-Date), $text; Write-Output $line
  # A reader holding the file open must not stop the run.
  for ($i = 0; $i -lt 5; $i++) { try { Add-Content (Join-Path $out "measure.log") $line -ErrorAction Stop; break } catch { Start-Sleep -Milliseconds 100 } }
}
function Check-Time { if (((Get-Date) - $started).TotalSeconds -gt $LimitSeconds) { throw "over the time limit of $LimitSeconds s" } }

# ----- start and stop -----
$all_started = New-Object System.Collections.Generic.List[int]
$script:runNo = 0
function Start-Trial([string]$name, [string[]]$extra, [int]$exitAfterMs) {
  $script:runNo++
  $status = Join-Path $out ("status-{0:00}-{1}.jsonl" -f $script:runNo, $name)
  Remove-Item $status -ErrorAction SilentlyContinue
  $args_ = @("--status=$status", "--exit-after=$exitAfterMs", "--no-clipboard=1") + $extra
  $p = Start-Process $exe -ArgumentList $args_ -PassThru
  $all_started.Add($p.Id); Add-Content $pidFile $p.Id
  return [pscustomobject]@{ proc = $p; status = $status; name = $name; fgViolations = 0; fgSamples = 0; fgOthers = 0 }
}
function Stop-Trial($t) {
  if (-not $t) { return }
  if (-not $t.proc.HasExited) {
    Stop-Process -Id $t.proc.Id -Force -ErrorAction SilentlyContinue
    try { Wait-Process -Id $t.proc.Id -Timeout 10 -ErrorAction SilentlyContinue } catch {}
  }
}

# ----- the status file -----
function Events($t) {
  if (-not (Test-Path $t.status)) { return @() }
  return @(Get-Content $t.status | Where-Object { $_ } | ForEach-Object { $_ | ConvertFrom-Json })
}
function Find-Event($t, [string]$name) { return (Events $t | Where-Object { $_.event -eq $name } | Select-Object -Last 1) }
# Waits, polling the foreground window every 50 ms; a foreground window of the
# trial process counts as a violation (the trial must never take the focus).
function Watch([object]$t, [double]$seconds, [scriptblock]$until = $null) {
  $end = (Get-Date).AddSeconds($seconds)
  while ((Get-Date) -lt $end) {
    Check-Time
    $fp = [W]::ForegroundPid()
    $t.fgSamples++
    if ($fp -eq [uint32]$t.proc.Id) { $t.fgViolations++ }
    if ($until -and (& $until)) { return $true }
    if ($t.proc.HasExited -and -not $until) { return $false }
    Start-Sleep -Milliseconds 50
  }
  if ($until) { throw "timed out in $($t.name)" }
  return $false
}
function Wait-Event($t, [string]$name, [int]$seconds) {
  [void](Watch $t $seconds { [bool](Find-Event $t $name) })
  return Find-Event $t $name
}

# ----- memory: Task Manager's "Memory" (private working set) and private bytes -----
function Memory([int]$id) {
  # "Process V2" names instances "<name>:<pid>", so one counter is read, not all.
  $samples = $null
  $counter = "\Process V2(scribe-slint-trial:$id)\Working Set - Private"
  for ($try = 0; $try -lt 10 -and -not $samples; $try++) {
    try { $samples = (Get-Counter -Counter $counter -ErrorAction Stop).CounterSamples } catch { $lastErr = $_.Exception.Message; Start-Sleep -Milliseconds 300 }
  }
  if (-not $samples) { throw "the Working Set - Private counter could not be read: $lastErr" }
  $pws = $samples[0].CookedValue
  $pr = Get-Process -Id $id -ErrorAction SilentlyContinue
  if ($null -eq $pws -or -not $pr) { throw "process $id not found" }
  return [ordered]@{ privateWorkingSetMb = [math]::Round($pws / 1MB, 1); privateBytesMb = [math]::Round($pr.PrivateMemorySize64 / 1MB, 1); workingSetMb = [math]::Round($pr.WorkingSet64 / 1MB, 1); threads = $pr.Threads.Count; handles = $pr.HandleCount }
}
function Memory-Series($t, [int]$count = 5, [double]$gap = 2) {
  $list = @()
  for ($i = 0; $i -lt $count; $i++) {
    if ($i -gt 0) { [void](Watch $t $gap) }
    try { $list += , (Memory $t.proc.Id) } catch { $report.errors += "memory sample skipped: $($_.Exception.Message)" }
  }
  if (-not $list.Count) { throw "no memory sample could be taken" }
  $avg = { param($k) [math]::Round((($list | ForEach-Object { $_[$k] } | Measure-Object -Average).Average), 1) }
  $max = { param($k) [math]::Round((($list | ForEach-Object { $_[$k] } | Measure-Object -Maximum).Maximum), 1) }
  return [ordered]@{ meanPrivateWorkingSetMb = (& $avg "privateWorkingSetMb"); maxPrivateWorkingSetMb = (& $max "privateWorkingSetMb"); meanPrivateBytesMb = (& $avg "privateBytesMb"); samples = $list }
}

# ----- modules: no graphics driver DLLs expected on the CPU renderers -----
# Vendor driver DLLs (AMD, NVIDIA, Intel user-mode drivers) are what a GPU
# renderer loads; the system graphics API DLLs can be plain imports.
$driverPattern = '^(atiu|atig|atid|atic|atim|amdxc|amdihk|amdvlk|amdenc|amdlvr|amfrt|amdadlx|nvwgf|nvd3d|nvoglv|nvldumd|nvcuda|nvapi|nvgpucomp|igd|igc|igvk|igxe|intelocl)'
$apiPattern = '^(vulkan-1|opengl32|d3d9|d3d10|d3d11|d3d12|d3d12core|d3dcompiler|dxgi|dxcore|libegl|libglesv2|dcomp)'
function Modules($t, [string]$label) {
  $mods = @((Get-Process -Id $t.proc.Id).Modules | ForEach-Object { $_.ModuleName } | Sort-Object -Unique)
  Set-Content (Join-Path $out "modules-$label.txt") $mods
  $drv = @($mods | Where-Object { $_.ToLower() -match $driverPattern })
  $api = @($mods | Where-Object { $_.ToLower() -match $apiPattern })
  return [ordered]@{ count = $mods.Count; gpuDrivers = $drv; graphicsApi = $api }
}

# ----- windows and pictures -----
function Find-Window([int]$procId, [string]$title) {
  $pick = [W]::Windows([uint32]$procId) | Where-Object { $_[1] -eq $title -and $_[2] -eq "1" } | Select-Object -First 1
  if ($pick) { return [IntPtr][int64]$pick[0] }
  return [IntPtr]::Zero
}
function Visible-Windows($t) { return @([W]::Windows([uint32]$t.proc.Id) | Where-Object { $_[2] -eq "1" } | ForEach-Object { $_[1] }) }
function Capture($h, [string]$name) {
  try { return (Capture-Inner $h $name) } catch {
    $r = New-Object W+RECT; [void][W]::GetWindowRect($h, [ref]$r)
    $msg = "capture $name failed (hwnd $h, rect $($r.L),$($r.T),$($r.R),$($r.B)): $($_.Exception.Message)"
    $report.errors += $msg; Log $msg
    return [ordered]@{ error = $msg }
  }
}
function Capture-Inner($h, [string]$name) {
  $r = New-Object W+RECT; [void][W]::GetWindowRect($h, [ref]$r)
  $w = $r.R - $r.L; $hh = $r.B - $r.T
  $bmp = New-Object Drawing.Bitmap $w, $hh, ([Drawing.Imaging.PixelFormat]::Format32bppArgb)
  $g = [Drawing.Graphics]::FromImage($bmp); $g.Clear([Drawing.Color]::Magenta); $dc = $g.GetHdc()
  $ok = [W]::PrintWindow($h, $dc, 2); $g.ReleaseHdc($dc); $g.Dispose()
  $png = Join-Path $out "$name.png"
  $bmp.Save($png, [Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
  return [ordered]@{ png = $png; printWindowOk = $ok; rect = "$($r.L),$($r.T) $($w)x$($hh)"; dpi = [W]::GetDpiForWindow($h) }
}
function Style($h) {
  $ex = [int64][W]::GetWindowLongPtrW($h, -20)
  return [ordered]@{ exStyle = ("0x{0:X}" -f $ex); toolWindow = (($ex -band 0x80) -ne 0); noActivate = (($ex -band 0x08000000) -ne 0); topmost = (($ex -band 0x8) -ne 0); appWindow = (($ex -band 0x40000) -ne 0) }
}

# ----- UI Automation: names, roles and states, read from outside (no input) -----
function Uia-Dump($h, [string]$name) {
  $root = [System.Windows.Automation.AutomationElement]::FromHandle($h)
  $walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker
  $lines = New-Object System.Collections.Generic.List[string]
  $script:uiaCount = 0
  function Walk($el, [int]$depth) {
    if ($depth -gt 14 -or $script:uiaCount -gt 300) { return }
    $script:uiaCount++
    $c = $el.Current
    $extra = ""
    try { $tp = $el.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern); $extra += " toggle=" + $tp.Current.ToggleState } catch {}
    try { $sp = $el.GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern); $extra += " selected=" + $sp.Current.IsSelected } catch {}
    try { $vp = $el.GetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern); $extra += " value=" + $vp.Current.Value } catch {}
    try { $ip = $el.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern); if ($ip) { $extra += " invoke" } } catch {}
    $nm = $c.Name; if ($nm.Length -gt 60) { $nm = $nm.Substring(0, 60) + "..." }
    $lines.Add(("{0}{1} '{2}' enabled={3} focusable={4}{5}" -f ("  " * $depth), $c.ControlType.ProgrammaticName.Replace("ControlType.", ""), $nm, $c.IsEnabled, $c.IsKeyboardFocusable, $extra))
    $child = $walker.GetFirstChild($el)
    while ($child) { Walk $child ($depth + 1); $child = $walker.GetNextSibling($child) }
  }
  Walk $root 0
  $file = Join-Path $out "uia-$name.txt"
  Set-Content -Encoding UTF8 $file $lines
  return [ordered]@{ file = $file; nodes = $lines.Count }
}

$watch = Start-Process powershell -WindowStyle Hidden -PassThru -ArgumentList "-NoProfile", "-Command", "Start-Sleep $($LimitSeconds + 60); Get-Content '$pidFile' | Where-Object { `$_ } | ForEach-Object { Stop-Process -Id ([int]`$_) -Force -ErrorAction SilentlyContinue }"
$t = $null
try {
  Log "START (limit $LimitSeconds s)"
  $report.monitors = @([System.Windows.Forms.Screen]::AllScreens | ForEach-Object { "$($_.DeviceName) $($_.Bounds.X),$($_.Bounds.Y) $($_.Bounds.Width)x$($_.Bounds.Height) primary=$($_.Primary)" }) 2>$null
  foreach ($r in $Renderers) {
    # (PowerShell names are case-insensitive: never call this $res.)
    $res = [ordered]@{}
    $report.runs[$r] = $res
    $rr = @("--renderer=$r")

    # idle: tray only
    Log "[$r] idle"
    $t = Start-Trial "idle-$r" $rr 120000
    [void](Wait-Event $t "start" 20)
    [void](Watch $t $SettleSeconds)
    $res.idle = Memory-Series $t
    $res.idle.modules = Modules $t "idle-$r"
    $res.idle.visibleWindows = Visible-Windows $t
    $res.idle.foregroundViolations = $t.fgViolations
    Log ("[$r] idle: private WS {0} MB, private bytes {1} MB, GPU drivers [{2}], graphics API DLLs [{3}]" -f $res.idle.meanPrivateWorkingSetMb, $res.idle.meanPrivateBytesMb, ($res.idle.modules.gpuDrivers -join ","), ($res.idle.modules.graphicsApi -join ","))
    Stop-Trial $t; $t = $null

    # pill animating: bar, then text (growing words, bars)
    foreach ($scenario in @("bar", "text")) {
      Log "[$r] pill $scenario"
      $t = Start-Trial "pill-$scenario-$r" ($rr + @("--pill=$scenario", "--hold=1")) 120000
      [void](Wait-Event $t "pill-shown" 20)
      $h = [IntPtr]::Zero
      [void](Watch $t 10 { $script:h = Find-Window $t.proc.Id "Scribe Pill (trial)"; $script:h -ne [IntPtr]::Zero })
      [void](Watch $t $SettleSeconds)
      $e = [ordered]@{}
      $e.memory = Memory-Series $t
      $e.modules = Modules $t "pill-$scenario-$r"
      $e.style = Style $h
      $e.picture = Capture $h "printwindow-pill-$scenario-$r"
      $e.visibleWindows = Visible-Windows $t
      $e.foregroundViolations = $t.fgViolations
      $e.foregroundSamples = $t.fgSamples
      $res["pill_$scenario"] = $e
      Log ("[$r] pill {0}: private WS {1} MB (max {2}), style {3}, fg violations {4}/{5}" -f $scenario, $e.memory.meanPrivateWorkingSetMb, $e.memory.maxPrivateWorkingSetMb, $e.style.exStyle, $t.fgViolations, $t.fgSamples)
      Stop-Trial $t; $t = $null
    }

    # window open on history, then destroyed; trimmed (default) and not
    foreach ($trim in @("trim", "notrim")) {
      Log "[$r] window ($trim)"
      $extra = @("--open=history", "--close-after=75000")
      if ($trim -eq "notrim") { $extra += "--no-trim=1" }
      $t = Start-Trial "window-$trim-$r" ($rr + $extra) 180000
      $ff = Wait-Event $t "main-first-frame" 30
      $h = [IntPtr]::Zero
      [void](Watch $t 10 { $script:h = Find-Window $t.proc.Id "Scribe Slint trial"; $script:h -ne [IntPtr]::Zero })
      [void](Watch $t $SettleSeconds)
      $e = [ordered]@{ firstFrameMs = $ff.ms }
      if ($trim -eq "trim") {
        $e.picture = Capture $h "printwindow-history-$r"
        $e.modules = Modules $t "window-$r"
      }
      $e.open = Memory-Series $t
      # The open samples only count if the window was still open for all of them.
      $e.openSamplesBeforeClose = -not [bool](Find-Event $t "main-closed")
      [void](Wait-Event $t "main-closed" 90)
      [void](Watch $t 10)
      $e.closed = Memory-Series $t
      $e.visibleWindowsAfterClose = Visible-Windows $t
      $e.allWindowsAfterClose = @([W]::Windows([uint32]$t.proc.Id) | ForEach-Object { $_[1] })
      $e.foregroundViolations = $t.fgViolations
      $res["window_$trim"] = $e
      Log ("[$r] window {0}: open {1} MB, 10 s after close {2} MB, first frame {3} ms, windows left [{4}]" -f $trim, $e.open.meanPrivateWorkingSetMb, $e.closed.meanPrivateWorkingSetMb, $ff.ms, ($e.visibleWindowsAfterClose -join ","))
      Stop-Trial $t; $t = $null
    }

    # probes: frame timings need the window on a monitor (off screen, Windows
    # paints it only when something changes), so they run only with -Probes,
    # which puts the main window on screen. Never run it while the owner works.
    $probeList = if ($Probes) { @("scroll", "editor", "open") } else { @() }
    foreach ($probe in $probeList) {
      Log "[$r] probe $probe"
      $extra = @("--probe=$probe", "--main-onscreen=1")
      if ($probe -ne "open") { $extra += "--open=history" }
      $t = Start-Trial "probe-$probe-$r" ($rr + $extra) 120000
      $ev = Wait-Event $t "probe-$probe" 90
      $res["probe_$probe"] = $ev
      $res["probe_$probe"] | Add-Member -NotePropertyName foregroundViolations -NotePropertyValue $t.fgViolations -Force
      Log ("[$r] probe {0}: {1}" -f $probe, ($ev | ConvertTo-Json -Compress))
      Stop-Trial $t; $t = $null
    }
  }

  $r = $Renderers[0]
  $rr = @("--renderer=$r")

  # pages: pictures and accessibility tree
  foreach ($page in @("history", "editor", "input", "controls")) {
    Log "page $page"
    $t = Start-Trial "page-$page" ($rr + @("--open=$page")) 60000
    [void](Wait-Event $t "main-first-frame" 30)
    $h = [IntPtr]::Zero
    [void](Watch $t 10 { $script:h = Find-Window $t.proc.Id "Scribe Slint trial"; $script:h -ne [IntPtr]::Zero })
    [void](Watch $t 2)
    $report["page_$page"] = [ordered]@{ picture = (Capture $h "page-$page"); uia = (Uia-Dump $h $page); foregroundViolations = $t.fgViolations }
    Stop-Trial $t; $t = $null
  }

  # cycle: shown, animated and hidden for 30 s, foreground polled every 50 ms
  Log "cycle"
  $t = Start-Trial "cycle" ($rr + @("--pill=cycle")) 40000
  [void](Wait-Event $t "pill-shown" 20)
  [void](Watch $t 30)
  $ev = Events $t
  $report.cycle = [ordered]@{ shown = @($ev | Where-Object { $_.event -eq "pill-shown" }).Count; hidden = @($ev | Where-Object { $_.event -eq "pill-hidden" }).Count; foregroundSamples = $t.fgSamples; foregroundViolations = $t.fgViolations }
  Log ("cycle: shown {0}, hidden {1}, fg violations {2}/{3}" -f $report.cycle.shown, $report.cycle.hidden, $t.fgViolations, $t.fgSamples)
  Stop-Trial $t; $t = $null

  # click: a press and release posted to the pill itself
  Log "click"
  $t = Start-Trial "click" ($rr + @("--pill=bar", "--fake-click=3000")) 40000
  $after = Wait-Event $t "fake-click-after" 20
  [void](Watch $t 2)
  $report.click = [ordered]@{ click = (Find-Event $t "fake-click"); after = $after; stop = (Find-Event $t "pill-stop"); statuses = @(Events $t | Where-Object { $_.event -eq "pill-status" } | ForEach-Object { $_.status }); foregroundViolations = $t.fgViolations }
  Log ("click: status after {0}, fg unchanged {1}, violations {2}" -f $after.status, $after.foregroundUnchanged, $t.fgViolations)
  Stop-Trial $t; $t = $null

  # placement: saved position on the second monitor, and the off-screen sentinel
  $report.placement = [ordered]@{}
  foreach ($case in @(@("monitor1", "--monitor=1"), @("offscreen", "--pill-pos=-32000,-32000"), @("monitor0", "--monitor=0"))) {
    Log "placement $($case[0])"
    $t = Start-Trial "place-$($case[0])" ($rr + @("--pill=bar", "--hold=1", $case[1])) 30000
    $placed = Wait-Event $t "pill-placed" 20
    $h = [IntPtr]::Zero
    [void](Watch $t 10 { $script:h = Find-Window $t.proc.Id "Scribe Pill (trial)"; $script:h -ne [IntPtr]::Zero })
    [void](Watch $t 1.5)
    $rect = New-Object W+RECT; [void][W]::GetWindowRect($h, [ref]$rect)
    $report.placement[$case[0]] = [ordered]@{ placed = $placed; rect = "$($rect.L),$($rect.T) to $($rect.R),$($rect.B)"; picture = (Capture $h "printwindow-pill-$($case[0])"); foregroundViolations = $t.fgViolations }
    Stop-Trial $t; $t = $null
  }
  Log "END"
} catch {
  Log "STOPPED: $($_.Exception.Message)"
  $report.errors += $_.Exception.Message
} finally {
  if ($t) { Stop-Trial $t }
  foreach ($id in $all_started) { Stop-Process -Id $id -Force -ErrorAction SilentlyContinue }
  if ($watch) { Stop-Process -Id $watch.Id -Force -ErrorAction SilentlyContinue }
  $report.leftRunning = @($all_started | Where-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue })
  $report.seconds = [int]((Get-Date) - $started).TotalSeconds
  $report | ConvertTo-Json -Depth 10 | Set-Content (Join-Path $out "report.json")
}
