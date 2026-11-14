; Sage was called Dots up to version 0.4.0. This file makes an install of Dots become Sage
; in place: the same folder and the same program file, so the entry that starts it at sign-in,
; a taskbar pin, the firewall rule for the phone page and "go back to the version before" all
; keep working. Its data is in that folder too and is not touched.
;
; The installer template runs NSIS_HOOK_PREINSTALL at the start of the install section, after
; it has made the new folder, and NSIS_HOOK_POSTINSTALL at its end. In an update it makes no
; shortcuts, so the old ones are renamed here.

!define DOTS_NAME "Dots"
!define DOTS_UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Dots"
; Where the Dots installer wrote its folder (its publisher was "dots").
!define DOTS_PRODUCTKEY "Software\dots\Dots"
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"
!define RUN_APPROVED_KEY "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"

Var DotsDir

!macro NSIS_HOOK_PREINSTALL
  StrCpy $DotsDir ""
  ReadRegStr $R8 HKCU "${DOTS_PRODUCTKEY}" ""
  ${If} $R8 != ""
  ${AndIf} ${FileExists} "$R8\${MAINBINARYNAME}.exe"
    StrCpy $DotsDir $R8
    ${If} $INSTDIR != $R8
      ; The folder the template has just made is empty: nothing of Sage is in it yet.
      SetOutPath $R8
      RMDir "$INSTDIR"
      StrCpy $INSTDIR $R8
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ${If} $DotsDir != ""
    ; A shortcut keeps what the person did with it (its place, a pin): it is renamed, and
    ; one the template made beside it in a first install goes.
    ${If} ${FileExists} "$SMPROGRAMS\${DOTS_NAME}.lnk"
      Delete "$SMPROGRAMS\${PRODUCTNAME}.lnk"
      Rename "$SMPROGRAMS\${DOTS_NAME}.lnk" "$SMPROGRAMS\${PRODUCTNAME}.lnk"
    ${EndIf}
    ${If} ${FileExists} "$DESKTOP\${DOTS_NAME}.lnk"
      Delete "$DESKTOP\${PRODUCTNAME}.lnk"
      Rename "$DESKTOP\${DOTS_NAME}.lnk" "$DESKTOP\${PRODUCTNAME}.lnk"
    ${EndIf}
    ; The entry that starts it at sign-in takes the new name, so it still starts at the next
    ; sign-in if Sage is not opened before then. Sage rewrites it from its setting when it starts.
    ReadRegStr $R8 HKCU "${RUN_KEY}" "${DOTS_NAME}"
    ${If} $R8 != ""
      WriteRegStr HKCU "${RUN_KEY}" "${PRODUCTNAME}" $R8
    ${EndIf}
    DeleteRegValue HKCU "${RUN_KEY}" "${DOTS_NAME}"
    DeleteRegValue HKCU "${RUN_APPROVED_KEY}" "${DOTS_NAME}"
    ; Windows' list of installed apps shows Sage only.
    DeleteRegKey HKCU "${DOTS_UNINSTKEY}"
    DeleteRegKey HKCU "${DOTS_PRODUCTKEY}"
    DeleteRegKey /ifempty HKCU "Software\dots"
  ${EndIf}
!macroend
