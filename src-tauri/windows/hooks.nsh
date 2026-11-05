; xmu-sports NSIS hooks — paired with windows/installer.nsi
; App data and install identity stay at xmu-sports when display branding changes.

!ifndef XMU_STARTUP_RUN_KEY
!define XMU_STARTUP_RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"
!endif
!ifndef XMU_STARTUP_APPROVED_KEY
!define XMU_STARTUP_APPROVED_KEY "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"
!endif

!define XMU_HTTP_BUILD_ROOT "${__FILEDIR__}\..\resources\http-booking"

; Python distributions use different names for the same support libraries.
; Remove a known old name only when the new bundle does not ship that file.
!macro XMU_REMOVE_RETIRED_HTTP_FILE relative_path
  !if /FileExists "${XMU_HTTP_BUILD_ROOT}\${relative_path}"
  !else
    ${If} ${FileExists} "$INSTDIR\http-booking\${relative_path}"
      Delete "$INSTDIR\http-booking\${relative_path}"
    ${EndIf}
  !endif
!macroend

!macro XMU_UPDATE_STARTUP
  ReadRegStr $R8 HKCU "${XMU_STARTUP_RUN_KEY}" "${PRODUCTNAME}"
  ${If} "$R8" != ""
    WriteRegStr HKCU "${XMU_STARTUP_RUN_KEY}" "${PRODUCTNAME}" '$\"$INSTDIR\${MAINBINARYNAME}.exe$\" --autostart'
  ${EndIf}
!macroend

!macro XMU_REMOVE_STARTUP
  ReadRegStr $R8 HKCU "${XMU_STARTUP_RUN_KEY}" "${PRODUCTNAME}"
  ${If} "$R8" == '$\"$INSTDIR\${MAINBINARYNAME}.exe$\" --autostart'
    DeleteRegValue HKCU "${XMU_STARTUP_RUN_KEY}" "${PRODUCTNAME}"
    DeleteRegValue HKCU "${XMU_STARTUP_APPROVED_KEY}" "${PRODUCTNAME}"
  ${EndIf}
!macroend

!macro XMU_RENAME_SHORTCUT directory previous_name
!if "${PRODUCTNAME}" != "${previous_name}"
  Push $R8
  !insertmacro IsShortcutTarget "${directory}\${previous_name}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
  Pop $R8
  ${If} $R8 = 1
    ClearErrors
    CreateShortcut "${directory}\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    ${IfNot} ${Errors}
      !insertmacro SetLnkAppUserModelId "${directory}\${PRODUCTNAME}.lnk"
      Delete "${directory}\${previous_name}.lnk"
    ${EndIf}
  ${EndIf}
  Pop $R8
!endif
!macroend

!macro XMU_MIGRATE_SHORTCUT directory
  !insertmacro XMU_RENAME_SHORTCUT "${directory}" "${INTERNALNAME}"
  !insertmacro XMU_RENAME_SHORTCUT "${directory}" "XMU Sport"
!macroend

; Program resources only. In old installations INSTDIR can equal the data root:
; requests/tasks/runs next to the worker must survive this cleanup.
!macro XMU_REMOVE_OBSOLETE_COMPONENTS
  ${If} "$INSTDIR" != ""
  ${AndIf} ${FileExists} "$INSTDIR\${MAINBINARYNAME}.exe"
    ClearErrors
    ; Only the legacy hook runtime needs wholesale replacement. Current HTTP
    ; dependencies are overlaid by NSIS so an upgrade no longer deletes them all.
    ${If} ${FileExists} "$INSTDIR\http-booking\xmu-http-booking.exe"
      ${If} ${FileExists} "$INSTDIR\http-booking\_internal\frida.dll"
      ${OrIf} ${FileExists} "$INSTDIR\http-booking\_internal\frida\_frida.pyd"
        RMDir /r "$INSTDIR\http-booking\_internal"
      ${EndIf}
      !insertmacro XMU_REMOVE_RETIRED_HTTP_FILE "_internal\onnxruntime\capi\onnxruntime.dll"
      !insertmacro XMU_REMOVE_RETIRED_HTTP_FILE "_internal\PIL\_avif.cp312-win_amd64.pyd"
      !insertmacro XMU_REMOVE_RETIRED_HTTP_FILE "_internal\libcrypto-3.dll"
      !insertmacro XMU_REMOVE_RETIRED_HTTP_FILE "_internal\libssl-3.dll"
      !insertmacro XMU_REMOVE_RETIRED_HTTP_FILE "_internal\libffi-8.dll"
      !insertmacro XMU_REMOVE_RETIRED_HTTP_FILE "_internal\libcrypto-3-x64.dll"
      !insertmacro XMU_REMOVE_RETIRED_HTTP_FILE "_internal\libssl-3-x64.dll"
      !insertmacro XMU_REMOVE_RETIRED_HTTP_FILE "_internal\ffi.dll"
    ${EndIf}
    ${If} ${FileExists} "$INSTDIR\ocr\README.md"
      RMDir /r "$INSTDIR\ocr"
    ${EndIf}
    ${If} ${FileExists} "$INSTDIR\vision-pack\manifest.json"
      RMDir /r "$INSTDIR\vision-pack"
    ${EndIf}
    ${If} ${Errors}
      MessageBox MB_ICONSTOP "旧组件仍被占用，请关闭预约程序后重新安装。个人配置与预约记录保持不变。"
      Abort
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro XMU_REMOVE_OBSOLETE_COMPONENTS
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Preserve an existing opt-in when the installation folder changes.
  !insertmacro XMU_UPDATE_STARTUP
  ; After migrating off the legacy install root, drop only orphaned program files.
  ; Never RmDir the data tree (rapidocr / captcha / tasks / runs / logs).
  StrCpy $R8 "$LOCALAPPDATA\${INTERNALNAME}"
  ${If} "$INSTDIR" != "$R8"
    Delete "$R8\${MAINBINARYNAME}.exe"
    Delete "$R8\uninstall.exe"
  ${EndIf}
  ; Rename existing owned shortcuts during an upgrade, even in /UPDATE mode.
  ; If no desktop shortcut existed, upgrading does not create one.
  !insertmacro XMU_MIGRATE_SHORTCUT "$SMPROGRAMS"
  !insertmacro XMU_MIGRATE_SHORTCUT "$SMPROGRAMS\$AppStartMenuFolder"
  !insertmacro XMU_MIGRATE_SHORTCUT "$DESKTOP"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  !insertmacro XMU_REMOVE_STARTUP
!macroend
