Unicode true
!include "MUI2.nsh"

!ifndef VERSION
  !error "VERSION is required"
!endif
!ifndef PAYLOAD
  !error "PAYLOAD is required"
!endif
!ifndef OUTPUT
  !error "OUTPUT is required"
!endif

Name "蜡笔小新 (Rust)"
OutFile "${OUTPUT}"
InstallDir "$LOCALAPPDATA\Programs\crayon-shin-chan-rust"
InstallDirRegKey HKCU "Software\CrayonShinChan\Rust" "InstallLocation"
RequestExecutionLevel user
SetCompressor /SOLID lzma
SetCompressorDictSize 32
VIProductVersion "${VERSION}.0"
VIAddVersionKey /LANG=2052 "ProductName" "蜡笔小新 (Rust)"
VIAddVersionKey /LANG=2052 "FileDescription" "蜡笔小新 Rust 安装程序"
VIAddVersionKey /LANG=2052 "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=2052 "LegalCopyright" "Copyright © 2026 byqianye"
!define MUI_ICON "..\..\build\icon.ico"
!define MUI_UNICON "..\..\build\icon.ico"
!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"

Section "Install"
  SetShellVarContext current
  SetOutPath "$INSTDIR"
  File /r "${PAYLOAD}\*.*"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateDirectory "$SMPROGRAMS\蜡笔小新 (Rust)"
  CreateShortcut "$SMPROGRAMS\蜡笔小新 (Rust)\蜡笔小新.lnk" "$INSTDIR\crayon-rust.exe" "" "$INSTDIR\icon.ico"
  CreateShortcut "$DESKTOP\蜡笔小新.lnk" "$INSTDIR\crayon-rust.exe" "" "$INSTDIR\icon.ico"
  WriteRegStr HKCU "Software\CrayonShinChan\Rust" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\CrayonShinChanRust" "DisplayName" "蜡笔小新 (Rust) ${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\CrayonShinChanRust" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\CrayonShinChanRust" "Publisher" "byqianye"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\CrayonShinChanRust" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\CrayonShinChanRust" "DisplayIcon" "$INSTDIR\icon.ico"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\CrayonShinChanRust" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\CrayonShinChanRust" "QuietUninstallString" '$\"$INSTDIR\Uninstall.exe$\" /S'
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\CrayonShinChanRust" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\CrayonShinChanRust" "NoRepair" 1
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  Delete "$DESKTOP\蜡笔小新.lnk"
  Delete "$SMPROGRAMS\蜡笔小新 (Rust)\蜡笔小新.lnk"
  RMDir "$SMPROGRAMS\蜡笔小新 (Rust)"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\CrayonShinChanRust"
  DeleteRegKey HKCU "Software\CrayonShinChan\Rust"
  Delete "$INSTDIR\crayon-rust.exe"
  Delete "$INSTDIR\WebView2Loader.dll"
  Delete "$INSTDIR\icon.ico"
  Delete "$INSTDIR\version.json"
  Delete "$INSTDIR\FILES.sha256"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\THIRD-PARTY-NOTICES.txt"
  RMDir /r "$INSTDIR\spider-host"
  RMDir /r "$INSTDIR\licenses"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
SectionEnd
