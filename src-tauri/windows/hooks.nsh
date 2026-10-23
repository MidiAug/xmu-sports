; xmu-sports NSIS hooks — paired with windows/installer.nsi
; App data and install identity stay at xmu-sports when display branding changes.

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
    ${If} ${FileExists} "$INSTDIR\http-booking\xmu-http-booking.exe"
      RMDir /r "$INSTDIR\http-booking\_internal"
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
