; SPDX-License-Identifier: Apache-2.0
; Standalone native Flow installer. No Tauri plugins, WebView or global runtime.
; payload.nsh and settings.nsh are generated from the verified package manifest.
Unicode true
ManifestDPIAware true
ManifestDPIAwareness PerMonitorV2
RequestExecutionLevel user
SetCompressor /SOLID zlib
SetOverwrite on
!include MUI2.nsh
!include LogicLib.nsh
!include FileFunc.nsh
!include nsDialogs.nsh
!include x64.nsh
!include "settings.nsh"

Name "流光"
BrandingText "流光"
OutFile "${FLOW_OUTFILE}"
InstallDir "$LOCALAPPDATA\流光"
InstallDirRegKey HKCU "Software\org.flowdown.desktop\Installer" "InstallLocation"
VIProductVersion "${FLOW_VERSION}.0"
VIAddVersionKey "ProductName" "流光"
VIAddVersionKey "FileDescription" "流光原生桌面版安装程序"
VIAddVersionKey "FileVersion" "${FLOW_VERSION}"
VIAddVersionKey "ProductVersion" "${FLOW_VERSION}"
!define FLOW_UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\流光"
!define FLOW_INSTALLKEY "Software\org.flowdown.desktop\Installer"
!define MUI_ABORTWARNING
!define MUI_ICON "${FLOW_ICON}"
!define MUI_UNICON "${FLOW_ICON}"

Var KeepData
Var UpdateMode
Var KeepCheckbox
Var OldRoot
Var OldVersion
Var NoShortcut
Var ShortcutOwned

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\flow-desktop.exe"
!define MUI_FINISHPAGE_RUN_NOTCHECKED
!insertmacro MUI_PAGE_FINISH
UninstPage custom un.DataPage un.DataPageLeave
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_UNPAGE_FINISH
!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"

; Never enumerate/delete recursively. Missing leaves are accepted for interrupted
; installs; linked ancestors and inaccessible entries fail before copying/removal.
!macro PathCheckFunction PREFIX
Function ${PREFIX}PathIsSafe
  Exch $0
  Push $1
  Push $2
  Push $3
  StrCpy $3 1
  check_loop:
    System::Call 'kernel32::GetFileAttributesW(w r0) i .r1'
    ${If} $1 = -1
      System::Call 'kernel32::GetLastError() i .r2'
      ${If} $2 != 2
      ${AndIf} $2 != 3
        StrCpy $3 0
        Goto check_done
      ${EndIf}
    ${Else}
      IntOp $2 $1 & 0x400
      ${If} $2 <> 0
        StrCpy $3 0
        Goto check_done
      ${EndIf}
    ${EndIf}
    ${GetParent} "$0" $2
    ${If} $2 == ""
    ${OrIf} $2 == $0
      Goto check_done
    ${EndIf}
    StrCpy $0 $2
    Goto check_loop
  check_done:
    StrCpy $0 $3
    Pop $3
    Pop $2
    Pop $1
    Exch $0
FunctionEnd
!macroend
!insertmacro PathCheckFunction ""
!insertmacro PathCheckFunction "un."

!macro RequireSafePath PREFIX PATH
  Push "${PATH}"
  Call ${PREFIX}PathIsSafe
  Pop $R9
  ${If} $R9 != 1
    SetErrorLevel 32
    IfSilent +2
    MessageBox MB_ICONSTOP "路径不可访问或包含目录链接，已停止：${PATH}"
    Abort
  ${EndIf}
!macroend

!macro CheckedHelper COMMAND
  nsExec::ExecToStack '${COMMAND}'
  Pop $0
  Pop $1
  ${If} $0 != 0
    DetailPrint "$1"
    SetErrorLevel 34
    IfSilent +2
    MessageBox MB_ICONSTOP "操作未完成，原数据保留。$\r$\n$1"
    Abort
  ${EndIf}
!macroend

!macro DeleteOwnedFile PATH
  ${If} ${FileExists} "${PATH}"
    ClearErrors
    Delete "${PATH}"
    ${If} ${Errors}
      SetErrorLevel 35
      IfSilent +2
      MessageBox MB_ICONSTOP "无法删除应用文件，请关闭流光和命令行任务后重试：${PATH}"
      Abort
    ${EndIf}
  ${EndIf}
!macroend

!include "payload.nsh"

Function .onInit
  SetShellVarContext current
  SetRegView 64
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "此安装包需要 64 位 Windows。"
    SetErrorLevel 31
    Abort
  ${EndIf}
  StrCpy $NoShortcut 0
  ClearErrors
  ${GetOptions} $CMDLINE "/NOSHORTCUT" $0
  ${IfNot} ${Errors}
    StrCpy $NoShortcut 1
  ${EndIf}
  ; Legacy 1.x used this same product uninstall key. Never execute arbitrary
  ; registry command strings: only a verified root's fixed uninstall.exe.
  ReadRegStr $OldRoot HKCU "${FLOW_UNINSTKEY}" "InstallLocation"
  ${If} $OldRoot != ""
    StrCpy $0 $OldRoot 1
    ${If} $0 == '$\"'
      StrCpy $OldRoot $OldRoot -1 1
    ${EndIf}
    StrCpy $INSTDIR $OldRoot
    ReadRegStr $OldVersion HKCU "${FLOW_UNINSTKEY}" "DisplayVersion"
  ${EndIf}
FunctionEnd

Section "Install"
  InitPluginsDir
  File "/oname=$PLUGINSDIR\flow-cli.exe" "${FLOW_HELPER}"
  File "/oname=$PLUGINSDIR\package-manifest.json" "${FLOW_PACKAGE_MANIFEST}"
  !insertmacro RequireSafePath "" "$INSTDIR"
  !insertmacro CheckPayloadPaths ""
  !insertmacro CheckedHelper '"$PLUGINSDIR\flow-cli.exe" --flow-check-install-root "$INSTDIR"'
  !insertmacro CheckedHelper '"$PLUGINSDIR\flow-cli.exe" --flow-check-processes "$INSTDIR"'
  ${If} $OldRoot != ""
    !insertmacro RequireSafePath "" "$OldRoot\uninstall.exe"
    !insertmacro CheckedHelper '"$PLUGINSDIR\flow-cli.exe" --flow-check-install-root "$OldRoot"'
    !insertmacro CheckedHelper '"$PLUGINSDIR\flow-cli.exe" --flow-check-processes "$OldRoot"'
    ${If} ${FileExists} "$OldRoot\.flow-install-pending.json"
      ${If} $OldRoot != $INSTDIR
        SetErrorLevel 37
        IfSilent +2
        MessageBox MB_ICONSTOP "请先在原安装位置修复中断的安装。"
        Abort
      ${EndIf}
    ${Else}
      ${IfNot} ${FileExists} "$OldRoot\uninstall.exe"
        SetErrorLevel 37
        IfSilent +2
        MessageBox MB_ICONSTOP "旧版卸载程序缺失，请先修复旧版安装。不会覆盖已有文件。"
        Abort
      ${EndIf}
      ClearErrors
      ExecWait '"$OldRoot\uninstall.exe" /S /UPDATE _?=$OldRoot' $0
      ${If} ${Errors}
      ${OrIf} $0 != 0
        SetErrorLevel 37
        IfSilent +2
        MessageBox MB_ICONSTOP "旧版移除未完成，已停止升级。设置和视频未主动清理。"
        Abort
      ${EndIf}
    ${EndIf}
  ${EndIf}
  ; Recheck after replacement removal; a foreign same-named file must not be adopted.
  !insertmacro CheckPayloadPaths ""
  !insertmacro CheckedHelper '"$PLUGINSDIR\flow-cli.exe" --flow-check-install-root "$INSTDIR"'
  ; Extraction happens in a private temporary directory, not over live files.
  !insertmacro ExtractPayload
  WriteUninstaller "$PLUGINSDIR\payload\uninstall.exe"
  !insertmacro CheckedHelper '"$PLUGINSDIR\flow-cli.exe" --flow-install-payload "$INSTDIR" "$PLUGINSDIR\payload" "$PLUGINSDIR\package-manifest.json"'
  WriteRegStr HKCU "${FLOW_INSTALLKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${FLOW_UNINSTKEY}" "DisplayName" "流光"
  WriteRegStr HKCU "${FLOW_UNINSTKEY}" "DisplayVersion" "${FLOW_VERSION}"
  WriteRegStr HKCU "${FLOW_UNINSTKEY}" "Publisher" "流光"
  WriteRegStr HKCU "${FLOW_UNINSTKEY}" "DisplayIcon" '$\"$INSTDIR\flow-desktop.exe$\"'
  WriteRegStr HKCU "${FLOW_UNINSTKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${FLOW_UNINSTKEY}" "UninstallString" '$\"$INSTDIR\uninstall.exe$\"'
  WriteRegStr HKCU "${FLOW_UNINSTKEY}" "QuietUninstallString" '$\"$INSTDIR\uninstall.exe$\" /S'
  WriteRegStr HKCU "${FLOW_UNINSTKEY}" "MainBinaryName" "flow-desktop.exe"
  WriteRegDWORD HKCU "${FLOW_UNINSTKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${FLOW_UNINSTKEY}" "NoRepair" 1
  WriteRegDWORD HKCU "${FLOW_UNINSTKEY}" "EstimatedSize" ${FLOW_SIZE_KIB}
  ; Keep recovery registration if initialization fails; helper owns migration.
  !insertmacro CheckedHelper '"$INSTDIR\flow-cli.exe" --flow-initialize-data'
  nsExec::ExecToStack '"$INSTDIR\flow-cli.exe" --flow-clean-legacy-ui-cache'
  Pop $0
  Pop $1
  ${If} $0 != 0
    DetailPrint "旧界面缓存仍被占用或缺少所有权证明，已保留：$1"
    IfSilent +2
    MessageBox MB_ICONINFORMATION "流光已安装。部分旧界面缓存暂时保留，可在关闭相关程序后重试清理。"
  ${EndIf}
  ${If} $NoShortcut != 1
    ${If} ${FileExists} "$SMPROGRAMS\流光.lnk"
      nsExec::ExecToStack '"$INSTDIR\flow-cli.exe" --flow-check-shortcut "$SMPROGRAMS\流光.lnk" "$INSTDIR\flow-desktop.exe"'
      Pop $0
      Pop $1
      ${If} $0 = 0
        CreateShortcut "$SMPROGRAMS\流光.lnk" "$INSTDIR\flow-desktop.exe"
        !insertmacro CheckedHelper '"$INSTDIR\flow-cli.exe" --flow-register-shortcut "$SMPROGRAMS\流光.lnk" "$INSTDIR\flow-desktop.exe"'
      ${Else}
        DetailPrint "已保留指向其他程序的同名快捷方式。"
      ${EndIf}
    ${Else}
      CreateShortcut "$SMPROGRAMS\流光.lnk" "$INSTDIR\flow-desktop.exe"
      !insertmacro CheckedHelper '"$INSTDIR\flow-cli.exe" --flow-register-shortcut "$SMPROGRAMS\流光.lnk" "$INSTDIR\flow-desktop.exe"'
    ${EndIf}
  ${EndIf}
SectionEnd

Function un.onInit
  SetShellVarContext current
  SetRegView 64
  StrCpy $KeepData 0
  StrCpy $UpdateMode 0
  ; /KEEPDATA wins over /DELETEAPPDATA, including unattended uninstall.
  ClearErrors
  ${GetOptions} $CMDLINE "/DELETEAPPDATA" $0
  ${IfNot} ${Errors}
    StrCpy $KeepData 0
  ${EndIf}
  ClearErrors
  ${GetOptions} $CMDLINE "/KEEPDATA" $0
  ${IfNot} ${Errors}
    StrCpy $KeepData 1
  ${EndIf}
  ClearErrors
  ${GetOptions} $CMDLINE "/UPDATE" $0
  ${IfNot} ${Errors}
    StrCpy $UpdateMode 1
  ${EndIf}
FunctionEnd

Function un.DataPage
  ${If} $UpdateMode = 1
    Abort
  ${EndIf}
  nsDialogs::Create 1018
  Pop $0
  ${If} $0 == error
    Abort
  ${EndIf}
  ${NSD_CreateLabel} 0 0 100% 28u "卸载流光。视频文件会保留。"
  Pop $0
  ${NSD_CreateCheckbox} 0 35u 100% 25u "保留设置、预设、历史和私有引擎"
  Pop $KeepCheckbox
  ${NSD_SetState} $KeepCheckbox $KeepData
  ${NSD_CreateLabel} 0 65u 100% 35u "未勾选时清理流光自己的设置、缓存和临时文件；勾选后仍清理可丢弃缓存。"
  Pop $0
  nsDialogs::Show
FunctionEnd

Function un.DataPageLeave
  ${NSD_GetState} $KeepCheckbox $KeepData
FunctionEnd

Section "Uninstall"
  !insertmacro CheckPayloadPaths "un."
  !insertmacro CheckedHelper '"$INSTDIR\flow-cli.exe" --flow-check-install-root "$INSTDIR"'
  !insertmacro CheckedHelper '"$INSTDIR\flow-cli.exe" --flow-check-processes "$INSTDIR"'
  StrCpy $ShortcutOwned 0
  ${If} ${FileExists} "$SMPROGRAMS\流光.lnk"
    nsExec::ExecToStack '"$INSTDIR\flow-cli.exe" --flow-check-shortcut "$SMPROGRAMS\流光.lnk" "$INSTDIR\flow-desktop.exe"'
    Pop $0
    Pop $1
    ${If} $0 = 0
      StrCpy $ShortcutOwned 1
    ${EndIf}
  ${EndIf}
  ${If} $UpdateMode != 1
    ${If} $KeepData = 1
      !insertmacro CheckedHelper '"$INSTDIR\flow-cli.exe" --flow-uninstall-cleanup --keep-data'
    ${Else}
      !insertmacro CheckedHelper '"$INSTDIR\flow-cli.exe" --flow-uninstall-cleanup'
    ${EndIf}
  ${EndIf}
  ; Resources first: retain helper, app, ownership marker and registration until
  ; all resource deletes succeeded so an interrupted uninstall can be retried.
  !insertmacro DeletePayloadResources
  !insertmacro DeleteOwnedFile "$INSTDIR\flow-desktop.exe"
  !insertmacro DeleteOwnedFile "$INSTDIR\flow-cli.exe"
  !insertmacro DeleteOwnedFile "$INSTDIR\.flow-install-pending.json"
  !insertmacro DeleteOwnedFile "$INSTDIR\.flow-install-owner.json"
  Delete "$INSTDIR\uninstall.exe"
  !insertmacro RemoveEmptyPayloadDirectories
  RMDir "$INSTDIR"
  ReadRegStr $0 HKCU "${FLOW_UNINSTKEY}" "InstallLocation"
  ${If} $0 == $INSTDIR
    DeleteRegKey HKCU "${FLOW_UNINSTKEY}"
  ${EndIf}
  ${If} $UpdateMode != 1
    ${If} $ShortcutOwned = 1
      Delete "$SMPROGRAMS\流光.lnk"
    ${EndIf}
    ReadRegStr $0 HKCU "${FLOW_INSTALLKEY}" "InstallLocation"
    ${If} $0 == $INSTDIR
      DeleteRegKey HKCU "${FLOW_INSTALLKEY}"
      DeleteRegKey /ifempty HKCU "Software\org.flowdown.desktop"
    ${EndIf}
  ${EndIf}
SectionEnd
