; mini-todo Windows NSIS 安装器（release.yml 的 windows 作业用 makensis 编译；写法参考 mini-term 的
; scripts/windows-installer.nsi）。⚠️ 本文件必须保存为 **UTF-8 带 BOM**：makensis 靠 BOM 认 UTF-8，没有 BOM 时按系统代码页读、
; 中文全成乱码（报 Bad text encoding）。
;
; 身份沿用旧 Tauri 版（tauri-bundler 2.9.x 的 NSIS 模板，currentUser 模式）：
; - 卸载键 HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\mini-todo，经 InstallDirRegKey 读旧
;   InstallLocation（Tauri 写的是带引号的路径，InstallDirRegKey 会去掉引号）—— 老用户默认落回原目录，
;   同名键覆盖写入，"应用和功能"里不会出现两条
; - 主程序 mini-todo.exe、开始菜单 $SMPROGRAMS\mini-todo.lnk、桌面 $DESKTOP\mini-todo.lnk，名字都不变
;   （用户固定到任务栏 / 开始屏幕的快捷方式继续有效）
; - 快捷方式登记 AppUserModelID com.tauri-app.mini-todo：应用的 Toast 通知用这个 ID
;   （crates/app/src/lib.rs 的 APP_IDENTIFIER），没有登记它的开始菜单快捷方式时系统通知可能弹不出来
;
; ⚠️ 默认安装目录 $LOCALAPPDATA\mini-todo **就是数据目录**（data.db / images / backups / holidays /
; fence-icons 都在里面，见 crates/core/src/db/paths.rs）。所以安装器 / 卸载器只增删自己的两个文件
; （mini-todo.exe、uninstall.exe），收尾的 RMDir 一律**不带 /r**，目录里还有数据时自然删不掉。
; 旧 Tauri 版的卸载器同样只删自己的文件、不递归删目录（已对照 tauri-bundler 2.9.6 模板确认）。
;
; 升级：.onInit 读注册表认出已装版本（旧 Tauri 版或本安装器装的 GPUI 版）→ 弹框告知会先卸载
; （取消即退出）→ 用户按下安装后先关掉安装目录里在跑的 mini-todo，再跑旧版自己的卸载器
; （/S /UPDATE，Tauri 卸载器认 /UPDATE：保留开机自启值，不删数据），最后铺新文件。
;
; 编译期必须 /D 传入（全部绝对路径）：
;   VERSION      完整语义版本（如 3.0.0-pre，进注册表 DisplayVersion）
;   VERSION_NUM  纯数字四段（如 3.0.0.0，VIProductVersion 只收这个）
;   SOURCE_DIR   cargo build --release 的产物目录（pc-gpui\target\release）
;   ICON_FILE    安装器图标（pc-gpui\crates\app\assets\icon.ico）
;   OUT_FILE     产物 setup.exe 输出路径

Unicode true
!include "MUI2.nsh"
!include "FileFunc.nsh"
!include "LogicLib.nsh"
!include "Sections.nsh"
!include "x64.nsh"
; SetLnkAppUserModelId 用到的 COM 接口与 PROPERTYKEY 定义（NSIS 3 自带）
!include "Win\COM.nsh"
!include "Win\Propkey.nsh"

!ifndef VERSION
  !error "makensis 需要 /DVERSION=<semver>"
!endif
!ifndef VERSION_NUM
  !error "makensis 需要 /DVERSION_NUM=<x.y.z.w>"
!endif
!ifndef SOURCE_DIR
  !error "makensis 需要 /DSOURCE_DIR=<target\release 绝对路径>"
!endif
!ifndef ICON_FILE
  !error "makensis 需要 /DICON_FILE=<icon.ico 绝对路径>"
!endif
!ifndef OUT_FILE
  !error "makensis 需要 /DOUT_FILE=<setup.exe 输出路径>"
!endif

!define PRODUCT_NAME "mini-todo"
!define MAIN_EXE "mini-todo.exe"
; Toast 通知的 AppUserModelID（= 旧 Tauri identifier，见文件头）
!define APP_USER_MODEL_ID "com.tauri-app.mini-todo"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}"
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"
; 旧 Tauri 安装器写的"厂商\产品"键（默认值 = 安装目录、Installer Language）。本安装器也照写默认值：
; 用户回退到 2.x 时，Tauri 安装器卸载已装版本用的是 `"<UninstallString>" _?=<这个默认值>`，没有它
; `_?=` 就是空的、卸载不掉（全新装 3.x 的机器上从没有过这个键）。真卸载时由本卸载器清掉
!define LEGACY_MANU_KEY "Software\tauri-app"
!define LEGACY_PRODUCT_KEY "${LEGACY_MANU_KEY}\${PRODUCT_NAME}"

; 已装旧版的现场（.onInit 探到，Section 用）：空串 = 没装过，走全新安装
Var OldUninstaller
Var OldInstallDir
Var OldVersion
Var UninstStatus
; CloseRunning 的入参：要清场的目录，`|` 分隔（Windows 路径里不会出现 `|`）
Var KillDirs

Name "${PRODUCT_NAME}"
OutFile "${OUT_FILE}"
; 用户级安装，无 UAC —— 与旧 Tauri currentUser 模式一致
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\${PRODUCT_NAME}"
InstallDirRegKey HKCU "${UNINST_KEY}" "InstallLocation"
SetCompressor /SOLID lzma
ManifestDPIAware true

VIProductVersion "${VERSION_NUM}"
VIAddVersionKey /LANG=1033 "ProductName" "${PRODUCT_NAME}"
VIAddVersionKey /LANG=1033 "ProductVersion" "${VERSION}"
VIAddVersionKey /LANG=1033 "FileVersion" "${VERSION_NUM}"
VIAddVersionKey /LANG=1033 "FileDescription" "${PRODUCT_NAME} Installer"
VIAddVersionKey /LANG=1033 "LegalCopyright" "mini-todo"

!define MUI_ICON "${ICON_FILE}"
!define MUI_UNICON "${ICON_FILE}"
!define MUI_ABORTWARNING

!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\${MAIN_EXE}"
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

; 双语：运行时按系统界面语言自动挑选，不弹选择框
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "SimpChinese"

; 升级路径的文案（LangString 必须排在 MUI_LANGUAGE 之后；串里的 $Var 运行时展开）
LangString MSG_OLD_FOUND ${LANG_ENGLISH} "${PRODUCT_NAME} $OldVersion is already installed in:$\r$\n$OldInstallDir$\r$\n$\r$\nIt will be uninstalled first, then ${VERSION} will be installed. Your to-dos, settings and images are kept.$\r$\n$\r$\nClick OK to continue, or Cancel to quit."
LangString MSG_OLD_FOUND ${LANG_SIMPCHINESE} "检测到已安装 ${PRODUCT_NAME} $OldVersion，位置：$\r$\n$OldInstallDir$\r$\n$\r$\n将先卸载它，再安装 ${VERSION}。你的待办、设置与图片不会被删除。$\r$\n$\r$\n点「确定」继续，点「取消」退出安装。"
LangString MSG_UNINST_RUN ${LANG_ENGLISH} "Uninstalling ${PRODUCT_NAME} $OldVersion from $OldInstallDir ..."
LangString MSG_UNINST_RUN ${LANG_SIMPCHINESE} "正在卸载旧版 ${PRODUCT_NAME} $OldVersion（$OldInstallDir）..."
LangString MSG_UNINST_FAIL ${LANG_ENGLISH} "The old version was not fully removed (uninstaller exit code: $UninstStatus).$\r$\n$\r$\nInstall ${VERSION} anyway (overwriting the existing files)?"
LangString MSG_UNINST_FAIL ${LANG_SIMPCHINESE} "旧版本没有卸载干净（卸载器退出码：$UninstStatus）。$\r$\n$\r$\n仍要继续安装 ${VERSION} 吗（直接覆盖现有文件）？"

LangString SEC_MAIN_NAME ${LANG_ENGLISH} "${PRODUCT_NAME} (required)"
LangString SEC_MAIN_NAME ${LANG_SIMPCHINESE} "${PRODUCT_NAME} 主程序（必装）"
LangString SEC_MAIN_DESC ${LANG_ENGLISH} "The application and its Start Menu shortcut."
LangString SEC_MAIN_DESC ${LANG_SIMPCHINESE} "主程序与开始菜单快捷方式。"
LangString SEC_DESKTOP_NAME ${LANG_ENGLISH} "Desktop shortcut"
LangString SEC_DESKTOP_NAME ${LANG_SIMPCHINESE} "桌面快捷方式"
LangString SEC_DESKTOP_DESC ${LANG_ENGLISH} "Create a ${PRODUCT_NAME} shortcut on the desktop."
LangString SEC_DESKTOP_DESC ${LANG_SIMPCHINESE} "在桌面上创建 ${PRODUCT_NAME} 快捷方式。"

LangString MSG_APP_RUNNING ${LANG_ENGLISH} "${PRODUCT_NAME} is running from the installation folder and has to be closed to continue.$\r$\n$\r$\nClick Retry to let Setup close it (your data is saved as you go, nothing is lost). You can also exit it from the tray icon first, then click Retry.$\r$\n$\r$\nClick Cancel to quit Setup."
LangString MSG_APP_RUNNING ${LANG_SIMPCHINESE} "安装目录中的 ${PRODUCT_NAME} 正在运行，继续安装需要先关闭它。$\r$\n$\r$\n点「重试」由安装器关闭它（数据随改随存，不会丢失）。也可以先从托盘图标退出它，再点「重试」。$\r$\n$\r$\n点「取消」退出安装。"
LangString MSG_APP_RUNNING_UNINST ${LANG_ENGLISH} "${PRODUCT_NAME} is running and has to be closed before it can be uninstalled.$\r$\n$\r$\nClick Retry to let the uninstaller close it. You can also exit it from the tray icon first, then click Retry.$\r$\n$\r$\nClick Cancel to quit."
LangString MSG_APP_RUNNING_UNINST ${LANG_SIMPCHINESE} "${PRODUCT_NAME} 正在运行，卸载前需要先关闭它。$\r$\n$\r$\n点「重试」由卸载程序关闭它。也可以先从托盘图标退出它，再点「重试」。$\r$\n$\r$\n点「取消」退出卸载。"
LangString MSG_CLOSING_RUNNING ${LANG_ENGLISH} "Closing ${PRODUCT_NAME} running from $KillDirs ..."
LangString MSG_CLOSING_RUNNING ${LANG_SIMPCHINESE} "正在关闭 $KillDirs 下运行的 ${PRODUCT_NAME}..."
LangString MSG_CLOSE_UNAVAILABLE ${LANG_ENGLISH} "Could not check for running programs (PowerShell returned: $1). If a file is reported in use, exit ${PRODUCT_NAME} from the tray icon and retry."
LangString MSG_CLOSE_UNAVAILABLE ${LANG_SIMPCHINESE} "没能检查运行中的程序（PowerShell 返回：$1）。如提示文件被占用，请先从托盘图标退出 ${PRODUCT_NAME} 再重试。"

; ── 关闭安装目录下运行中的实例 ───────────────────────────────────────
;
; 升级 / 卸载要替换或删除 mini-todo.exe。口径：
; - **只动路径落在清场目录下的 mini-todo.exe**：安装目录；升级且用户改选了新目录时再加上旧版目录
;   （旧 Tauri 版的进程名同样是 mini-todo.exe）。dev 构建（target\...）、便携版副本一概不碰。
;   例外：从 2.x 升级那一次，旧 Tauri 卸载器自己的 CheckIfAppIsRunning 按进程名结束当前用户**所有**
;   mini-todo.exe（含便携版 / dev 构建），那是旧卸载器的行为，这里管不到
; - 直接结束进程，不发 WM_CLOSE：关主窗口只有在没有别的窗口时才退出进程（桌面收纳箱 / 编辑窗口开着时进程
;   继续，见 windows/main_window.rs），而且 Process.CloseMainWindow 只认可见、无 owner 的顶层窗口 —— 固定 /
;   嵌入桌面模式下主窗口有 owner、贴边隐藏时不可见，发不到它。数据每次操作即写入 SQLite，强制结束不丢数据
; - 交互安装 / 卸载：探到在跑先弹框（重试 = 由安装器关闭后继续；取消 = 退出）；静默（/S）不弹框直接关
;
; 实现走 PowerShell（-Command 的参数不受执行策略约束）。输入经环境变量传，不拼进命令行：
;   MTD_KILL_DIRS  清场目录，`|` 分隔
;   MTD_KILL_MODE  probe = 只数实例个数作退出码；close = 结束它们并等待真正退出（释放文件句柄）
; 进程路径取 Process.Path，取不到退回 CIM 的 ExecutablePath；目录补尾部 `\` 后按前缀比（OrdinalIgnoreCase），
; 不用 -like —— 路径里的 [ ] 会被当成通配符。
; ⚠️ 整条脚本进 nsExec 的命令行，受 NSIS_MAX_STRLEN（官方构建 1024 字符）限制，只能写紧。
!define CLOSE_PS_1 "$$d=@($$env:MTD_KILL_DIRS -split '\|' | ?{$$_} | %{$$_.TrimEnd('\')+'\'});"
!define CLOSE_PS_2 "$$m=@(Get-Process -Name 'mini-todo' -EA 0 | ?{$$p=$$_.Path;if(!$$p){$$p=(Get-CimInstance Win32_Process -Filter ('ProcessId='+$$_.Id) -EA 0).ExecutablePath};$$h=0;foreach($$x in $$d){if($$p -and $$p.StartsWith($$x,'OrdinalIgnoreCase')){$$h=1}};$$h});"
!define CLOSE_PS_3 "if($$env:MTD_KILL_MODE -eq 'probe'){exit $$m.Count};$$m | Stop-Process -Force -EA 0;$$m | Wait-Process -Timeout 10 -EA 0;exit 0"

; 用 64 位 PowerShell 跑上面那条脚本，退出码（起不来是 "error"，超时是 "timeout"）进 ${OUT}。
; 安装器是 32 位进程，不关文件系统重定向的话起的是 32 位 PowerShell，读不到 64 位进程的路径
!macro RUN_CLOSE_PS OUT
  ${If} ${RunningX64}
    ${DisableX64FSRedirection}
  ${EndIf}
  nsExec::Exec /TIMEOUT=60000 `"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -Command "${CLOSE_PS_1}${CLOSE_PS_2}${CLOSE_PS_3}"`
  Pop ${OUT}
  ${If} ${RunningX64}
    ${EnableX64FSRedirection}
  ${EndIf}
!macroend

; 清场 $KillDirs 下的 mini-todo（口径见上）。安装器与卸载器各一份（卸载器里的函数必须以 un. 开头）
!macro DEFINE_CLOSE_RUNNING UN MSG
Function ${UN}CloseRunning
  Push $0
  Push $1
  StrCpy $0 $KillDirs
  ; r0 = 取 $0 的值，不把路径拼进调用串（路径里的括号会搅乱 System 插件的解析）
  System::Call 'Kernel32::SetEnvironmentVariable(t "MTD_KILL_DIRS", t r0)'
  ${IfNot} ${Silent}
    System::Call 'Kernel32::SetEnvironmentVariable(t "MTD_KILL_MODE", t "probe")'
    !insertmacro RUN_CLOSE_PS $1
    ; $1 = 实例数。PowerShell 起不来时是 "error"，`>` 按整数比当 0 —— 探不到就不拦，真有文件被占用时
    ; NSIS 自己的写文件失败框（重试 / 忽略）会接住
    ${If} $1 > 0
      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$(${MSG})" /SD IDRETRY IDRETRY +2
        Abort
    ${EndIf}
  ${EndIf}
  DetailPrint "$(MSG_CLOSING_RUNNING)"
  System::Call 'Kernel32::SetEnvironmentVariable(t "MTD_KILL_MODE", t "close")'
  !insertmacro RUN_CLOSE_PS $1
  ${If} $1 != "0"
    DetailPrint "$(MSG_CLOSE_UNAVAILABLE)"
  ${EndIf}
  Pop $1
  Pop $0
FunctionEnd
!macroend
!insertmacro DEFINE_CLOSE_RUNNING "" MSG_APP_RUNNING
!insertmacro DEFINE_CLOSE_RUNNING "un." MSG_APP_RUNNING_UNINST

; 给快捷方式登记 AppUserModelID（照搬 tauri-bundler NSIS 模板 utils.nsh 的 SetLnkAppUserModelId）。
; 用到 $0~$5，调用方不依赖这几个寄存器
!macro SetLnkAppUserModelId shortcut
  !insertmacro ComHlpr_CreateInProcInstance ${CLSID_ShellLink} ${IID_IShellLink} r0 ""
  ${If} $0 P<> 0
    ${IUnknown::QueryInterface} $0 '("${IID_IPersistFile}",.r1)'
    ${If} $1 P<> 0
      ${IPersistFile::Load} $1 '("${shortcut}", ${STGM_READWRITE})'
      ${IUnknown::QueryInterface} $0 '("${IID_IPropertyStore}",.r2)'
      ${If} $2 P<> 0
        System::Call 'Oleaut32::SysAllocString(w "${APP_USER_MODEL_ID}") i.r3'
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
!macroend

; 跑旧版自己的卸载器。带 `_?=` 是关键：没有它，NSIS 卸载器会先把自己复制到 %TEMP% 再启动，
; ExecWait 等到的是那个立即返回的壳，新文件会和卸载动作打架。代价是运行中的 uninstall.exe 删不掉自己，
; 残留由本宏补删。`_?=` 必须是最后一个参数（它把后面整段当目录，允许带空格）。
; /UPDATE：旧 Tauri 卸载器据此保留开机自启值（HKCU Run\mini-todo，新版启动时会按自己的路径刷新）；
; 本安装器写出的卸载器同样认它（见 Section "Uninstall"）。
; 卸载器会删掉 Uninstall 注册表键（Tauri 版还会留着开始菜单 / 桌面快捷方式）：这些由 SecMain / SecDesktop
; 原样重建。
!macro UNINSTALL_OLD
  ${If} $OldUninstaller != ""
    DetailPrint "$(MSG_UNINST_RUN)"
    ClearErrors
    ExecWait '"$OldUninstaller" /S /UPDATE _?=$OldInstallDir' $UninstStatus
    ${If} ${Errors}
      StrCpy $UninstStatus "-1"
    ${EndIf}
    Delete "$OldUninstaller"
    ; 只删空目录：默认安装目录同时是数据目录，里面有数据时不会被删
    RMDir "$OldInstallDir"
    ${If} $UninstStatus != "0"
      ; 卸载失败不直接判死：静默安装默认继续覆盖，交互装由用户定
      MessageBox MB_YESNO|MB_ICONEXCLAMATION "$(MSG_UNINST_FAIL)" /SD IDYES IDYES +2
        Abort
    ${EndIf}
  ${EndIf}
!macroend

; 主程序：SectionIn RO = 组件页上灰掉、永远勾着
Section "$(SEC_MAIN_NAME)" SecMain
  SectionIn RO
  ; 先清场再卸载：旧卸载器同样要删 mini-todo.exe。用户在目录页改选了新目录时，旧目录里在跑的也要一起清
  StrCpy $KillDirs "$INSTDIR"
  ${If} $OldInstallDir != ""
  ${AndIf} $OldInstallDir != "$INSTDIR"
    StrCpy $KillDirs "$INSTDIR|$OldInstallDir"
  ${EndIf}
  Call CloseRunning
  !insertmacro UNINSTALL_OLD

  SetOutPath "$INSTDIR"
  File "${SOURCE_DIR}\${MAIN_EXE}"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  CreateShortcut "$SMPROGRAMS\${PRODUCT_NAME}.lnk" "$INSTDIR\${MAIN_EXE}"
  !insertmacro SetLnkAppUserModelId "$SMPROGRAMS\${PRODUCT_NAME}.lnk"
  ; 桌面快捷方式交给 SecDesktop：旧 Tauri 卸载器在 /UPDATE 下不删它，这里先删掉，没勾选就不再出现
  Delete "$DESKTOP\${PRODUCT_NAME}.lnk"

  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${PRODUCT_NAME}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\${MAIN_EXE}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "mini-todo"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
  ; 回退到 2.x 时 Tauri 安装器靠它定位卸载目录（见 LEGACY_MANU_KEY 的注释）
  WriteRegStr HKCU "${LEGACY_PRODUCT_KEY}" "" "$INSTDIR"
  ; 只算本程序的两个文件（安装目录可能同时是数据目录，按目录算会把数据也算进"程序大小"）
  ${GetSize} "$INSTDIR" "/M=${MAIN_EXE} /S=0K /G=0" $0 $1 $2
  ${GetSize} "$INSTDIR" "/M=uninstall.exe /S=0K /G=0" $3 $1 $2
  IntOp $0 $0 + $3
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKCU "${UNINST_KEY}" "EstimatedSize" $0
SectionEnd

; 桌面快捷方式：可选，全新安装默认勾选；升级时的默认值见 .onInit
Section "$(SEC_DESKTOP_NAME)" SecDesktop
  CreateShortcut "$DESKTOP\${PRODUCT_NAME}.lnk" "$INSTDIR\${MAIN_EXE}"
  !insertmacro SetLnkAppUserModelId "$DESKTOP\${PRODUCT_NAME}.lnk"
SectionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecMain} "$(SEC_MAIN_DESC)"
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} "$(SEC_DESKTOP_DESC)"
!insertmacro MUI_FUNCTION_DESCRIPTION_END

; 认出已装版本并征得同意。放 .onInit 是为了让用户在第一屏就知道要先卸载；真正的卸载动作留到 Section
; （用户在目录页仍可反悔，反悔时旧版原封不动）。排在 Section 之后是因为它要引用 ${SecDesktop}
Function .onInit
  ${If} ${RunningX64}
    SetRegView 64
  ${EndIf}
  ReadRegStr $0 HKCU "${UNINST_KEY}" "UninstallString"
  ${If} $0 == ""
    Return
  ${EndIf}
  ; 注册表里的 UninstallString 是带引号的命令行，剥成裸路径才能喂 ExecWait
  StrCpy $1 $0 1
  StrCpy $2 $0 "" -1
  ${If} $1 == '"'
  ${AndIf} $2 == '"'
    StrCpy $0 $0 -1 1
  ${EndIf}
  ${IfNot} ${FileExists} "$0"
    ; 注册表在、卸载器不在（用户手删过目录）：当全新安装处理
    Return
  ${EndIf}
  StrCpy $OldUninstaller "$0"
  ; `_?=` 要的是卸载器自己所在的目录，所以取它的父目录而不是注册表里的 InstallLocation
  ; （Tauri 写的那条带引号）
  ${GetParent} "$0" $OldInstallDir
  ReadRegStr $OldVersion HKCU "${UNINST_KEY}" "DisplayVersion"
  ${If} $OldVersion == ""
    StrCpy $OldVersion "?"
  ${EndIf}
  ${IfNot} ${Silent}
    MessageBox MB_OKCANCEL|MB_ICONINFORMATION "$(MSG_OLD_FOUND)" /SD IDOK IDOK +2
      Abort
  ${EndIf}
  ; 升级沿用上一次的选择：旧版装过桌面快捷方式就默认勾上，没有就默认不勾。此刻旧版还没卸，桌面上那个
  ; .lnk 就是现场
  ${IfNot} ${FileExists} "$DESKTOP\${PRODUCT_NAME}.lnk"
    !insertmacro UnselectSection ${SecDesktop}
  ${EndIf}
FunctionEnd

Function un.onInit
  ${If} ${RunningX64}
    SetRegView 64
  ${EndIf}
FunctionEnd

Section "Uninstall"
  ; 升级时由新安装器以 /S /UPDATE 调起：那边已经清过场，这里多半什么都探不到
  StrCpy $KillDirs "$INSTDIR"
  Call un.CloseRunning

  Delete "$INSTDIR\${MAIN_EXE}"
  Delete "$INSTDIR\uninstall.exe"
  ; ⚠️ 不带 /r：默认安装目录同时是数据目录（见文件头），有数据时这一步什么也不删
  RMDir "$INSTDIR"

  Delete "$SMPROGRAMS\${PRODUCT_NAME}.lnk"
  Delete "$DESKTOP\${PRODUCT_NAME}.lnk"
  DeleteRegKey HKCU "${UNINST_KEY}"

  ; 真卸载（不是新安装器带 /UPDATE 调起的升级）才摘开机自启值（与 Tauri 卸载器同口径）与旧 Tauri 的厂商键
  ; （Tauri 只在勾了"删除应用数据"时删它；本安装器每次都会重写，真卸载就一并清掉）。
  ; 用户数据（数据目录、日志目录、收纳箱文件夹）一概不动
  ${un.GetParameters} $0
  ClearErrors
  ${un.GetOptions} $0 "/UPDATE" $1
  ${If} ${Errors}
    DeleteRegValue HKCU "${RUN_KEY}" "${PRODUCT_NAME}"
    DeleteRegKey HKCU "${LEGACY_PRODUCT_KEY}"
    DeleteRegKey /ifempty HKCU "${LEGACY_MANU_KEY}"
  ${EndIf}
SectionEnd
