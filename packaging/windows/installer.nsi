; Ferrent Windows installer.
;
; Cross-built with makensis on Linux, from the staged mingw binaries. Every
; value that changes per build comes from packaging/windows/make-installer.sh
; through -D, so this file stays free of version strings.
;
; The service is registered by service.ps1 (NSSM), the same script the ZIP
; ships: one code path for both install routes.

Unicode true

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"
!include "FileFunc.nsh"

!ifndef VERSION
  !define VERSION "0.0.0"
!endif
!ifndef VERSION4
  !define VERSION4 "0.0.0.0"
!endif
!ifndef SRCDIR
  !define SRCDIR "."
!endif
!ifndef OUTFILE
  !define OUTFILE "Ferrent-setup.exe"
!endif
!ifndef ICONFILE
  !define ICONFILE "${SRCDIR}/ferrent.ico"
!endif
!ifndef REPO_ROOT
  !define REPO_ROOT "../.."
!endif

!define APPNAME "Ferrent"

; Language ids as numbers: NSIS 3 on Linux ships no Languages.nsh to define
; LANG_ENGLISH and friends for the LangString directives.
!define LANG_EN 1033
!define LANG_HU 1038
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APPNAME}"
!define DATADIR "$APPDATA\ferrent"

Name "${APPNAME} ${VERSION}"
OutFile "${OUTFILE}"
InstallDir "$PROGRAMFILES64\${APPNAME}"
InstallDirRegKey HKLM "Software\${APPNAME}" "InstallDir"
RequestExecutionLevel admin
SetCompressor /SOLID lzma
BrandingText "${APPNAME} ${VERSION}"

VIProductVersion "${VERSION4}"
VIAddVersionKey "ProductName" "${APPNAME}"
VIAddVersionKey "FileDescription" "Ferrent torrent daemon, CLI and desktop client"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "MIT OR Apache-2.0"

!define MUI_ICON "${ICONFILE}"
!define MUI_UNICON "${ICONFILE}"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\ferrent-gui.exe"
!define MUI_FINISHPAGE_RUN_TEXT "$(FinishRun)"

LangString FinishRun        ${LANG_EN}  "Start Ferrent GUI"
LangString FinishRun        ${LANG_HU} "Ferrent GUI indítása"
LangString SecCore          ${LANG_EN}  "Program files (required)"
LangString SecCore          ${LANG_HU} "Programfájlok (kötelező)"
LangString SecCoreDesc      ${LANG_EN}  "ferrentd, ferrent-cli, ferrent-gui and the C++ runtime."
LangString SecCoreDesc      ${LANG_HU} "ferrentd, ferrent-cli, ferrent-gui és a C++ runtime."
LangString SecService       ${LANG_EN}  "Windows service (automatic start)"
LangString SecService       ${LANG_HU} "Windows service (automatikus indítás)"
LangString SecServiceDesc   ${LANG_EN}  "Registers Ferrent as a service through NSSM, with a data directory under %ProgramData%\ferrent."
LangString SecServiceDesc   ${LANG_HU} "A Ferrent regisztrálása service-ként NSSM-mel, a %ProgramData%\ferrent alatti adatkönyvtárral."
LangString SecDesktop       ${LANG_EN}  "Desktop shortcut"
LangString SecDesktop       ${LANG_HU} "Asztali parancsikon"
LangString SecDesktopDesc   ${LANG_EN}  "A shortcut to the GUI on the desktop."
LangString SecDesktopDesc   ${LANG_HU} "Parancsikon a GUI-hoz az asztalon."
LangString MsgToken         ${LANG_EN}  "The Ferrent service is installed and running.$\r$\n$\r$\nToken for the GUI:$\r$\n$8$\r$\n$\r$\nFile: ${DATADIR}\token"
LangString MsgToken         ${LANG_HU} "A Ferrent service telepítve van és fut.$\r$\n$\r$\nToken a GUI-hoz:$\r$\n$8$\r$\n$\r$\nFájl: ${DATADIR}\token"
LangString MsgServiceFailed ${LANG_EN}  "The service could not be registered (exit code $0). The files are installed; run service.ps1 install from an elevated PowerShell to retry."
LangString MsgServiceFailed ${LANG_HU} "A service regisztrálása nem sikerült (exit kód: $0). A fájlok telepítve vannak; emelt PowerShellből futtasd a service.ps1 install parancsot."
LangString MsgKeepData      ${LANG_EN}  "Delete the data directory too?$\r$\n$\r$\n${DATADIR}$\r$\n$\r$\nIt holds the torrents, the state and the token."
LangString MsgKeepData      ${LANG_HU} "Töröljük az adatkönyvtárat is?$\r$\n$\r$\n${DATADIR}$\r$\n$\r$\nItt vannak a torrentek, az állapot és a token."

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${REPO_ROOT}/LICENSE"
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "Hungarian"

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "Ferrent requires 64-bit Windows."
    Abort
  ${EndIf}
  SetRegView 64
  SetShellVarContext all
FunctionEnd

Function un.onInit
  SetRegView 64
  SetShellVarContext all
FunctionEnd

Section "$(SecCore)" SEC_CORE
  SectionIn RO
  SetOutPath "$INSTDIR"
  File "${SRCDIR}/ferrentd.exe"
  File "${SRCDIR}/ferrent-cli.exe"
  File "${SRCDIR}/ferrent-gui.exe"
  File "${SRCDIR}/libstdc++-6.dll"
  File "${SRCDIR}/nssm.exe"
  File "${SRCDIR}/service.ps1"
  File "${SRCDIR}/config.example.toml"
  File "${SRCDIR}/README.txt"
  File "${ICONFILE}"

  CreateDirectory "$SMPROGRAMS\${APPNAME}"
  CreateShortCut "$SMPROGRAMS\${APPNAME}\${APPNAME}.lnk" "$INSTDIR\ferrent-gui.exe" "" "$INSTDIR\ferrent.ico"
  CreateShortCut "$SMPROGRAMS\${APPNAME}\Uninstall.lnk" "$INSTDIR\uninstall.exe"

  WriteRegStr HKLM "Software\${APPNAME}" "InstallDir" "$INSTDIR"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  WriteRegStr HKLM "${UNINST_KEY}" "DisplayName" "${APPNAME}"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${UNINST_KEY}" "Publisher" "Ferrent"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\ferrent.ico"
  WriteRegStr HKLM "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKLM "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegStr HKLM "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $2 $3 $4
  WriteRegDWORD HKLM "${UNINST_KEY}" "EstimatedSize" "$2"
SectionEnd

Section "$(SecService)" SEC_SERVICE
  DetailPrint "Registering the Ferrent service..."
  nsExec::ExecToLog 'powershell -NoProfile -ExecutionPolicy Bypass -File "$INSTDIR\service.ps1" install -BinDir "$INSTDIR"'
  Pop $0
  ${If} $0 != "0"
    MessageBox MB_ICONEXCLAMATION|MB_OK "$(MsgServiceFailed)"
  ${Else}
    StrCpy $8 ""
    FileOpen $9 "${DATADIR}\token" r
    ${If} $9 != ""
      FileRead $9 $8
      FileClose $9
    ${EndIf}
    ${IfNot} ${Silent}
      MessageBox MB_ICONINFORMATION|MB_OK "$(MsgToken)"
    ${EndIf}
  ${EndIf}
SectionEnd

Section "$(SecDesktop)" SEC_DESKTOP
  CreateShortCut "$DESKTOP\${APPNAME}.lnk" "$INSTDIR\ferrent-gui.exe" "" "$INSTDIR\ferrent.ico"
SectionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SEC_CORE} "$(SecCoreDesc)"
  !insertmacro MUI_DESCRIPTION_TEXT ${SEC_SERVICE} "$(SecServiceDesc)"
  !insertmacro MUI_DESCRIPTION_TEXT ${SEC_DESKTOP} "$(SecDesktopDesc)"
!insertmacro MUI_FUNCTION_DESCRIPTION_END

Section "Uninstall"
  ; Stop and unregister first: nssm.exe and service.ps1 live in $INSTDIR.
  nsExec::ExecToLog 'powershell -NoProfile -ExecutionPolicy Bypass -File "$INSTDIR\service.ps1" uninstall -BinDir "$INSTDIR"'
  Pop $0

  Delete "$SMPROGRAMS\${APPNAME}\${APPNAME}.lnk"
  Delete "$SMPROGRAMS\${APPNAME}\Uninstall.lnk"
  RMDir "$SMPROGRAMS\${APPNAME}"
  Delete "$DESKTOP\${APPNAME}.lnk"

  Delete "$INSTDIR\ferrentd.exe"
  Delete "$INSTDIR\ferrent-cli.exe"
  Delete "$INSTDIR\ferrent-gui.exe"
  Delete "$INSTDIR\libstdc++-6.dll"
  Delete "$INSTDIR\nssm.exe"
  Delete "$INSTDIR\service.ps1"
  Delete "$INSTDIR\config.example.toml"
  Delete "$INSTDIR\README.txt"
  Delete "$INSTDIR\ferrent.ico"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  DeleteRegKey HKLM "${UNINST_KEY}"
  DeleteRegKey HKLM "Software\${APPNAME}"

  ; The data directory is the torrents and the state: keep it unless asked.
  ${If} ${Silent}
    DetailPrint "Keeping ${DATADIR}"
  ${Else}
    MessageBox MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON2 "$(MsgKeepData)" IDNO keep_data
    RMDir /r "${DATADIR}"
    keep_data:
  ${EndIf}
SectionEnd
