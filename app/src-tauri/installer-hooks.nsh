; Custom NSIS installer hooks for Scribe.
;
; PUBLISHER CHANGE (natkins -> n8builds.dev). The installer remembers the install
; folder under HKCU\Software\<publisher>\Scribe and reads it back on the next
; install, so changing `bundle.publisher` makes it look under a new key.
; - v0.16.2 and later copies write the new key (n8builds.dev) themselves, so an
;   update from them lands in the same folder.
; - Older copies only have the old key (natkins). SCRIBE_ADOPT_OLD_INSTALL_DIR
;   reads it as a fallback when the new key is empty, so an update never makes
;   a second copy in the default folder. A silent or passive run (the updater)
;   adopts it in PREINSTALL; an interactive run adopts it in .onGUIInit, before
;   the folder page shows, so the user sees (and may change) the right folder.
;   A folder passed with /D= always wins.
; - POSTINSTALL then removes the old key, and POSTUNINSTALL removes both keys
;   unless the run is an update.
!define SCRIBE_OLD_MANUFACTURER "natkins"

; Sets $INSTDIR to the folder the old publisher key recorded, when no folder
; was passed with /D=, the new key records none, and the old folder exists.
!macro SCRIBE_ADOPT_OLD_INSTALL_DIR
  ${If} $CmdLineInstallDir == ""
    Push $0
    Push $1
    ReadRegStr $0 HKCU "${MANUPRODUCTKEY}" ""
    ${If} $0 == ""
      ReadRegStr $1 HKCU "Software\${SCRIBE_OLD_MANUFACTURER}\${PRODUCTNAME}" ""
      ${If} $1 != ""
      ${AndIf} ${FileExists} "$1\*.*"
        StrCpy $INSTDIR $1
      ${EndIf}
    ${EndIf}
    Pop $1
    Pop $0
  ${EndIf}
!macroend

; Interactive runs: adopt the old folder before the first page shows.
; Defined inside the finish page hook because that is the only hook at file
; scope placed after the variables it reads are declared.
!macro NSIS_HOOK_PREFINISHPAGE
  Function .onGUIInit
    !insertmacro SCRIBE_ADOPT_OLD_INSTALL_DIR
  FunctionEnd
!macroend

!macro NSIS_HOOK_POSTINSTALL
  DeleteRegKey HKCU "Software\${SCRIBE_OLD_MANUFACTURER}\${PRODUCTNAME}"
  DeleteRegKey /ifempty HKCU "Software\${SCRIBE_OLD_MANUFACTURER}"
!macroend

; The default uninstall keeps the folder key so a reinstall goes to the same
; place; remove it (and any old-publisher key) on a real uninstall. An update
; skips this because the new installer rewrites the key right after.
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    DeleteRegKey HKCU "${MANUPRODUCTKEY}"
    DeleteRegKey /ifempty HKCU "${MANUKEY}"
    DeleteRegKey HKCU "Software\${SCRIBE_OLD_MANUFACTURER}\${PRODUCTNAME}"
    DeleteRegKey /ifempty HKCU "Software\${SCRIBE_OLD_MANUFACTURER}"
  ${EndIf}
!macroend

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
  ; Only a silent or passive run skips the folder page; an interactive run
  ; already adopted the old folder in .onGUIInit (see below).
  ${If} ${Silent}
  ${OrIf} $PassiveMode = 1
    !insertmacro SCRIBE_ADOPT_OLD_INSTALL_DIR
    SetOutPath $INSTDIR
  ${EndIf}
  nsExec::ExecToLog `powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "& { param([string] $$target) Get-CimInstance Win32_Process | Where-Object { $$_.Name -ieq 'whisper-server.exe' -and $$_.ExecutablePath -and [IO.Path]::GetFullPath($$_.ExecutablePath) -ieq [IO.Path]::GetFullPath($$target) } | ForEach-Object { Stop-Process -Id $$_.ProcessId -Force -ErrorAction SilentlyContinue } }" "$INSTDIR\bin\windows\whisper-server.exe"`
  Sleep 300
!macroend
