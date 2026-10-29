; KohdaLab PLE Windows installer.
; SPDX-License-Identifier: MIT
;
; Required compile-time definitions from scripts/build_windows_installer.py:
; PAYLOAD_INCLUDE, OUTPUT_FILE, VERSION, VERSION_FILE.

Unicode true
SetCompressor /SOLID zlib
SetDatablockOptimize on
CRCCheck force

!ifndef PAYLOAD_INCLUDE
  !error "PAYLOAD_INCLUDE must name the generated payload include"
!endif
!ifndef OUTPUT_FILE
  !error "OUTPUT_FILE must name Setup.exe"
!endif
!ifndef VERSION
  !error "VERSION must be set"
!endif
!ifndef VERSION_FILE
  !error "VERSION_FILE must be a four-part numeric version"
!endif

!include "MUI2.nsh"
!include "x64.nsh"
!include "LogicLib.nsh"
!include "${PAYLOAD_INCLUDE}"

Name "KohdaLab PLE ${VERSION}"
OutFile "${OUTPUT_FILE}"
InstallDir "$PROGRAMFILES64\Kohdalab\KohdaLab PLE"
RequestExecutionLevel admin
ShowInstDetails show
ShowUninstDetails show
BrandingText "KohdaLab PLE"

VIProductVersion ${VERSION_FILE}
VIAddVersionKey /LANG=1033 "ProductName" "KohdaLab PLE"
VIAddVersionKey /LANG=1033 "ProductVersion" "${VERSION}"
VIAddVersionKey /LANG=1033 "CompanyName" "Kohdalab"
VIAddVersionKey /LANG=1033 "FileDescription" "KohdaLab PLE Setup"
VIAddVersionKey /LANG=1033 "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=1033 "LegalCopyright" "Copyright Kohdalab contributors"

!define MUI_ABORTWARNING
!define MUI_WELCOMEPAGE_TITLE "KohdaLab PLE Setup"
!define MUI_WELCOMEPAGE_TEXT "This installs KohdaLab PLE for all Windows users. Existing settings are kept."
!define MUI_FINISHPAGE_TITLE "KohdaLab PLE is ready"
!define MUI_FINISHPAGE_TEXT "Start KohdaLab PLE from the Start menu or desktop shortcut."

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "Japanese"

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP|MB_OK "KohdaLab PLE requires 64-bit Windows."
    SetErrorLevel 1
    Abort
  ${EndIf}
  SetRegView 64
  ; Do not accept a registry value or /D override for this application.
  StrCpy $INSTDIR "$PROGRAMFILES64\Kohdalab\KohdaLab PLE"
  SetShellVarContext all
FunctionEnd

Function un.onInit
  SetRegView 64
  ; NSIS may launch a temporary uninstaller copy, but retains the original
  ; install path in $INSTDIR. Refuse caller-supplied removal paths.
  StrCpy $0 $INSTDIR
  StrCpy $INSTDIR "$PROGRAMFILES64\Kohdalab\KohdaLab PLE"
  SetShellVarContext all
  ${If} $0 != ""
  ${AndIf} $0 != $INSTDIR
    IfSilent reject
    MessageBox MB_ICONSTOP|MB_OK "KohdaLab PLE can only be removed from its installed location."
reject:
    SetErrorLevel 1
    Abort
  ${EndIf}
FunctionEnd

; Request an exclusive write handle before changing an existing installation.
; This detects a running or otherwise locked process without terminating it.
Function EnsurePLEIsClosed
  IfFileExists "$INSTDIR\kohdalab-ple.exe" 0 install_probe_cli
  System::Call 'kernel32::CreateFileW(w "$INSTDIR\kohdalab-ple.exe", i 0x40000000, i 0, p 0, i 3, i 0, p 0) p.r0'
  ${If} $0 == -1
    IfSilent install_abort_gui
    MessageBox MB_ICONEXCLAMATION|MB_OK "Close KohdaLab PLE before installing or updating it."
install_abort_gui:
    SetErrorLevel 1
    Abort
  ${EndIf}
  System::Call 'kernel32::CloseHandle(p r0)'
install_probe_cli:
  IfFileExists "$INSTDIR\companions\kohdalab-shutter\kohdalab-shutter-gui.exe" 0 install_done
  System::Call 'kernel32::CreateFileW(w "$INSTDIR\companions\kohdalab-shutter\kohdalab-shutter-gui.exe", i 0x40000000, i 0, p 0, i 3, i 0, p 0) p.r0'
  ${If} $0 == -1
    IfSilent install_abort_shutter
    MessageBox MB_ICONEXCLAMATION|MB_OK "Close the bundled shutter GUI before changing this installation."
install_abort_shutter:
    SetErrorLevel 1
    Abort
  ${EndIf}
  System::Call 'kernel32::CloseHandle(p r0)'
install_done:
FunctionEnd

Function un.EnsurePLEIsClosed
  IfFileExists "$INSTDIR\kohdalab-ple.exe" 0 uninstall_probe_cli
  System::Call 'kernel32::CreateFileW(w "$INSTDIR\kohdalab-ple.exe", i 0x40000000, i 0, p 0, i 3, i 0, p 0) p.r0'
  ${If} $0 == -1
    IfSilent uninstall_abort_gui
    MessageBox MB_ICONEXCLAMATION|MB_OK "Close KohdaLab PLE before uninstalling it."
uninstall_abort_gui:
    SetErrorLevel 1
    Abort
  ${EndIf}
  System::Call 'kernel32::CloseHandle(p r0)'
uninstall_probe_cli:
  IfFileExists "$INSTDIR\companions\kohdalab-shutter\kohdalab-shutter-gui.exe" 0 uninstall_done
  System::Call 'kernel32::CreateFileW(w "$INSTDIR\companions\kohdalab-shutter\kohdalab-shutter-gui.exe", i 0x40000000, i 0, p 0, i 3, i 0, p 0) p.r0'
  ${If} $0 == -1
    IfSilent uninstall_abort_shutter
    MessageBox MB_ICONEXCLAMATION|MB_OK "Close the bundled shutter GUI before changing this installation."
uninstall_abort_shutter:
    SetErrorLevel 1
    Abort
  ${EndIf}
  System::Call 'kernel32::CloseHandle(p r0)'
uninstall_done:
FunctionEnd

Var TargetDirectory
Var StagingDirectory
Var PreviousDirectory

Section "KohdaLab PLE" SecMain
  Call EnsurePLEIsClosed
  ; Extract completely into a same-volume sibling before changing an install.
  StrCpy $TargetDirectory $INSTDIR
  ClearErrors
  CreateDirectory "$PROGRAMFILES64\Kohdalab"
  IfErrors stage_not_started
  GetTempFileName $StagingDirectory "$PROGRAMFILES64\Kohdalab"
  IfErrors stage_not_started
  StrCmp $StagingDirectory "" stage_not_started
  Delete "$StagingDirectory"
  CreateDirectory "$StagingDirectory"
  IfErrors stage_not_started
  StrCpy $INSTDIR $StagingDirectory
  ClearErrors
  !insertmacro InstallPayload
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  IfErrors stage_failed
  SetOutPath "$TEMP"
  StrCpy $PreviousDirectory ""
  StrCpy $INSTDIR $TargetDirectory
  IfFileExists "$TargetDirectory\." 0 activate
  GetTempFileName $PreviousDirectory "$PROGRAMFILES64\Kohdalab"
  Delete "$PreviousDirectory"
  ClearErrors
  Rename "$TargetDirectory" "$PreviousDirectory"
  IfErrors stage_failed
activate:
  ClearErrors
  Rename "$StagingDirectory" "$TargetDirectory"
  IfErrors activation_failed
  ${If} $PreviousDirectory != ""
    DetailPrint "Previous application preserved at $PreviousDirectory"
  ${EndIf}
  Goto activated
activation_failed:
  ${If} $PreviousDirectory != ""
    Rename "$PreviousDirectory" "$TargetDirectory"
  ${EndIf}
stage_failed:
  ; Remove only files extracted by this payload. User data is never a target.
  StrCpy $INSTDIR $StagingDirectory
  !insertmacro RemovePayload
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
stage_not_started:
  StrCpy $INSTDIR $TargetDirectory
  IfSilent stage_abort
  MessageBox MB_ICONSTOP|MB_OK "Installation failed. The previous application was preserved. Close PLE and retry."
stage_abort:
  SetErrorLevel 1
  Abort
activated:
  WriteRegStr HKLM "Software\KohdaLab\KohdaLab PLE" "PreviousLocation" "$PreviousDirectory"
  WriteRegStr HKLM "Software\KohdaLab\KohdaLab PLE" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\KohdaLabPLE" "DisplayName" "KohdaLab PLE"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\KohdaLabPLE" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\KohdaLabPLE" "Publisher" "Kohdalab"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\KohdaLabPLE" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\KohdaLabPLE" "DisplayIcon" "$INSTDIR\kohdalab-ple.exe"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\KohdaLabPLE" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\KohdaLabPLE" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\KohdaLabPLE" "URLInfoAbout" "https://github.com/woolen-yarn/kohdalab-ple"
  WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\KohdaLabPLE" "NoModify" 1
  WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\KohdaLabPLE" "NoRepair" 1
  CreateDirectory "$SMPROGRAMS\KohdaLab\PLE"
  SetOutPath "$INSTDIR"
  CreateShortcut "$SMPROGRAMS\KohdaLab\PLE\KohdaLab PLE.lnk" "$INSTDIR\kohdalab-ple.exe" "" "$INSTDIR\kohdalab-ple.exe" 0
  CreateShortcut "$SMPROGRAMS\KohdaLab\PLE\Shutter (automatic BG).lnk" "$INSTDIR\companions\kohdalab-shutter\kohdalab-shutter-gui.exe" "--gui-api"
  CreateShortcut "$SMPROGRAMS\KohdaLab\PLE\Getting started.lnk" "$SYSDIR\notepad.exe" '$\"$INSTDIR\START-HERE.md$\"'
  CreateShortcut "$SMPROGRAMS\KohdaLab\PLE\Uninstall KohdaLab PLE.lnk" "$INSTDIR\Uninstall.exe"
  CreateShortcut "$DESKTOP\KohdaLab PLE.lnk" "$INSTDIR\kohdalab-ple.exe" "" "$INSTDIR\kohdalab-ple.exe" 0
SectionEnd

Section "Uninstall"
  Call un.EnsurePLEIsClosed
  SetOutPath "$TEMP"
  Delete "$SMPROGRAMS\KohdaLab\PLE\KohdaLab PLE.lnk"
  Delete "$SMPROGRAMS\KohdaLab\PLE\Shutter (automatic BG).lnk"
  Delete "$SMPROGRAMS\KohdaLab\PLE\Getting started.lnk"
  Delete "$SMPROGRAMS\KohdaLab\PLE\Uninstall KohdaLab PLE.lnk"
  RMDir "$SMPROGRAMS\KohdaLab\PLE"
  Delete "$DESKTOP\KohdaLab PLE.lnk"
  DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\KohdaLabPLE"
  DeleteRegKey HKLM "Software\KohdaLab\KohdaLab PLE"
  !insertmacro RemovePayload
  Delete /REBOOTOK "$INSTDIR\Uninstall.exe"
  RMDir "$SMPROGRAMS\KohdaLab"
  RMDir "$INSTDIR"
SectionEnd
