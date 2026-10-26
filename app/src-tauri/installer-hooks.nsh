; Extra steps for the NSIS installer (wired up in tauri.conf.json > bundle.windows.nsis).

; Uninstalling must not leave a "launch at startup" entry pointing at a deleted exe.
!macro NSIS_HOOK_PREUNINSTALL
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "File Beaver"
!macroend
