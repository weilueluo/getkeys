; Puts the bundled `getkeys` command ($INSTDIR\bin\getkeys.exe) on the user's PATH.
; The welcome page offers it as a checkbox, checked by default; passive and silent installs add it.

!ifndef WM_SETTINGCHANGE
  !define WM_SETTINGCHANGE 0x001A
!endif

Var GetKeysPathBox
Var GetKeysSkipPath

; Consumed by the template's first page, the welcome page.
!define MUI_PAGE_CUSTOMFUNCTION_SHOW GetKeysWelcomeShow
!define MUI_PAGE_CUSTOMFUNCTION_LEAVE GetKeysWelcomeLeave

Function GetKeysWelcomeShow
  ${NSD_CreateCheckbox} 120u 172u 195u 12u "Add the getkeys command to PATH"
  Pop $GetKeysPathBox
  SetCtlColors $GetKeysPathBox "" "FFFFFF"
  ${If} $GetKeysSkipPath != 1
    ${NSD_Check} $GetKeysPathBox
  ${EndIf}
FunctionEnd

Function GetKeysWelcomeLeave
  Push $0
  ${NSD_GetState} $GetKeysPathBox $0
  ${If} $0 == ${BST_CHECKED}
    StrCpy $GetKeysSkipPath ""
  ${Else}
    StrCpy $GetKeysSkipPath 1
  ${EndIf}
  Pop $0
FunctionEnd

; Adds (`add`) or removes (`remove`) $INSTDIR\bin in the user PATH. PowerShell edits the registry
; value directly: NSIS strings stop at 1024 characters, and %VARIABLES% must stay unexpanded.
!macro GETKEYS_PATH MODE
  System::Call 'Kernel32::SetEnvironmentVariable(t "GETKEYS_BIN", t "$INSTDIR\bin")i'
  System::Call 'Kernel32::SetEnvironmentVariable(t "GETKEYS_PATH", t "${MODE}")i'
  Push $0
  nsExec::Exec `"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "$$d = $$env:GETKEYS_BIN; $$k = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment'); $$p = [string]$$k.GetValue('Path', '', 'DoNotExpandEnvironmentNames'); $$e = @($$p -split ';' | Where-Object { $$_ -and $$_.TrimEnd('\') -ne $$d }); if ($$env:GETKEYS_PATH -eq 'add') { $$e += $$d }; $$n = $$e -join ';'; if ($$n -ne $$p) { if ($$n) { $$k.SetValue('Path', $$n, 'ExpandString') } else { $$k.DeleteValue('Path', $$false) } }"`
  Pop $0
  Pop $0
  SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=5000
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ${If} $GetKeysSkipPath != 1
  ${AndIf} ${FileExists} "$INSTDIR\bin\getkeys.exe"
    !insertmacro GETKEYS_PATH add
  ${Else}
    !insertmacro GETKEYS_PATH remove
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  !insertmacro GETKEYS_PATH remove
!macroend
