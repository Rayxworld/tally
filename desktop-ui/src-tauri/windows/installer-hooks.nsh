!include "StrFunc.nsh"
${StrStr}
${UnStrRep}
!include "WinMessages.nsh"

!macro NSIS_HOOK_POSTINSTALL
  IfFileExists "$INSTDIR\tally.exe" tally_cli_ready 0
  MessageBox MB_ICONSTOP "The Tally command-line component could not be installed."
  Abort

tally_cli_ready:
  DeleteRegKey HKCU "Software\Classes\Directory\shell\Tally"
  ReadRegStr $0 HKCU "Environment" "Path"
  ${StrStr} $1 "$0" "$INSTDIR"
  StrCmp $1 "" 0 tally_path_done
  StrCmp $0 "" tally_empty_path
  WriteRegExpandStr HKCU "Environment" "Path" "$0;$INSTDIR"
  Goto tally_path_done
tally_empty_path:
  WriteRegExpandStr HKCU "Environment" "Path" "$INSTDIR"
tally_path_done:
  SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=5000
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ReadRegStr $0 HKCU "Environment" "Path"
  ${UnStrRep} $1 "$0" ";$INSTDIR" ""
  ${UnStrRep} $1 "$1" "$INSTDIR;" ""
  StrCmp $1 "$INSTDIR" 0 +2
  StrCpy $1 ""
  WriteRegExpandStr HKCU "Environment" "Path" "$1"
  DeleteRegKey HKCU "Software\Classes\Directory\shell\Tally"
  Delete "$INSTDIR\tally.exe"
  SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=5000
!macroend
