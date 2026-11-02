; Townjump installer for Windows x64 and ARM64 (NSIS 3, Unicode, MUI2).
; Build: makensis -V2 -WX -DVERSION=<x.y.z> -DEXE=<townjump.exe> -DLICENSE=<LICENSE file> \
;                -DOUTFILE=<installer path> installer.nsi
; (scripts/dist/windows-installer.sh passes all four).
; Installs townjump.exe, LICENSE.txt and an uninstaller under Program Files and adds that directory to the
; machine PATH. No shortcuts, no service, no firewall rule. User data is never touched.
; PATH is edited by path-entry.ps1 only: NSIS strings are limited to 1023 characters (NSIS_MAX_STRLEN), a long
; system Path would be truncated by ReadRegStr and written back damaged.

!ifndef VERSION
  !error "define VERSION: makensis -DVERSION=<x.y.z> ..."
!endif
!ifndef EXE
  !error "define EXE: makensis -DEXE=<path to townjump.exe> ..."
!endif
!ifndef LICENSE
  !error "define LICENSE: makensis -DLICENSE=<path to the license file> ..."
!endif
!ifndef OUTFILE
  !error "define OUTFILE: makensis -DOUTFILE=<path to the installer> ..."
!endif

!define APP "Townjump"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Townjump"
!define HOME_URL "https://github.com/mesh-town/townjump"

Unicode true
Name "${APP}"
OutFile "${OUTFILE}"
InstallDir "$PROGRAMFILES64\${APP}"
InstallDirRegKey HKLM "${UNINST_KEY}" "InstallLocation"
RequestExecutionLevel admin
SetCompressor /SOLID lzma
BrandingText "${APP} ${VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APP}"
VIAddVersionKey "CompanyName" "Mesh Town"
VIAddVersionKey "FileDescription" "${APP} installer"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "AGPL-3.0-only"

!include "MUI2.nsh"
!include "x64.nsh"
!include "WinMessages.nsh"

!define MUI_ABORTWARNING

!insertmacro MUI_PAGE_LICENSE "${LICENSE}"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

; Runs path-entry.ps1 (add or remove) for $INSTDIR. A failure is a warning, never an abort.
; The script is extracted to $PLUGINSDIR (installer) or read from $INSTDIR (uninstaller, where File is not allowed).
!macro PathEdit un action script
  nsExec::ExecToLog 'powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "${script}" ${action} "$INSTDIR"'
  Pop $0
  ${If} $0 == 0
    SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=5000
  ${Else}
    DetailPrint "Could not update PATH (result: $0)."
    MessageBox MB_OK|MB_ICONEXCLAMATION "Could not update PATH. ${action} $INSTDIR in the PATH manually." /SD IDOK
  ${EndIf}
!macroend

Function .onInit
  ${IfNot} ${RunningX64}
  ${AndIfNot} ${IsNativeARM64}
    MessageBox MB_OK|MB_ICONSTOP "${APP} needs 64-bit Windows." /SD IDOK
    Abort
  ${EndIf}
  SetRegView 64
  ReadRegStr $0 HKLM "${UNINST_KEY}" "InstallLocation"
  ${If} $0 != ""
    StrCpy $INSTDIR $0
  ${EndIf}
FunctionEnd

Function un.onInit
  SetRegView 64
FunctionEnd

Section "${APP}" SecMain
  SectionIn RO
  SetRegView 64
  SetShellVarContext all

  SetOutPath "$INSTDIR"
  File "/oname=townjump.exe" "${EXE}"
  File "/oname=LICENSE.txt" "${LICENSE}"
  File "/oname=path-entry.ps1" "path-entry.ps1"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  WriteRegStr HKLM "${UNINST_KEY}" "DisplayName" "${APP}"
  WriteRegStr HKLM "${UNINST_KEY}" "Publisher" "Mesh Town"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKLM "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegStr HKLM "${UNINST_KEY}" "URLInfoAbout" "${HOME_URL}"
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoRepair" 1

  !insertmacro PathEdit "" add "$INSTDIR\path-entry.ps1"
SectionEnd

Section "Uninstall"
  SetRegView 64
  SetShellVarContext all

  !insertmacro PathEdit "un." remove "$INSTDIR\path-entry.ps1"

  Delete "$INSTDIR\townjump.exe"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\path-entry.ps1"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  DeleteRegKey HKLM "${UNINST_KEY}"
SectionEnd
