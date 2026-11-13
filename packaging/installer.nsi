; Zecky Lock it for Windows: per-user installer (no admin needed).
Unicode true
SetCompressor /SOLID lzma
!include "MUI2.nsh"

!define NAME "Zecky Lock it"
!define VERSION "0.1.0-beta.1"
!define PUBLISHER "Zecky"
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\ZeckyLockIt"

Name "${NAME} ${VERSION}"
OutFile "out\ZeckyLockIt-${VERSION}-setup.exe"
InstallDir "$LOCALAPPDATA\Programs\Zecky Lock it"
InstallDirRegKey HKCU "Software\ZeckyLockIt" "InstallDir"
RequestExecutionLevel user
BrandingText "Zecky Lock it ${VERSION}"
VIProductVersion "0.1.0.1"
VIAddVersionKey "ProductName" "${NAME}"
VIAddVersionKey "FileDescription" "${NAME} installer"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "CompanyName" "${PUBLISHER}"
VIAddVersionKey "LegalCopyright" "Zecky"

!define MUI_ICON "icon.ico"
!define MUI_UNICON "icon.ico"
!define MUI_WELCOMEPAGE_TITLE "Zecky Lock it ${VERSION}"
!define MUI_WELCOMEPAGE_TEXT "A calm, self-custody Zcash wallet.$\r$\n$\r$\nThis is a beta: use small amounts only (under 0.1 ZEC).$\r$\n$\r$\nYour wallet is kept in your Windows profile, not in the program folder, and stays when Lock it is removed."
!define MUI_FINISHPAGE_RUN "$INSTDIR\Zecky Lock it.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Start Zecky Lock it"
!define MUI_FINISHPAGE_SHOWREADME "$INSTDIR\READ-ME-FIRST.txt"
!define MUI_FINISHPAGE_SHOWREADME_TEXT "Read the first-start notes"
!define MUI_FINISHPAGE_SHOWREADME_NOTCHECKED

!insertmacro MUI_PAGE_WELCOME
!define MUI_LICENSEPAGE_TEXT_TOP "Please read the disclaimer and the license. Scroll down for the full license."
!define MUI_LICENSEPAGE_TEXT_BOTTOM "Zecky Lock it is free software, provided as is, without warranty. Click I Agree to accept and continue."
!insertmacro MUI_PAGE_LICENSE "stage\LICENSE-AND-DISCLAIMER.txt"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

; Lock it's window is drawn by Microsoft Edge WebView2, present on almost every
; Windows 10/11 PC. Without it the app cannot open: say so before installing,
; and offer Microsoft's own download page (nothing is downloaded by us).
!define WEBVIEW2_KEY "SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
!define WEBVIEW2_PAGE "https://developer.microsoft.com/microsoft-edge/webview2/#download"

Function HasWebView2
  ; Per machine (the 32-bit view reads WOW6432Node on 64-bit Windows), then per user.
  ReadRegStr $0 HKLM "${WEBVIEW2_KEY}" "pv"
  StrCmp $0 "" 0 +2
  ReadRegStr $0 HKCU "${WEBVIEW2_KEY}" "pv"
  StrCmp $0 "0.0.0.0" 0 +2
  StrCpy $0 ""
FunctionEnd

Function .onInit
  Call HasWebView2
  StrCmp $0 "" 0 done
  MessageBox MB_YESNO|MB_ICONEXCLAMATION "Zecky Lock it needs Microsoft Edge WebView2, which is missing on this PC.$\r$\n$\r$\nOpen Microsoft's download page now? Install the Evergreen Bootstrapper there, then run this installer again.$\r$\n$\r$\n(No: install Lock it anyway.)" IDNO done
  ExecShell "open" "${WEBVIEW2_PAGE}"
  Quit
  done:
FunctionEnd

Section "Install"
  ; Lock it must be closed (it may be running Go Max).
  nsExec::Exec 'taskkill /IM "Zecky Lock it.exe" /F'
  nsExec::Exec 'taskkill /IM zakurad.exe /F'
  nsExec::Exec 'taskkill /IM zainod.exe /F'
  Sleep 800
  SetOutPath "$INSTDIR"
  File "stage\Zecky Lock it.exe"
  File "stage\WebView2Loader.dll"
  File "stage\READ-ME-FIRST.txt"
  File "stage\THIRD-PARTY-NOTICES.txt"
  File "stage\LICENSE-AND-DISCLAIMER.txt"
  SetOutPath "$INSTDIR\node"
  File "stage\node\zakurad.exe"
  File "stage\node\zainod.exe"
  SetOutPath "$INSTDIR\params"
  File "stage\params\sapling-spend.params"
  File "stage\params\sapling-output.params"
  SetOutPath "$INSTDIR"

  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortcut "$SMPROGRAMS\Zecky Lock it.lnk" "$INSTDIR\Zecky Lock it.exe"
  CreateShortcut "$DESKTOP\Zecky Lock it.lnk" "$INSTDIR\Zecky Lock it.exe"

  WriteRegStr HKCU "Software\ZeckyLockIt" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayName" "${NAME}"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTKEY}" "Publisher" "${PUBLISHER}"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayIcon" "$INSTDIR\Zecky Lock it.exe"
  WriteRegStr HKCU "${UNINSTKEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "${UNINSTKEY}" "InstallLocation" "$INSTDIR"
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  nsExec::Exec 'taskkill /IM "Zecky Lock it.exe" /F'
  nsExec::Exec 'taskkill /IM zakurad.exe /F'
  nsExec::Exec 'taskkill /IM zainod.exe /F'
  Sleep 800
  Delete "$INSTDIR\Zecky Lock it.exe"
  Delete "$INSTDIR\WebView2Loader.dll"
  Delete "$INSTDIR\READ-ME-FIRST.txt"
  Delete "$INSTDIR\THIRD-PARTY-NOTICES.txt"
  Delete "$INSTDIR\LICENSE-AND-DISCLAIMER.txt"
  RMDir /r "$INSTDIR\node"
  RMDir /r "$INSTDIR\params"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\Zecky Lock it.lnk"
  Delete "$DESKTOP\Zecky Lock it.lnk"
  DeleteRegKey HKCU "${UNINSTKEY}"
  DeleteRegKey HKCU "Software\ZeckyLockIt"
  ; The wallet (%LOCALAPPDATA%\app.zecky.lockit) stays: it holds your sealed words.
SectionEnd
