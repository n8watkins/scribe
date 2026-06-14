; Custom NSIS installer hooks for Scribe.
;
; PREINSTALL: kill any leftover whisper-server.exe before overwriting files.
; The resident whisper backend (bin/windows/whisper-server.exe) loads
; ggml-*.dll; if a previous run exited non-gracefully (crash, force-kill, an
; interrupted update) the child can orphan and keep those DLLs locked, which
; makes the next install fail with "Error opening file for writing:
; ...\bin\windows\ggml-base.dll". Killing it here lets the overwrite succeed.
; /T also terminates any child processes; failures are ignored (nothing to kill).
!macro NSIS_HOOK_PREINSTALL
  nsExec::Exec 'taskkill /F /T /IM whisper-server.exe'
  Sleep 300
!macroend
