Unicode true
RequestExecutionLevel user
SetCompressor /SOLID lzma

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "Sections.nsh"
!include "WinVer.nsh"
!include "x64.nsh"

!define AppName "Android Device Desk"
!define RepoUrl "https://github.com/hardline7/AndroidDeviceDesk"
!define UninstallKey "Software\Microsoft\Windows\CurrentVersion\Uninstall\AndroidDeviceDesk"

Name "${AppName}"
OutFile "${SetupExe}"
InstallDir "$LOCALAPPDATA\Programs\AndroidDeviceDesk"
VIProductVersion "${AppVersion}.0"
VIAddVersionKey "ProductName" "${AppName}"
VIAddVersionKey "CompanyName" "hardline7"
VIAddVersionKey "FileDescription" "${AppName} Setup"
VIAddVersionKey "FileVersion" "${AppVersion}"
VIAddVersionKey "ProductVersion" "${AppVersion}"
VIAddVersionKey "LegalCopyright" "Copyright Android Device Desk contributors"

!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\AndroidDeviceDesk.exe"
!define MUI_FINISHPAGE_RUN_NOTCHECKED

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_COMPONENTS
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipDirectoryForInstalledVersion
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "Korean"

LangString ProgramFiles ${LANG_ENGLISH} "Program and Start menu shortcut"
LangString ProgramFiles ${LANG_KOREAN} "프로그램 및 시작 메뉴 바로가기"
LangString DesktopShortcut ${LANG_ENGLISH} "Desktop shortcut"
LangString DesktopShortcut ${LANG_KOREAN} "바탕 화면 바로가기"
LangString UnsupportedWindows ${LANG_ENGLISH} "Android Device Desk requires 64-bit Windows 10 or later."
LangString UnsupportedWindows ${LANG_KOREAN} "Android Device Desk를 설치하려면 64비트 Windows 10 이상이 필요합니다."
LangString AppInUse ${LANG_ENGLISH} "Close Android Device Desk before uninstalling it."
LangString AppInUse ${LANG_KOREAN} "제거하려면 Android Device Desk를 먼저 닫으세요."

Section "$(ProgramFiles)"
    SectionIn RO
    SetOutPath "$INSTDIR"
    File /oname=AndroidDeviceDesk.exe "${AppExe}"
    File "..\LICENSE"
    File "..\NOTICE"
    File "..\THIRD_PARTY.md"
    File /oname=licenses.zip "${LicensesZip}"
    CreateShortCut "$SMPROGRAMS\${AppName}.lnk" "$INSTDIR\AndroidDeviceDesk.exe"
    WriteUninstaller "$INSTDIR\Uninstall.exe"
    WriteRegStr HKCU "${UninstallKey}" "DisplayName" "${AppName}"
    WriteRegStr HKCU "${UninstallKey}" "DisplayVersion" "${AppVersion}"
    WriteRegStr HKCU "${UninstallKey}" "Publisher" "hardline7"
    WriteRegStr HKCU "${UninstallKey}" "URLInfoAbout" "${RepoUrl}"
    WriteRegStr HKCU "${UninstallKey}" "InstallLocation" "$INSTDIR"
    WriteRegStr HKCU "${UninstallKey}" "DisplayIcon" "$INSTDIR\AndroidDeviceDesk.exe"
    WriteRegStr HKCU "${UninstallKey}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
    WriteRegStr HKCU "${UninstallKey}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
    WriteRegStr HKCU "${UninstallKey}" "Language" "$LANGUAGE"
    WriteRegDWORD HKCU "${UninstallKey}" "NoModify" 1
    WriteRegDWORD HKCU "${UninstallKey}" "NoRepair" 1
SectionEnd

Section /o "$(DesktopShortcut)" Desktop
    CreateShortCut "$DESKTOP\${AppName}.lnk" "$INSTDIR\AndroidDeviceDesk.exe"
    WriteRegDWORD HKCU "${UninstallKey}" "DesktopShortcut" 1
SectionEnd

Function .onInit
    System::Call 'kernel32::GetUserDefaultUILanguage() i.r0'
    IntOp $0 $0 & 0x3ff
    ${If} $0 == 0x12
        StrCpy $LANGUAGE ${LANG_KOREAN}
    ${Else}
        StrCpy $LANGUAGE ${LANG_ENGLISH}
    ${EndIf}

    ${IfNot} ${RunningX64}
    ${OrIfNot} ${AtLeastWin10}
        MessageBox MB_ICONSTOP "$(UnsupportedWindows)" /SD IDOK
        Abort
    ${EndIf}
    SetRegView 64
    ReadRegStr $0 HKCU "${UninstallKey}" "InstallLocation"
    ${If} $0 != ""
        StrCpy $INSTDIR $0
    ${EndIf}
    ReadRegDWORD $0 HKCU "${UninstallKey}" "DesktopShortcut"
    ${If} $0 == 1
        !insertmacro SelectSection ${Desktop}
    ${EndIf}
FunctionEnd

Function SkipDirectoryForInstalledVersion
    ReadRegStr $0 HKCU "${UninstallKey}" "InstallLocation"
    ${If} $0 != ""
        Abort
    ${EndIf}
FunctionEnd

Function un.onInit
    SetRegView 64
    ReadRegStr $0 HKCU "${UninstallKey}" "Language"
    ${If} $0 == "${LANG_KOREAN}"
        StrCpy $LANGUAGE $0
    ${EndIf}
FunctionEnd

Section "Uninstall"
    ClearErrors
    Delete "$INSTDIR\AndroidDeviceDesk.exe"
    ${If} ${Errors}
        MessageBox MB_ICONEXCLAMATION "$(AppInUse)" /SD IDOK
        SetErrorLevel 1
        Abort
    ${EndIf}
    Delete "$SMPROGRAMS\${AppName}.lnk"
    ReadRegDWORD $0 HKCU "${UninstallKey}" "DesktopShortcut"
    ${If} $0 == 1
        Delete "$DESKTOP\${AppName}.lnk"
    ${EndIf}
    Delete "$INSTDIR\licenses.zip"
    Delete "$INSTDIR\THIRD_PARTY.md"
    Delete "$INSTDIR\NOTICE"
    Delete "$INSTDIR\LICENSE"
    Delete "$INSTDIR\Uninstall.exe"
    DeleteRegKey HKCU "${UninstallKey}"
    RMDir "$INSTDIR"
SectionEnd
