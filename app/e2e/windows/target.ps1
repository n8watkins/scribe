# A stand-in app for the end-to-end test: one window with a multi-line text
# box, in its own process like a real app. Writes its window handles to
# -InfoPath once shown, so the test can focus it and read its text.
# Run with Windows PowerShell in STA mode:
#   powershell -NoProfile -STA -ExecutionPolicy Bypass -File target.ps1 -Title T -InfoPath p -Left 20 -Top 20
param(
  [Parameter(Mandatory = $true)][string]$Title,
  [Parameter(Mandatory = $true)][string]$InfoPath,
  [int]$Left = 20,
  [int]$Top = 20
)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Windows.Forms, System.Drawing

$form = New-Object System.Windows.Forms.Form
$form.Text = $Title
$form.StartPosition = [System.Windows.Forms.FormStartPosition]::Manual
$form.Left = $Left
$form.Top = $Top
$form.Width = 480
$form.Height = 240
# Stays above Scribe's dashboard so a focusing click always lands here.
$form.TopMost = $true

$box = New-Object System.Windows.Forms.TextBox
$box.Multiline = $true
$box.Dock = [System.Windows.Forms.DockStyle]::Fill
$box.Font = New-Object System.Drawing.Font("Segoe UI", 11)
$box.HideSelection = $false
$form.Controls.Add($box)

$form.Add_Shown({
  $info = @{ form = [int64]$form.Handle; box = [int64]$box.Handle; pid = $PID } | ConvertTo-Json -Compress
  # Written whole and renamed into place, so the reader never sees half a file.
  Set-Content -Path "$InfoPath.tmp" -Value $info -Encoding ascii
  Move-Item -Force "$InfoPath.tmp" $InfoPath
})

[System.Windows.Forms.Application]::Run($form)
