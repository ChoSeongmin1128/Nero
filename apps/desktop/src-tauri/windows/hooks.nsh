; Tauri CLI 2.12.3 includes utils.nsh before installerHooks. Replace its
; force-shutdown helper for both installation and uninstallation. In particular,
; a silent update must not terminate an active recording or a saving worker.
!ifmacrondef CheckIfAppIsRunning
  !error "Review Nero's running-app protection after changing the Tauri NSIS template."
!endif
!macroundef CheckIfAppIsRunning

!macro CheckIfAppIsRunning executablePath productName
  ${If} ${FileExists} "${executablePath}"
    ; Official updater starts this installer just before the old process exits.
    ; Only /UPDATE gets a bounded grace period; never terminate any process.
    StrCpy $R2 1
    ClearErrors
    ${GetOptions} $CMDLINE "/UPDATE" $R3
    ${IfNot} ${Errors}
      StrCpy $R2 50
    ${EndIf}
    ClearErrors
    ${Do}
    !insertmacro RestartManager_StartSession $R0
    ${If} $R0 == ""
      SetErrorLevel 2
      Abort "${productName} 실행 상태를 확인하지 못했습니다. 앱을 종료한 뒤 다시 실행해 주세요."
    ${EndIf}
    !insertmacro RestartManager_RegisterFile $R0 "${executablePath}"
    ${If} $0 != 0
      !insertmacro RestartManager_EndSession $R0
      SetErrorLevel 2
      Abort "${productName} 실행 상태를 확인하지 못했습니다. 앱을 종료한 뒤 다시 실행해 주세요."
    ${EndIf}
    System::Call 'RSTRTMGR::RmGetList(p R0, *i .r1, *i .r2, p 0, *i .r3) i .r0'
    ; Preserve the RmGetList result before ending the session.
    StrCpy $R1 $0
    !insertmacro RestartManager_EndSession $R0
    ${If} $R1 == 0
      ${ExitDo}
    ${EndIf}
    ${If} $R1 == 234
    ${AndIf} $R2 > 1
      IntOp $R2 $R2 - 1
      Sleep 100
    ${Else}
      ${ExitDo}
    ${EndIf}
    ${Loop}
    ${If} $R1 != 0
      SetErrorLevel 2
      Abort "${productName}가 실행 중이거나 실행 상태를 확인하지 못했습니다. 녹음과 처리를 마친 뒤 앱을 종료하고 다시 실행해 주세요."
    ${EndIf}
  ${EndIf}
!macroend

; Maintenance may invoke an older uninstaller before the Install section. Check
; before showing that page too. Silent mode skips pages and uses the helper above.
; This filename is checked against Cargo/config by test_windows_installer.py.
!define NERO_APP_EXECUTABLE "nero-desktop.exe"
!ifdef MUI_CUSTOMFUNCTION_GUIINIT
  !error "Compose Nero's pre-maintenance check with the existing GUI initializer."
!endif
!define MUI_CUSTOMFUNCTION_GUIINIT NeroCheckBeforeMaintenance
Function NeroCheckBeforeMaintenance
  !insertmacro CheckIfAppIsRunning "$INSTDIR\${NERO_APP_EXECUTABLE}" "Nero"
FunctionEnd
