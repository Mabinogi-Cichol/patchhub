Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!ifndef BUNDLE
  !error "BUNDLE must point to the complete Flutter Release directory"
!endif
!ifndef VERSION
  !define VERSION "0.3.0"
!endif
Name "PatchHub"
OutFile "${OUTPUT}"
InstallDir "$LOCALAPPDATA\PatchHub"
RequestExecutionLevel user
SetCompressor /SOLID lzma
Var ProductName
Var UninstallKey
Var MaintenanceHandle
!define MUI_ICON "${__FILEDIR__}/../apps/windows-flutter/windows/runner/resources/app_icon.ico"
!define MUI_UNICON "${__FILEDIR__}/../apps/windows-flutter/windows/runner/resources/app_icon.ico"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  SetRegView 64
  StrCpy $ProductName "PatchHub"
  ReadRegStr $0 HKCU "Software\github\PatchHub" ""
  ${If} $0 != ""
    StrCpy $INSTDIR $0
  ${EndIf}
  StrCpy $UninstallKey "Software\Microsoft\Windows\CurrentVersion\Uninstall\$ProductName"
FunctionEnd

Section "PatchHub"
  CreateDirectory "$INSTDIR"
  System::Call 'kernel32::CreateFileW(w "$INSTDIR\.patchhub-maintenance.lock", i 0xC0000000, i 0, p 0, i 4, i 0, p 0) p.r0'
  StrCpy $MaintenanceHandle $0
  ${If} $MaintenanceHandle == -1
    MessageBox MB_OK "应用正在更新或卸载，请稍后重试。"
    Abort
  ${EndIf}
  IfFileExists "$INSTDIR\.patchhub-update.json" 0 +4
    System::Call 'kernel32::CloseHandle(p $MaintenanceHandle)'
    MessageBox MB_OK "请先启动应用完成更新恢复，再运行安装器。"
    Abort
  SetOutPath "$INSTDIR"
  File /r /x ".patchhub-maintenance.lock" /x ".patchhub-update.json" /x "install.ini" /x "uninstall.exe" "${BUNDLE}\*"
  CreateShortcut "$SMPROGRAMS\$ProductName.lnk" "$INSTDIR\patchhub.exe"
  WriteRegStr HKCU "Software\github\$ProductName" "" "$INSTDIR"
  WriteRegStr HKCU "$UninstallKey" "DisplayName" "$ProductName"
  WriteRegStr HKCU "$UninstallKey" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "$UninstallKey" "Publisher" "github"
  WriteRegStr HKCU "$UninstallKey" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "$UninstallKey" "MainBinaryName" "patchhub.exe"
  WriteRegStr HKCU "$UninstallKey" "DisplayIcon" "$INSTDIR\patchhub.exe"
  WriteRegStr HKCU "$UninstallKey" "UninstallString" '$\"$INSTDIR\uninstall.exe$\"'
  WriteRegStr HKCU "$UninstallKey" "QuietUninstallString" '$\"$INSTDIR\uninstall.exe$\" /S'
  WriteRegDWORD HKCU "$UninstallKey" "NoModify" 1
  WriteRegDWORD HKCU "$UninstallKey" "NoRepair" 1
  WriteINIStr "$INSTDIR\install.ini" "Application" "ProductName" "$ProductName"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  System::Call 'kernel32::CloseHandle(p $MaintenanceHandle)'
SectionEnd

Section "Uninstall"
  SetRegView 64
  ReadINIStr $ProductName "$INSTDIR\install.ini" "Application" "ProductName"
  StrCpy $UninstallKey "Software\Microsoft\Windows\CurrentVersion\Uninstall\$ProductName"
  ExecWait '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$INSTDIR\windows-update.ps1" -Mode Uninstall -Target "$INSTDIR"' $0
  ${If} $0 != 0
    MessageBox MB_OK "卸载未完成：请关闭应用并完成待恢复的更新后重试。"
    Abort
  ${EndIf}
  Delete "$SMPROGRAMS\$ProductName.lnk"
  DeleteRegKey HKCU "$UninstallKey"
  DeleteRegKey HKCU "Software\github\$ProductName"
  Delete "$INSTDIR\install.ini"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  ; APPDATA/PatchHub contains recovery data and is retained.
SectionEnd
