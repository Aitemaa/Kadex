; Kadex Windows installer (Inno Setup 6).
;
; Built by .github/workflows/release.yml:
;   ISCC /DAppVersion=0.1.0 /DSourceDir=<abs>\target\release /DOutputDir=<abs>\target\release packaging\windows\kadex.iss
;
; Relative paths in this script resolve from this folder, so pass absolute paths on the command line.
;
; Installs per user by default (no admin prompt) into %LOCALAPPDATA%\Programs\Kadex. The "for all
; users" choice on the first page installs into Program Files instead. Notes live in
; %APPDATA%\kadex and are never touched by the installer or uninstaller.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\..\target\release"
#endif
#ifndef OutputDir
  #define OutputDir "..\..\target\release"
#endif

[Setup]
; Never change AppId: Windows uses it to recognise upgrades and the uninstall entry.
AppId={{6034B62A-3B7C-4587-B236-F6A474C7A409}
AppName=Kadex
AppVersion={#AppVersion}
AppVerName=Kadex {#AppVersion}
AppPublisher=Ignatius Karel
AppPublisherURL=https://github.com/Aitemaa/Kadex
AppSupportURL=https://github.com/Aitemaa/Kadex/issues
AppCopyright=Copyright (c) Ignatius Karel
VersionInfoVersion={#AppVersion}
VersionInfoDescription=Kadex Setup
DefaultDirName={autopf}\Kadex
DefaultGroupName=Kadex
DisableProgramGroupPage=yes
DisableReadyPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
SetupIconFile=..\..\assets\icon.ico
UninstallDisplayIcon={app}\kadex.exe
UninstallDisplayName=Kadex
WizardStyle=modern
Compression=lzma2/ultra
SolidCompression=yes
OutputDir={#OutputDir}
OutputBaseFilename=kadex-{#AppVersion}-windows-x64-setup
; Kadex runs in the background (tray), so let Setup close it before replacing files.
CloseApplications=yes
RestartApplications=no
ChangesAssociations=yes

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked
Name: "startup"; Description: "Start Kadex when I sign in (keeps the Ctrl+Alt+N hotkey ready)"; GroupDescription: "Startup:"
Name: "openwith"; Description: "Add Kadex to ""Open with"" for text files"; GroupDescription: "Integration:"

[Files]
Source: "{#SourceDir}\kadex.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion

[Icons]
; The Start menu entry is what makes Kadex show up in Windows search.
Name: "{autoprograms}\Kadex"; Filename: "{app}\kadex.exe"; Comment: "Notepad & sticky notes"
Name: "{autodesktop}\Kadex"; Filename: "{app}\kadex.exe"; Comment: "Notepad & sticky notes"; Tasks: desktopicon
Name: "{autostartup}\Kadex"; Filename: "{app}\kadex.exe"; Comment: "Notepad & sticky notes"; Tasks: startup

[Registry]
; "kadex" in the Run dialog (Win+R).
Root: HKA; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\kadex.exe"; ValueType: string; ValueName: ""; ValueData: "{app}\kadex.exe"; Flags: uninsdeletekey
; "Open with" entry for text files.
Root: HKA; Subkey: "Software\Classes\Applications\kadex.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "Kadex"; Flags: uninsdeletekey; Tasks: openwith
Root: HKA; Subkey: "Software\Classes\Applications\kadex.exe\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\kadex.exe"",0"; Tasks: openwith
Root: HKA; Subkey: "Software\Classes\Applications\kadex.exe\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\kadex.exe"" ""%1"""; Tasks: openwith
Root: HKA; Subkey: "Software\Classes\Applications\kadex.exe\SupportedTypes"; ValueType: string; ValueName: ".txt"; ValueData: ""; Tasks: openwith
Root: HKA; Subkey: "Software\Classes\Applications\kadex.exe\SupportedTypes"; ValueType: string; ValueName: ".md"; ValueData: ""; Tasks: openwith
Root: HKA; Subkey: "Software\Classes\Applications\kadex.exe\SupportedTypes"; ValueType: string; ValueName: ".log"; ValueData: ""; Tasks: openwith
Root: HKA; Subkey: "Software\Classes\Applications\kadex.exe\SupportedTypes"; ValueType: string; ValueName: ".ini"; ValueData: ""; Tasks: openwith
Root: HKA; Subkey: "Software\Classes\.txt\OpenWithList\kadex.exe"; ValueType: string; ValueName: ""; ValueData: ""; Flags: uninsdeletekey; Tasks: openwith
Root: HKA; Subkey: "Software\Classes\.md\OpenWithList\kadex.exe"; ValueType: string; ValueName: ""; ValueData: ""; Flags: uninsdeletekey; Tasks: openwith
Root: HKA; Subkey: "Software\Classes\.log\OpenWithList\kadex.exe"; ValueType: string; ValueName: ""; ValueData: ""; Flags: uninsdeletekey; Tasks: openwith

[Run]
Filename: "{app}\kadex.exe"; Description: "Launch Kadex"; Flags: nowait postinstall skipifsilent

[Code]
// Asks a running Kadex to save its notes and exit, so its exe can be replaced or removed.
// `kadex quit` only signals the running instance, so wait a moment for it to finish.
procedure QuitRunningKadex();
var
  ExePath: String;
  ResultCode: Integer;
begin
  ExePath := ExpandConstant('{app}\kadex.exe');
  if FileExists(ExePath) then
  begin
    Exec(ExePath, 'quit', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
    Sleep(700);
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  QuitRunningKadex();
  Result := '';
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    QuitRunningKadex();
end;
