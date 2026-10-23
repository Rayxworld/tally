#define MyAppName "Tally"
#define MyAppVersion "0.1.0"
#define MyAppPublisher "Rayxworld"
#define MyAppURL "https://github.com/Rayxworld/tally"
#define MyAppExeName "tally.exe"

[Setup]
AppId={{B74B1D36-9B6D-4B7C-9E1A-3A9C8F4B12D7}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
DefaultDirName={autopf}\Tally
DefaultGroupName=Tally
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\{#MyAppExeName}
OutputDir=installer-output
OutputBaseFilename=Tally-Setup-0.1.0
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
PrivilegesRequired=admin
ChangesEnvironment=yes
CloseApplications=no

[Files]
Source: "target\release\tally.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Tally Command Guide"; Filename: "{cmd}"; Parameters: "/k ""echo Tally installed. Run tally --help to see available commands."""
Name: "{group}\Uninstall Tally"; Filename: "{uninstallexe}"

[Code]
const
  EnvironmentKey = 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment';
  TallyKey = 'Software\Rayxworld\Tally';

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
    AddTallyToPath;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    RemoveTallyFromPath;
end;
