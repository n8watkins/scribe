; Custom NSIS installer hooks for Scribe.
;
; PREINSTALL: kill Scribe's leftover whisper-server.exe before overwriting files.
; The resident whisper backend (bin/windows/whisper-server.exe) loads
; ggml-*.dll; if a previous run exited non-gracefully (crash, force-kill, an
; interrupted update) the child can orphan and keep those DLLs locked, which
; makes the next install fail with "Error opening file for writing:
; ...\bin\windows\ggml-base.dll". Killing it here lets the overwrite succeed.
; Match the executable's full path. Killing by image name would also terminate
; an unrelated whisper.cpp server owned by another application or the user.
; Failures are ignored because there is normally nothing to stop.
!macro NSIS_HOOK_PREINSTALL
  nsExec::ExecToLog `powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "& { param([string] $$target) Get-CimInstance Win32_Process | Where-Object { $$_.Name -ieq 'whisper-server.exe' -and $$_.ExecutablePath -and [IO.Path]::GetFullPath($$_.ExecutablePath) -ieq [IO.Path]::GetFullPath($$target) } | ForEach-Object { Stop-Process -Id $$_.ProcessId -Force -ErrorAction SilentlyContinue } }" "$INSTDIR\bin\windows\whisper-server.exe"`
  Sleep 300
!macroend
