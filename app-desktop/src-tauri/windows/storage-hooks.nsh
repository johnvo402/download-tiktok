; Tauri includes hooks before its standard pages. Storage selection is shown
; first, independently of the application's install-directory page.
!include "nsDialogs.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"

!define /ifndef JC_STORAGE_KEY "Software\com.ai.video-localization\Storage"
Var JCDataDirectory
Var JCDataInput
Var JCDataDialog
Var JCDataProbe
Var JCDataError

LangString JCStorageTitle 1033 "App data location"
LangString JCStorageTitle 1066 "Nơi lưu dữ liệu ứng dụng"
LangString JCStorageSubtitle 1033 "Choose a writable folder with enough space for videos and AI models."
LangString JCStorageSubtitle 1066 "Chọn thư mục có quyền ghi và đủ dung lượng cho video, model AI."
LangString JCStorageDescription 1033 "Projects, settings, AI models, caches, logs and the WebView profile will be stored here. App files are installed separately. Existing data is kept when uninstalling. To reuse data, select its current folder; choosing another folder does not move existing data."
LangString JCStorageDescription 1066 "Dự án, cài đặt, model AI, cache, log và hồ sơ WebView sẽ lưu tại đây. File ứng dụng được cài riêng. Gỡ ứng dụng giữ dữ liệu này. Để dùng lại dữ liệu, chọn thư mục hiện có; chọn thư mục khác không di chuyển dữ liệu cũ."
LangString JCStorageBrowse 1033 "Browse..."
LangString JCStorageBrowse 1066 "Chọn thư mục..."
LangString JCStorageError 1033 "Choose a writable absolute data folder outside the app installation folder."
LangString JCStorageError 1066 "Hãy chọn đường dẫn dữ liệu tuyệt đối có quyền ghi, nằm ngoài thư mục cài ứng dụng."

Page custom JCStoragePage JCStoragePageLeave

Function JCStorageDefault
  ${If} $JCDataDirectory == ""
    ReadRegStr $JCDataDirectory HKCU "${JC_STORAGE_KEY}" "DataDir"
    ${If} $JCDataDirectory == ""
      ; Preserve older installations' projects when upgrading. New installs
      ; default to Documents, outside both Local and Roaming AppData.
      ${If} ${FileExists} "$APPDATA\com.ai.video-localization\*.*"
        StrCpy $JCDataDirectory "$APPDATA\com.ai.video-localization"
      ${Else}
        StrCpy $JCDataDirectory "$DOCUMENTS\JC AI"
      ${EndIf}
    ${EndIf}
    Call JCStorageArguments
  ${EndIf}
FunctionEnd

Function JCStorageArguments
  ; Windows quoting may surround either the value or the entire argument.
  ; CommandLineToArgvW handles both, including Unicode and spaces.
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  Push $5
  StrCpy $3 $CMDLINE
  System::Call 'shell32::CommandLineToArgvW(w r3, *i .r0) p.r1'
  StrCpy $5 $1
  ${If} $1 != 0
    ${DoWhile} $0 > 0
      System::Call '*$1(p .r2)'
      System::Call 'kernel32::lstrcpynW(w .r3, p r2, i ${NSIS_MAX_STRLEN})'
      StrCpy $4 $3 9
      ${If} $4 == "/DATADIR="
        StrCpy $JCDataDirectory $3 "" 9
      ${EndIf}
      IntOp $1 $1 + ${NSIS_PTR_SIZE}
      IntOp $0 $0 - 1
    ${Loop}
    System::Call 'kernel32::LocalFree(p r5)'
  ${EndIf}
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
FunctionEnd

Function JCStoragePage
  Call JCStorageDefault
  ; Tauri's /P flag skips pages while displaying installation progress.
  ${GetParameters} $0
  ClearErrors
  ${GetOptions} $0 "/P" $1
  ${IfNot} ${Errors}
    Abort
  ${EndIf}
  ; Hooks are included before MUI declares its variables; use the dialog's
  ; standard header controls instead of referencing future MUI variables.
  GetDlgItem $0 $HWNDPARENT 1037
  SendMessage $0 ${WM_SETTEXT} 0 "STR:$(JCStorageTitle)"
  GetDlgItem $0 $HWNDPARENT 1038
  SendMessage $0 ${WM_SETTEXT} 0 "STR:$(JCStorageSubtitle)"
  nsDialogs::Create 1018
  Pop $JCDataDialog
  ${If} $JCDataDialog == error
    Abort
  ${EndIf}
  ${NSD_CreateLabel} 0 0 100% 65u "$(JCStorageDescription)"
  Pop $0
  ${NSD_CreateDirRequest} 0 78u 100% 14u "$JCDataDirectory"
  Pop $JCDataInput
  ${NSD_CreateBrowseButton} 0 100u 90u 16u "$(JCStorageBrowse)"
  Pop $0
  ${NSD_OnClick} $0 JCStorageBrowse
  nsDialogs::Show
FunctionEnd

Function JCStorageBrowse
  Pop $0
  ${NSD_GetText} $JCDataInput $JCDataDirectory
  nsDialogs::SelectFolderDialog "$(JCStorageTitle)" "$JCDataDirectory"
  Pop $0
  ${If} $0 != error
    ${NSD_SetText} $JCDataInput $0
  ${EndIf}
FunctionEnd

Function JCStorageValidate
  StrCpy $JCDataError 0
  ${If} $JCDataDirectory == ""
    StrCpy $JCDataError 1
    Return
  ${EndIf}
  ; Accept drive-absolute and UNC paths, reject relative/drive-relative paths.
  StrCpy $0 $JCDataDirectory 2
  StrCpy $1 $JCDataDirectory 2 1
  ${If} $0 != "\\"
  ${AndIf} $1 != ":\"
  ${AndIf} $1 != ":/"
    StrCpy $JCDataError 1
    Return
  ${EndIf}
  ; Probe with a unique filename; never overwrite an existing user file.
  ClearErrors
  CreateDirectory "$JCDataDirectory"
  ${If} ${Errors}
    StrCpy $JCDataError 1
    Return
  ${EndIf}
  GetFullPathName $JCDataDirectory "$JCDataDirectory"
  ${If} ${Errors}
    StrCpy $JCDataError 1
    Return
  ${EndIf}
  GetTempFileName $JCDataProbe "$JCDataDirectory"
  ${If} ${Errors}
    StrCpy $JCDataError 1
  ${Else}
    Delete "$JCDataProbe"
  ${EndIf}
FunctionEnd

Function JCStoragePageLeave
  ${NSD_GetText} $JCDataInput $JCDataDirectory
  Call JCStorageValidate
  ${If} $JCDataError == 1
    MessageBox MB_OK|MB_ICONEXCLAMATION "$(JCStorageError)"
    Abort
  ${EndIf}
FunctionEnd

!macro NSIS_HOOK_PREINSTALL
  Call JCStorageDefault
  Call JCStorageValidate
  ; Data inside INSTDIR would be vulnerable to installer upgrades/uninstalls.
  GetFullPathName $0 "$INSTDIR"
  StrLen $1 $0
  StrCpy $2 $JCDataDirectory $1
  ${If} $2 == $0
    StrCpy $2 $JCDataDirectory 1 $1
    ${If} $2 == ""
    ${OrIf} $2 == "\"
    ${OrIf} $2 == "/"
      StrCpy $JCDataError 1
    ${EndIf}
  ${EndIf}
  ${If} $JCDataError == 1
    MessageBox MB_OK|MB_ICONSTOP "$(JCStorageError)" /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ClearErrors
  FileOpen $0 "$INSTDIR\jc-ai-data-dir.txt" w
  ${IfNot} ${Errors}
    FileWriteUTF16LE /BOM $0 "$JCDataDirectory$\r$\n"
    FileClose $0
  ${EndIf}
  ${If} ${Errors}
    MessageBox MB_OK|MB_ICONSTOP "$(JCStorageError)" /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
  ; Separate key survives Tauri's uninstall/reinstall during an upgrade.
  WriteRegStr HKCU "${JC_STORAGE_KEY}" "DataDir" "$JCDataDirectory"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; Remove only the pointer. User data and its registry choice are preserved.
  Delete "$INSTDIR\jc-ai-data-dir.txt"
  RMDir "$INSTDIR"
!macroend
