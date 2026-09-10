;
; riviv NSIS installer (#26) — a port of upstream c-original/nsis/installer.nsi
; (voidtools/voidImageViewer, MIT; multi-language scaffolding by hespheros).
;
; Same flow as upstream: the installer itself stays RequestExecutionLevel
; user, stages the exe + this uninstaller into $pluginsdir, and lets the
; EXE's own /install pass elevate via its "runas" re-execution (see
; src/assoc.rs, upstream viv.c:4637-4658). Port choices: MUI2.nsh (upstream
; ships MUI v1), x64-only (riviv's target), the exe from cargo's release
; dir, and no Changes.txt (riviv ships none — recorded in README
; Differences).
;
; Build (from installer/nsis/):  makensis riviv.nsi
;   overrides: /DLANG=English  /DEXE_PATH="..\..\target\release\riviv.exe"
;

!verbose 3

; Language configuration (upstream: Chinese default, English switch)
!ifndef LANG
	!define LANG "Chinese"
!endif

; The release exe to ship (cargo's output).
!ifndef EXE_PATH
	!define EXE_PATH "..\..\target\release\riviv.exe"
!endif

; we need admin access to write to program files — the EXE elevates itself.
RequestExecutionLevel user

CRCCheck On
XPStyle on

; includes
!include "MUI2.nsh"

!include "version.nsh"

!include WinMessages.nsh
!include InstallOptions.nsh
!include FileFunc.nsh

; Language-specific file names
!if "${LANG}" == "Chinese"
	!define LICENSE_FILE "installer_license_Chinese.txt"
	!define INSTALL_OPTIONS_FILE "InstallOptions_Chinese.ini"
	!define INSTALL_OPTIONS2_FILE "InstallOptions2_Chinese.ini"
	!define LANG_CODE "zh-CN"
	!define LANG_NAME "Chinese"
!else
	!define LICENSE_FILE "installer_license_English.txt"
	!define INSTALL_OPTIONS_FILE "InstallOptions.ini"
	!define INSTALL_OPTIONS2_FILE "InstallOptions2.ini"
	!define LANG_CODE "en-US"
	!define LANG_NAME "English"
!endif

; riviv ships x64 only for now.
!define TARGETMACHINE "x64"
InstallDir "$PROGRAMFILES64\riviv"

; vars

Var existing_ini_filename
Var admin_install_options
Var user_install_options

BrandingText "riviv ${VERSION}${BETAVERSION} (${TARGETMACHINE}) Setup"

; settings /SOLID will save a few KBs
SetCompressor /SOLID lzma
Name "riviv"

; Output file name with language code
OutFile "riviv-${VERSION}${BETAVERSION}.${TARGETMACHINE}.${LANG_CODE}-Setup.exe"

; MUI settings
!define MUI_ICON "..\..\res\riviv.ico"
!define MUI_UNICON "..\..\res\riviv.ico"

!insertmacro MUI_PAGE_LICENSE "${LICENSE_FILE}"

!insertmacro MUI_PAGE_DIRECTORY

; options page
Page custom InstallOptions
Page custom InstallOptions2

!insertmacro MUI_PAGE_INSTFILES

!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_UNPAGE_FINISH

; Language selection
!if "${LANG}" == "Chinese"
	!insertmacro MUI_LANGUAGE "SimpChinese"
!else
	!insertmacro MUI_LANGUAGE "English"
!endif

!insertmacro GetOptions

; Version Info
VIProductVersion "${VERSION}${BETAVERSION}.0"

; don't localize these:
VIAddVersionKey "ProductName" "riviv"
VIAddVersionKey "Comments" ""
VIAddVersionKey "CompanyName" ""
VIAddVersionKey "LegalTrademarks" ""
VIAddVersionKey "LegalCopyright" "Copyright (c) ${VERSIONYEAR} riviv contributors; original C implementation (c) voidtools / David Carpenter"
VIAddVersionKey "FileDescription" "riviv Setup"

VIAddVersionKey "FileVersion" "${VERSION}${BETAVERSION}.${TARGETMACHINE}.${LANG_CODE}"
VIAddVersionKey "ProductVersion" "${VERSION}${BETAVERSION}.${TARGETMACHINE}.${LANG_CODE}"

Function .onInit

	; init options with language-specific files.
	!insertmacro INSTALLOPTIONS_EXTRACT "${INSTALL_OPTIONS_FILE}"
	!insertmacro INSTALLOPTIONS_EXTRACT "${INSTALL_OPTIONS2_FILE}"

	;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;
	; remember last install dir.
	;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;

	ClearErrors

	; use the appropriate reg view.
	; dont install to the previous x86 location C:\Program Files (x86) if we are x64

	SetRegView 64

	ReadRegStr $R2 HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\riviv" 'UninstallString'

	SetRegView 32

	IfErrors no_existing_install_dir

	ClearErrors
	system::Call 'Shlwapi::PathRemoveFileSpec(tR2R2) i.r1'
	IfErrors no_existing_install_dir

	StrCpy $INSTDIR $R2

no_existing_install_dir:

	; get the existing ini filename.
	StrCpy $existing_ini_filename "$APPDATA\riviv\riviv.ini"

	; Check if appdata is set to zero
	; $INSTDIR is the existing install location (or the default one if it does not exist)
	ReadINIStr $0 "$INSTDIR\riviv.ini" "riviv" "appdata"
    StrCmp $0 "0" 0 skip_check_app_data
	StrCpy $existing_ini_filename "$INSTDIR\riviv.ini"
	!insertmacro INSTALLOPTIONS_WRITE "${INSTALL_OPTIONS_FILE}" "Field 2" "State" "0"
	!insertmacro INSTALLOPTIONS_WRITE "${INSTALL_OPTIONS_FILE}" "Field 3" "State" "1"

skip_check_app_data:

	;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;
	; localization
	;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;

	; riviv is x64-only: check the OS is x64 capable.

	System::Call "kernel32::GetCurrentProcess() i .s"
	System::Call "kernel32::IsWow64Process(i s, *i .r0)"
	IntCmp $0 0 is32
	goto is64

is32:

	MessageBox MB_YESNOCANCEL|MB_ICONEXCLAMATION "OS is not x64.$\nInstall anyway?" IDYES is64
	Abort

is64:

FunctionEnd

Function un.onInit

	; get the existing ini filename.
	StrCpy $existing_ini_filename "$APPDATA\riviv\riviv.ini"

	; Check if appdata is set to zero
	; $INSTDIR is the existing install location
	ReadINIStr $0 "$INSTDIR\riviv.ini" "riviv" "appdata"
    StrCmp $0 "0" 0 skip_check_app_data
	StrCpy $existing_ini_filename "$INSTDIR\riviv.ini"

skip_check_app_data:

FunctionEnd

; sections
Section "riviv" SECTION_RIVIV

	; init
	StrCpy $admin_install_options ""
	StrCpy $user_install_options ""

	; app data
	!insertmacro INSTALLOPTIONS_READ $R0 "${INSTALL_OPTIONS_FILE}" "Field 2" "State"
	strcmp $R0 "0" no_app_data
	StrCpy $admin_install_options "$admin_install_options /appdata"
	Goto skip_app_data

no_app_data:

	; this option is special, we MUST unset any option that riviv was previously installed with.
	StrCpy $admin_install_options "$admin_install_options /noappdata"

skip_app_data:

	; startmenu
	!insertmacro INSTALLOPTIONS_READ $R0 "${INSTALL_OPTIONS2_FILE}" "Field 1" "State"
	strcmp $R0 "0" no_startmenu
	StrCpy $admin_install_options "$admin_install_options /startmenu"
	Goto skip_startmenu

no_startmenu:

	StrCpy $admin_install_options "$admin_install_options /nostartmenu"

skip_startmenu:

	; BMP Associations
	!insertmacro INSTALLOPTIONS_READ $R0 "${INSTALL_OPTIONS2_FILE}" "Field 3" "State"
	strcmp $R0 "0" no_bmp_association
	StrCpy $user_install_options "$user_install_options /bmp"
	Goto skip_bmp_association

no_bmp_association:

	StrCpy $user_install_options "$user_install_options /nobmp"

skip_bmp_association:

	; GIF Associations
	!insertmacro INSTALLOPTIONS_READ $R0 "${INSTALL_OPTIONS2_FILE}" "Field 4" "State"
	strcmp $R0 "0" no_gif_association
	StrCpy $user_install_options "$user_install_options /gif"
	Goto skip_gif_association

no_gif_association:

	StrCpy $user_install_options "$user_install_options /nogif"

skip_gif_association:

	; ico Associations
	!insertmacro INSTALLOPTIONS_READ $R0 "${INSTALL_OPTIONS2_FILE}" "Field 5" "State"
	strcmp $R0 "0" no_ico_association
	StrCpy $user_install_options "$user_install_options /ico"
	Goto skip_ico_association

no_ico_association:

	StrCpy $user_install_options "$user_install_options /noico"

skip_ico_association:

	; jpeg Associations
	!insertmacro INSTALLOPTIONS_READ $R0 "${INSTALL_OPTIONS2_FILE}" "Field 6" "State"
	strcmp $R0 "0" no_jpeg_association
	StrCpy $user_install_options "$user_install_options /jpeg"
	Goto skip_jpeg_association

no_jpeg_association:

	StrCpy $user_install_options "$user_install_options /nojpeg"

skip_jpeg_association:

	; jpg Associations
	!insertmacro INSTALLOPTIONS_READ $R0 "${INSTALL_OPTIONS2_FILE}" "Field 7" "State"
	strcmp $R0 "0" no_jpg_association
	StrCpy $user_install_options "$user_install_options /jpg"
	Goto skip_jpg_association

no_jpg_association:

	StrCpy $user_install_options "$user_install_options /nojpg"

skip_jpg_association:

	; png Associations
	!insertmacro INSTALLOPTIONS_READ $R0 "${INSTALL_OPTIONS2_FILE}" "Field 8" "State"
	strcmp $R0 "0" no_png_association
	StrCpy $user_install_options "$user_install_options /png"
	Goto skip_png_association

no_png_association:

	StrCpy $user_install_options "$user_install_options /nopng"

skip_png_association:

	; tif Associations
	!insertmacro INSTALLOPTIONS_READ $R0 "${INSTALL_OPTIONS2_FILE}" "Field 9" "State"
	strcmp $R0 "0" no_tif_association
	StrCpy $user_install_options "$user_install_options /tif"
	Goto skip_tif_association

no_tif_association:

	StrCpy $user_install_options "$user_install_options /notif"

skip_tif_association:

	; tiff Associations
	!insertmacro INSTALLOPTIONS_READ $R0 "${INSTALL_OPTIONS2_FILE}" "Field 10" "State"
	strcmp $R0 "0" no_tiff_association
	StrCpy $user_install_options "$user_install_options /tiff"
	Goto skip_tiff_association

no_tiff_association:

	StrCpy $user_install_options "$user_install_options /notiff"

skip_tiff_association:

	; webp Associations
	!insertmacro INSTALLOPTIONS_READ $R0 "${INSTALL_OPTIONS2_FILE}" "Field 11" "State"
	strcmp $R0 "0" no_webp_association
	StrCpy $user_install_options "$user_install_options /webp"
	Goto skip_webp_association

no_webp_association:

	StrCpy $user_install_options "$user_install_options /nowebp"

skip_webp_association:

	; ----------------------------------
	; begin riviv installation
	; ----------------------------------

	SectionIn RO

	InitPluginsDir
	SetOutPath "$pluginsdir\riviv"

	; write out files to copy.
	; the exe comes from cargo's release output (upstream: per-VS build dirs).

	File "${EXE_PATH}"

	WriteUninstaller "$pluginsdir\riviv\Uninstall.exe"

	; check for command line options that will override the default install options.
	${GetOptions} $CMDLINE "/install-options" $0
	IfErrors +2
	StrCpy $admin_install_options "$admin_install_options $0"

	; install with admin rights.
	ClearErrors
	ExecWait '"$pluginsdir\riviv\riviv.exe" /install "$INSTDIR" /install-options "$admin_install_options"' $0
	IfErrors exec_admin_error
	IntCmp $0 0 exec_admin_ok

exec_admin_error:

	MessageBox MB_OK|MB_ICONSTOP "Failed to execute admin command"

exec_admin_ok:

	ClearErrors
	ExecWait '"$INSTDIR\riviv.exe" $user_install_options' $0
	IfErrors exec_install_options_error
	IntCmp $0 0 exec_install_options_ok

exec_install_options_error:

	MessageBox MB_OK|MB_ICONSTOP "Failed to execute install options"

exec_install_options_ok:

SectionEnd

Section "Uninstall"

	; Make sure $InstDir is not the current directory so we can remove it
	SetOutPath $Temp

	; copy riviv.exe to temp folder.
	CopyFiles /SILENT $INSTDIR\riviv.exe $Temp\riviv.exe

	; run uninstaller with admin rights
	; this will uninstall any localized shortcuts etc..
	; do this before we try to terminate the app.
    ExecWait '"$Temp\riviv.exe" /uninstall "$INSTDIR"'

	; delete temp riviv
    Delete "$Temp\riviv.exe"

SectionEnd

Function InstallOptions

	!insertmacro INSTALLOPTIONS_INITDIALOG "${INSTALL_OPTIONS_FILE}"

	; upstream ships these headers English in both languages; the port
	; localizes them for the Chinese build (port deviation, PR #35).
	!if "${LANG}" == "Chinese"
		!insertmacro MUI_HEADER_TEXT "选择安装选项" "选择其他安装选项。"
	!else
		!insertmacro MUI_HEADER_TEXT "Select Install Options" "Choose any additional install options."
	!endif

	!insertmacro INSTALLOPTIONS_SHOW

FunctionEnd

Function InstallOptions2

	!insertmacro INSTALLOPTIONS_INITDIALOG "${INSTALL_OPTIONS2_FILE}"

	!if "${LANG}" == "Chinese"
		!insertmacro MUI_HEADER_TEXT "选择安装选项" "选择其他安装选项。"
	!else
		!insertmacro MUI_HEADER_TEXT "Select Install Options" "Choose any additional install options."
	!endif

	!insertmacro INSTALLOPTIONS_SHOW

FunctionEnd
