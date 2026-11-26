# A text window of the checks' own (instead of Notepad, which on Windows 11 can
# open as a tab in the owner's existing Notepad window). Closes itself after 14 min.
Add-Type -AssemblyName System.Windows.Forms
$f = New-Object Windows.Forms.Form
$f.Text = "Trial input sink"
$f.StartPosition = "Manual"
$f.Location = New-Object Drawing.Point 120, 160
$f.Size = New-Object Drawing.Size 520, 300
$f.TopMost = $true
$t = New-Object Windows.Forms.TextBox
$t.Multiline = $true
$t.Dock = "Fill"
$t.Font = New-Object Drawing.Font "Consolas", 11
$f.Controls.Add($t)
$timer = New-Object Windows.Forms.Timer
$timer.Interval = 840000
$timer.Add_Tick({ $f.Close() })
$timer.Start()
# Started hidden, Windows hides the first shown window too: show it again once.
$f.Add_Shown({ $f.Visible = $false; $f.Visible = $true })
[Windows.Forms.Application]::Run($f)
