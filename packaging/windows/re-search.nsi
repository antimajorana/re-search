Unicode true

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"
!include "FileFunc.nsh"

!ifndef RE_SEARCH_VERSION
  !error "Set RE_SEARCH_VERSION to the numeric release version"
!endif
!ifndef RE_SEARCH_STAGE
  !error "Set RE_SEARCH_STAGE to the directory containing the Windows payload"
!endif
!ifndef RE_SEARCH_INSTALLER
  !error "Set RE_SEARCH_INSTALLER to the installer output path"
!endif

!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\reSearch"
!define APP_MUTEX "Local\ReSearch.Desktop.SingleInstance"
!define MUI_ICON "..\..\crates\desktop-app\assets\re-search.ico"
!define MUI_UNICON "${MUI_ICON}"

Name "reSearch"
OutFile "${RE_SEARCH_INSTALLER}"
InstallDir "$LOCALAPPDATA\Programs\reSearch"
RequestExecutionLevel user
SetCompressor zlib
VIProductVersion "${RE_SEARCH_VERSION}.0"
VIAddVersionKey "ProductName" "reSearch"
VIAddVersionKey "ProductVersion" "${RE_SEARCH_VERSION}"
VIAddVersionKey "FileVersion" "${RE_SEARCH_VERSION}"
VIAddVersionKey "FileDescription" "reSearch installer"
VIAddVersionKey "LegalCopyright" "reSearch contributors"

!insertmacro MUI_PAGE_WELCOME
!ifdef RE_SEARCH_BUNDLED_PYTHON
  !insertmacro MUI_PAGE_LICENSE "${RE_SEARCH_STAGE}\runtime\LICENSE.txt"
!endif
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_TEXT "Open reSearch from the Start menu. The desktop app uses the Microsoft Edge WebView2 Runtime."
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Var Stage
Var Backup

; Runtime paths owned by the installer. User config and research data are separate.
!macro ManagedPaths action
  !insertmacro ${action} "re-search-app.exe"
  !insertmacro ${action} "re-search.exe"
  !insertmacro ${action} "core.exe"
  !insertmacro ${action} "WebView2Loader.dll"
  !insertmacro ${action} "LICENSE"
  ; Back up the previous root license files so upgrading or rolling back is safe.
  !insertmacro ${action} "LICENSE-APACHE"
  !insertmacro ${action} "LICENSE-MIT"
  !insertmacro ${action} "python\worker.py"
  !insertmacro ${action} "python\solver_calc.py"
  !insertmacro ${action} "python\numerical_validation.py"
  !insertmacro ${action} "python\symbolic_validation.py"
  !insertmacro ${action} "python\checks"
  !insertmacro ${action} "licenses"
  !insertmacro ${action} "runtime"
  !insertmacro ${action} "uninstall.exe"
!macroend

!macro Fatal message
  MessageBox MB_OK|MB_ICONSTOP "${message}" /SD IDOK
  SetErrorLevel 1
  Abort
!macroend

!macro PrepareOperation
  SetShellVarContext current
  SetRegView 64
  ; Holding the desktop's mutex prevents another launch during file changes.
  System::Call 'kernel32::CreateMutexW(p 0, i 0, w "${APP_MUTEX}") p.r0 ?e'
  Pop $1
  ${If} $0 == 0
  ${OrIf} $1 == 183 ; ERROR_ALREADY_EXISTS
    !insertmacro Fatal "Close reSearch and any other reSearch installer, then try again."
  ${EndIf}
  StrCpy $Stage "$INSTDIR\.re-search-setup.new"
  StrCpy $Backup "$INSTDIR\.re-search-setup.old"
  ${If} ${FileExists} "$Stage"
  ${OrIf} ${FileExists} "$Backup"
    !insertmacro Fatal "Setup found files from an interrupted operation in $INSTDIR. Keep the .re-search-setup folders for recovery."
  ${EndIf}
!macroend

!macro BackupPath relative
  ${If} ${FileExists} "$INSTDIR\${relative}"
    ClearErrors
    Rename "$INSTDIR\${relative}" "$Backup\${relative}"
    IfErrors backup_failed
  ${EndIf}
!macroend

!macro RestorePath relative
  ${If} ${FileExists} "$Backup\${relative}"
    ClearErrors
    Rename "$Backup\${relative}" "$INSTDIR\${relative}"
    IfErrors restore_failed
  ${EndIf}
!macroend

!macro PublishPath relative
  ${If} ${FileExists} "$Stage\${relative}"
    ClearErrors
    Rename "$Stage\${relative}" "$INSTDIR\${relative}"
    IfErrors publish_failed
  ${EndIf}
!macroend

!macro UndoPublication relative
  ; Every installed path had already been backed up before publication began.
  ${If} ${FileExists} "$INSTDIR\${relative}"
    ClearErrors
    Rename "$INSTDIR\${relative}" "$Stage\${relative}"
    IfErrors restore_failed
  ${EndIf}
!macroend

Function .onInit
  ${IfNot} ${RunningX64}
    !insertmacro Fatal "This installer requires 64-bit Windows."
  ${EndIf}
!ifdef RE_SEARCH_BUNDLED_PYTHON
  ${If} ${Silent}
    ${GetParameters} $2
    ${GetOptions} $2 "/ACCEPT_PYTHON_LICENSE" $3
    ${If} ${Errors}
    ${OrIf} $3 != ""
      !insertmacro Fatal "A silent install requires /ACCEPT_PYTHON_LICENSE after reviewing the bundled Python license."
    ${EndIf}
  ${EndIf}
!endif
  StrCpy $INSTDIR "$LOCALAPPDATA\Programs\reSearch"
  !insertmacro PrepareOperation
FunctionEnd

Section "reSearch"
  ; Extract the complete payload on the same volume before touching installed files.
  SetOverwrite on
  ClearErrors
  SetOutPath "$Stage"
  ; Windows MakeNSIS requires backslashes when splitting source directories from file patterns.
  File "${RE_SEARCH_STAGE}\re-search-app.exe"
  File "${RE_SEARCH_STAGE}\re-search.exe"
  File "${RE_SEARCH_STAGE}\core.exe"
  File "${RE_SEARCH_STAGE}\LICENSE"
  SetOutPath "$Stage\python"
  File "${RE_SEARCH_STAGE}\python\worker.py"
  File "${RE_SEARCH_STAGE}\python\solver_calc.py"
  File "${RE_SEARCH_STAGE}\python\numerical_validation.py"
  File "${RE_SEARCH_STAGE}\python\symbolic_validation.py"
  SetOutPath "$Stage\python\checks"
  File "${RE_SEARCH_STAGE}\python\checks\__init__.py"
  File "${RE_SEARCH_STAGE}\python\checks\srepr_safe.py"
  SetOutPath "$Stage\licenses"
  File /r "${RE_SEARCH_STAGE}\licenses\*"
!ifdef RE_SEARCH_BUNDLED_PYTHON
  SetOutPath "$Stage\runtime"
  File /r "${RE_SEARCH_STAGE}\runtime\*"
!endif
  WriteUninstaller "$Stage\uninstall.exe"
  IfErrors stage_failed
  CreateDirectory "$Backup\python"
  CreateDirectory "$INSTDIR\python"
  SetOutPath "$INSTDIR"
  IfErrors stage_failed

  !insertmacro ManagedPaths BackupPath
  !insertmacro ManagedPaths PublishPath

  RMDir /r "$Stage"
  RMDir /r "$Backup"
  ClearErrors
  CreateShortCut "$SMPROGRAMS\reSearch.lnk" "$INSTDIR\re-search-app.exe"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "reSearch"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${RE_SEARCH_VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "reSearch contributors"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\re-search-app.exe"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
  IfErrors registration_failed done

publish_failed:
  !insertmacro ManagedPaths UndoPublication
backup_failed:
  !insertmacro ManagedPaths RestorePath
  SetOutPath "$INSTDIR"
  RMDir /r "$Stage"
  RMDir /r "$Backup"
  !insertmacro Fatal "Could not replace the installed files. Close any reSearch command-line or Python processes and try again."
stage_failed:
  SetOutPath "$INSTDIR"
  RMDir /r "$Stage"
  RMDir /r "$Backup"
  !insertmacro Fatal "Could not extract the reSearch installer. Check free disk space and try again."
restore_failed:
  !insertmacro Fatal "Setup could not restore every file. Keep the .re-search-setup folders in $INSTDIR for recovery."
registration_failed:
  !insertmacro Fatal "reSearch files were installed in $INSTDIR. Setup could not finish creating the shortcut or the installed-apps entry."
done:
SectionEnd

Function un.onInit
  ${If} $INSTDIR != "$LOCALAPPDATA\Programs\reSearch"
    !insertmacro Fatal "Open the reSearch uninstaller from Windows Installed apps or the reSearch installation directory."
  ${EndIf}
  !insertmacro PrepareOperation
FunctionEnd

Section "Uninstall"
  ; Move files before deleting any, so a locked executable can be handled without
  ; removing the rest of the working installation.
  ClearErrors
  CreateDirectory "$Backup\python"
  IfErrors backup_failed
  SetOutPath "$TEMP"
  !insertmacro ManagedPaths BackupPath
  RMDir /r "$Backup"
  Delete "$SMPROGRAMS\reSearch.lnk"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  RMDir "$INSTDIR\python"
  RMDir "$INSTDIR"
  Goto done

backup_failed:
  !insertmacro ManagedPaths RestorePath
  RMDir /r "$Backup"
  !insertmacro Fatal "Could not remove the installed files. Close any reSearch command-line or Python processes and try again."
restore_failed:
  !insertmacro Fatal "Setup could not restore every file. Keep .re-search-setup.old in $INSTDIR for recovery."
done:
SectionEnd
