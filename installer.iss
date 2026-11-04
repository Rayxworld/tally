#define MyAppName      "Tally by Rayxworld"
#define MyAppVersion   "1.0.2"
#define MyAppPublisher "Rayxworld"
#define MyAppURL       "https://github.com/Rayxworld/tally"
#define MyAppExeName   "tally.exe"
#define MyGuiExeName   "Tally-GUI.exe"

[Setup]
AppId={{B74B1D36-9B6D-4B7C-9E1A-3A9C8F4B12D7}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
SetupIconFile=assets\Minimalist TALLY Tech Logo.ico
DefaultDirName={autopf}\Tally
DefaultGroupName=Tally
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\{#MyGuiExeName}
OutputDir=installer-output
OutputBaseFilename=Tally-Setup-{#MyAppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
PrivilegesRequired=admin
ChangesEnvironment=yes
CloseApplications=no
; Minimum Windows 10
MinVersion=10.0.17763

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Additional icons:"; Flags: unchecked

[Files]
; CLI binary
Source: "target\release\tally.exe";         DestDir: "{app}"; DestName: "{#MyAppExeName}"; Flags: ignoreversion
; Full Tauri/React desktop UI (built by `npm run build:windows` in desktop-ui)
Source: "desktop-ui\src-tauri\target\release\app.exe"; DestDir: "{app}"; DestName: "{#MyGuiExeName}"; Flags: ignoreversion

[Icons]
; Start Menu
Name: "{group}\Tally";                Filename: "{app}\{#MyGuiExeName}"; Comment: "Open the Tally file organizer"
Name: "{group}\Tally (CLI Help)";     Filename: "{cmd}"; Parameters: "/k ""{app}\{#MyAppExeName} --help"""; Comment: "Show CLI commands"
Name: "{group}\Uninstall Tally";      Filename: "{uninstallexe}"
; Desktop (optional task)
Name: "{commondesktop}\Tally";        Filename: "{app}\{#MyGuiExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyGuiExeName}"; Description: "Launch Tally now"; Flags: nowait postinstall skipifsilent

[Code]
const
  EnvironmentKey = 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment';
  TallyKey = 'Software\Rayxworld\Tally';
  ContextMenuKey = 'Software\Classes\Directory\shell\Tally';

function NormalizePathEntry(Value: string): string;
begin
  Result := Uppercase(Trim(Value));
  while (Length(Result) > 3) and
        (Result[Length(Result)] = '\') do
    Delete(Result, Length(Result), 1);
end;

function PathContainsEntry(PathValue, Entry: string): Boolean;
var
  Remaining, Part: string;
  P: Integer;
begin
  Result := False;
  Remaining := PathValue + ';';
  while Remaining <> '' do
  begin
    P := Pos(';', Remaining);
    if P = 0 then
      Break;
    Part := Copy(Remaining, 1, P - 1);
    if NormalizePathEntry(Part) = NormalizePathEntry(Entry) then
    begin
      Result := True;
      Exit;
    end;
    Delete(Remaining, 1, P);
  end;
end;

procedure AddTallyToPath;
var
  ExistingPath, InstallPath: string;
begin
  InstallPath := ExpandConstant('{app}');
  if not RegQueryStringValue(HKEY_LOCAL_MACHINE, EnvironmentKey,
    'Path', ExistingPath) then
    ExistingPath := '';

  if not PathContainsEntry(ExistingPath, InstallPath) then
  begin
    if (ExistingPath <> '') and
       (ExistingPath[Length(ExistingPath)] <> ';') then
      ExistingPath := ExistingPath + ';';
    ExistingPath := ExistingPath + InstallPath;

    if not RegWriteExpandStringValue(HKEY_LOCAL_MACHINE, EnvironmentKey,
      'Path', ExistingPath) then
      RaiseException('Could not add Tally to the system PATH.');

    RegWriteDWordValue(HKEY_LOCAL_MACHINE, TallyKey, 'AddedPath', 1);
  end
  else
    RegWriteDWordValue(HKEY_LOCAL_MACHINE, TallyKey, 'AddedPath', 0);
end;

procedure UpdateContextMenu;
var
  GuiPath: string;
begin
  if not RegKeyExists(HKEY_CURRENT_USER, ContextMenuKey) then
    Exit;

  GuiPath := ExpandConstant('{app}\{#MyGuiExeName}');
  if not RegWriteStringValue(HKEY_CURRENT_USER, ContextMenuKey, 'Icon',
    GuiPath) then
    RaiseException('Could not update the Tally Explorer menu icon.');
  if not RegWriteStringValue(HKEY_CURRENT_USER, ContextMenuKey + '\command',
    '', '"' + GuiPath + '" "%1"') then
    RaiseException('Could not update the Tally Explorer menu command.');
end;

procedure RemoveTallyFromPath;
var
  ExistingPath, NewPath, Part, InstallPath, Remaining: string;
  P: Integer;
  AddedPath: Cardinal;
begin
  if not RegQueryDWordValue(HKEY_LOCAL_MACHINE, TallyKey,
    'AddedPath', AddedPath) then
    Exit;

  if AddedPath <> 1 then
    Exit;

  InstallPath := ExpandConstant('{app}');
  if not RegQueryStringValue(HKEY_LOCAL_MACHINE, EnvironmentKey,
    'Path', ExistingPath) then
    Exit;

  NewPath := '';
  Remaining := ExistingPath + ';';
  while Remaining <> '' do
  begin
    P := Pos(';', Remaining);
    if P = 0 then
      Break;
    Part := Trim(Copy(Remaining, 1, P - 1));
    if NormalizePathEntry(Part) <> NormalizePathEntry(InstallPath) then
    begin
      if NewPath <> '' then
        NewPath := NewPath + ';';
      NewPath := NewPath + Part;
    end;
    Delete(Remaining, 1, P);
  end;

  if RegWriteExpandStringValue(HKEY_LOCAL_MACHINE, EnvironmentKey,
    'Path', NewPath) then
    RegDeleteKeyIfEmpty(HKEY_LOCAL_MACHINE, TallyKey);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
  begin
    AddTallyToPath;
    UpdateContextMenu;
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    RemoveTallyFromPath;
end;
