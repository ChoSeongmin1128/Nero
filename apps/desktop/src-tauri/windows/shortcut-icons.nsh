!ifndef NERO_SHORTCUT_ICONS_NSH
!define NERO_SHORTCUT_ICONS_NSH
!include "Win\COM.nsh"
!include "${__FILEDIR__}\shortcut-icon-resource.nsh"

; Package a stable, content-addressed icon independently of Explorer's EXE cache.
!macro NeroInstallShortcutIcon
  Push $0
  StrCpy $0 $OUTDIR
  SetOutPath "$INSTDIR"
  File "/oname=${NERO_SHORTCUT_ICON_FILE}" "${NERO_SHORTCUT_ICON_SOURCE}"
  SetOutPath "$0"
  Pop $0
!macroend

; Only retire the previous resource named by our versioned manifest.
; Never accept paths, arbitrary filenames, wildcards or non-hex components.
!macro NeroRecordShortcutIcon
  Push $0
  Push $1
  Push $2
  Push $3
  ReadINIStr $0 "$INSTDIR\nero-shortcut-icon.ini" "ShortcutIcon" "formatVersion"
  ${If} $0 == "1"
    ReadINIStr $0 "$INSTDIR\nero-shortcut-icon.ini" "ShortcutIcon" "file"
    ${If} $0 != "${NERO_SHORTCUT_ICON_FILE}"
      StrLen $1 $0
      StrCpy $2 $0 10
      StrCpy $3 $0 4 -4
      ${If} $1 == 78
      ${AndIf} $2 == "nero-icon-"
      ${AndIf} $3 == ".ico"
        ReadINIStr $2 "$INSTDIR\nero-shortcut-icon.ini" "ShortcutIcon" "sha256"
        StrCpy $3 $0 64 10
        ${If} $2 == $3
          System::Call 'shlwapi::StrSpnW(w r2, w "0123456789abcdef") i.r1'
          ${If} $1 == 64
            Delete "$INSTDIR\$0"
          ${EndIf}
        ${EndIf}
      ${EndIf}
    ${EndIf}
  ${EndIf}
  WriteINIStr "$INSTDIR\nero-shortcut-icon.ini" "ShortcutIcon" "formatVersion" "1"
  WriteINIStr "$INSTDIR\nero-shortcut-icon.ini" "ShortcutIcon" "file" "${NERO_SHORTCUT_ICON_FILE}"
  WriteINIStr "$INSTDIR\nero-shortcut-icon.ini" "ShortcutIcon" "sha256" "${NERO_SHORTCUT_ICON_SHA256}"
  Pop $3
  Pop $2
  Pop $1
  Pop $0
!macroend

!macro NeroRemoveShortcutIcon
  Push $0
  Push $1
  Push $2
  Delete "$INSTDIR\${NERO_SHORTCUT_ICON_FILE}"
  ReadINIStr $0 "$INSTDIR\nero-shortcut-icon.ini" "ShortcutIcon" "formatVersion"
  ReadINIStr $1 "$INSTDIR\nero-shortcut-icon.ini" "ShortcutIcon" "file"
  ReadINIStr $2 "$INSTDIR\nero-shortcut-icon.ini" "ShortcutIcon" "sha256"
  ${If} $0 == "1"
  ${AndIf} $1 == "${NERO_SHORTCUT_ICON_FILE}"
  ${AndIf} $2 == "${NERO_SHORTCUT_ICON_SHA256}"
    Delete "$INSTDIR\nero-shortcut-icon.ini"
  ${EndIf}
  ; Upstream attempts this before POSTUNINSTALL. Only remove an empty directory.
  RMDir "$INSTDIR"
  Pop $2
  Pop $1
  Pop $0
!macroend

; Edit the existing link in place: keep arguments, working directory, AppUserModelId
; and other shell properties. Never create a missing link or modify a foreign target.
!macro NeroRefreshShortcutIcon shortcut
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  ${If} ${FileExists} "${shortcut}"
  ${AndIf} ${FileExists} "$INSTDIR\${NERO_SHORTCUT_ICON_FILE}"
    !insertmacro ComHlpr_CreateInProcInstance ${CLSID_ShellLink} ${IID_IShellLink} r0 ""
    ${If} $0 P<> 0
      ${IUnknown::QueryInterface} $0 '("${IID_IPersistFile}",.r1)'
      ${If} $1 P<> 0
        ${IPersistFile::Load} $1 '("${shortcut}", ${STGM_READWRITE}).r3'
        ${If} $3 >= 0
          ; GetPath counts characters; System::Alloc counts bytes (Unicode NSIS).
          IntOp $4 ${NSIS_MAX_STRLEN} * ${NSIS_CHAR_SIZE}
          System::Alloc $4
          Pop $2
          ${If} $2 P<> 0
            ${IShellLink::GetPath} $0 '(p r2, i${NSIS_MAX_STRLEN}, p0, i4).r3'
            System::Call '*$2(&t${NSIS_MAX_STRLEN} .r4)'
            System::Free $2
            ${If} $3 >= 0
            ${AndIf} $4 == "$INSTDIR\${NERO_APP_EXECUTABLE}"
              ${IShellLink::SetIconLocation} $0 '("$INSTDIR\${NERO_SHORTCUT_ICON_FILE}", 0).r3'
              ${If} $3 >= 0
                ${IPersistFile::Save} $1 '("${shortcut}", 1).r3'
                ${If} $3 >= 0
                  ; Refresh this item only; do not purge the system icon cache.
                  System::Call 'shell32::SHChangeNotify(i0x2000, i0x1005, w "${shortcut}", p0)'
                ${EndIf}
              ${EndIf}
            ${EndIf}
          ${EndIf}
        ${EndIf}
        ${IUnknown::Release} $1 ""
      ${EndIf}
      ${IUnknown::Release} $0 ""
    ${EndIf}
  ${EndIf}
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
!macroend

; Tauri 2.12.3 creates the interactive desktop link on the finish page, after
; POSTINSTALL. Keep its AppUserModelId setup and refresh every newly created link.
!ifmacrodef SetLnkAppUserModelId
!macroundef SetLnkAppUserModelId
!macro SetLnkAppUserModelId shortcut
  !insertmacro ComHlpr_CreateInProcInstance ${CLSID_ShellLink} ${IID_IShellLink} r0 ""
  ${If} $0 P<> 0
    ${IUnknown::QueryInterface} $0 '("${IID_IPersistFile}",.r1)'
    ${If} $1 P<> 0
      ${IPersistFile::Load} $1 '("${shortcut}", ${STGM_READWRITE})'
      ${IUnknown::QueryInterface} $0 '("${IID_IPropertyStore}",.r2)'
      ${If} $2 P<> 0
        System::Call 'Oleaut32::SysAllocString(w "${BUNDLEID}") i.r3'
        System::Call '*${SYSSTRUCT_PROPERTYKEY}(${PKEY_AppUserModel_ID})p.r4'
        System::Call '*${SYSSTRUCT_PROPVARIANT}(${VT_BSTR},,&i4 $3)p.r5'
        ${IPropertyStore::SetValue} $2 '($4,$5)'
        System::Call 'Oleaut32::SysFreeString($3)'
        System::Free $4
        System::Free $5
        ${IPropertyStore::Commit} $2 ""
        ${IUnknown::Release} $2 ""
        ${IPersistFile::Save} $1 '("${shortcut}",1)'
      ${EndIf}
      ${IUnknown::Release} $1 ""
    ${EndIf}
    ${IUnknown::Release} $0 ""
  ${EndIf}
  !insertmacro NeroRefreshShortcutIcon "${shortcut}"
!macroend
!endif

; Tauri retains existing links on /UPDATE, including target-name migrations.
; Only repair surviving, owned links; respect /NS and user-deleted shortcuts.
!macro NeroRefreshInstalledShortcutIcons desktop programs
  !insertmacro NeroRefreshShortcutIcon "${desktop}\${PRODUCTNAME}.lnk"
  !insertmacro NeroRefreshShortcutIcon "${programs}\${PRODUCTNAME}.lnk"
  !if "${STARTMENUFOLDER}" != ""
    !insertmacro NeroRefreshShortcutIcon "${programs}\$AppStartMenuFolder\${PRODUCTNAME}.lnk"
  !endif
!macroend
!ifmacrodef NSIS_HOOK_POSTINSTALL
  !error "Compose Nero's shortcut refresh with the existing post-install hook."
!endif
!macro NSIS_HOOK_POSTINSTALL
  !insertmacro NeroInstallShortcutIcon
  !insertmacro NeroRefreshInstalledShortcutIcons "$DESKTOP" "$SMPROGRAMS"
  !insertmacro NeroRecordShortcutIcon
!macroend
!ifmacrodef NSIS_HOOK_POSTUNINSTALL
  !error "Compose Nero's icon cleanup with the existing post-uninstall hook."
!endif
!macro NSIS_HOOK_POSTUNINSTALL
  !insertmacro NeroRemoveShortcutIcon
!macroend
!endif
