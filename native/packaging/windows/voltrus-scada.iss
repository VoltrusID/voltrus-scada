; Voltrus SCADA for one Windows PC (a local station): the operator console
; (voltrus-native.exe) and the server (voltrus.exe), which runs as the
; VoltrusSCADA Windows service bound to 127.0.0.1, so closing the console never
; stops polling, alarms, history or writes. The plant data (config.yaml,
; data.db, logs, and station.toml, the console's sign-in) lives in
; %ProgramData%\Voltrus SCADA and survives upgrades and, unless asked,
; uninstalls.
;
; Built by native/scripts/installer-windows.ps1 (Inno Setup 6.3 or later):
;   ISCC /DAppVersion=0.50.1 /DAppNumericVersion=0.50.1.0 /DBinDir=<target\release>
;        /DOutputDir=<dist> /DOutputBaseFilename=<name> voltrus-scada.iss
; The installer is not code-signed: SmartScreen warns before it runs.
;
; Silent use: setup.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART [/LOG=<file>]
;             unins000.exe /VERYSILENT /SUPPRESSMSGBOXES [/REMOVEDATA]
; Setup exits with code 10 when the station could not be set up (the files are
; installed; the reason is in <station dir>\logs\setup.log).

#if Ver < EncodeVer(6, 3, 0)
  #error Inno Setup 6.3 or later is required
#endif
#ifndef AppVersion
  #error Pass the version: ISCC /DAppVersion=x.y.z
#endif
#ifndef AppNumericVersion
  #define AppNumericVersion "0.0.0.0"
#endif
; Paths relative to this script; installer-windows.ps1 passes absolute ones.
#define RepoDir "..\..\.."
#ifndef BinDir
  #define BinDir RepoDir + "\target\release"
#endif
#ifndef OutputDir
  #define OutputDir RepoDir + "\dist"
#endif
#ifndef OutputBaseFilename
  #define OutputBaseFilename "voltrus-scada-windows-x86_64-setup"
#endif

[Setup]
AppId={{95662E10-F305-4B38-AEED-35BE71D9FE12}
AppName=Voltrus SCADA
AppVersion={#AppVersion}
AppVerName=Voltrus SCADA {#AppVersion}
AppPublisher=JoyoDigitama
AppPublisherURL=https://voltrus.id/voltrus-scada
AppSupportURL=https://github.com/VoltrusID/voltrus-scada
AppCopyright=Copyright (c) 2026 JoyoDigitama
VersionInfoVersion={#AppNumericVersion}
VersionInfoProductName=Voltrus SCADA
VersionInfoDescription=Voltrus SCADA Setup
DefaultDirName={autopf}\Voltrus SCADA
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#OutputDir}
OutputBaseFilename={#OutputBaseFilename}
SetupIconFile=voltrus.ico
UninstallDisplayIcon={app}\voltrus-native.exe
UninstallDisplayName=Voltrus SCADA
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
SetupLogging=yes
; A running console is closed for the upgrade and not restarted elevated.
CloseApplications=yes
RestartApplications=no

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "startatsignin"; Description: "Start the console when anyone signs in to Windows"; GroupDescription: "Console:"; Flags: unchecked

[Files]
Source: "{#BinDir}\voltrus-native.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinDir}\voltrus.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RepoDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RepoDir}\NOTICE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RepoDir}\THIRD-PARTY-LICENSES.md"; DestDir: "{app}"; Flags: ignoreversion
; The console's bundled third-party works, as in the macOS app bundle.
Source: "{#RepoDir}\native\packaging\licenses\OFL-JetBrainsMono.txt"; DestDir: "{app}\Licenses"; Flags: ignoreversion
Source: "{#RepoDir}\native\packaging\licenses\LICENSE-lucide.txt"; DestDir: "{app}\Licenses"; Flags: ignoreversion
Source: "{#RepoDir}\native\packaging\licenses\NOTICE-voltrus-tools.md"; DestDir: "{app}\Licenses"; Flags: ignoreversion
Source: "{#RepoDir}\native\packaging\licenses\gpui-LICENSE-APACHE.txt"; DestDir: "{app}\Licenses"; Flags: ignoreversion
Source: "{#RepoDir}\native\packaging\licenses\iTerm2-Color-Schemes-LICENSE.txt"; DestDir: "{app}\Licenses"; Flags: ignoreversion
Source: "{#RepoDir}\native\packaging\licenses\muxy-LICENSE.txt"; DestDir: "{app}\Licenses"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Voltrus SCADA"; Filename: "{app}\voltrus-native.exe"; WorkingDir: "{app}"; Comment: "Voltrus SCADA operator console"
Name: "{autodesktop}\Voltrus SCADA"; Filename: "{app}\voltrus-native.exe"; WorkingDir: "{app}"; Comment: "Voltrus SCADA operator console"; Tasks: desktopicon
Name: "{autostartup}\Voltrus SCADA"; Filename: "{app}\voltrus-native.exe"; WorkingDir: "{app}"; Comment: "Voltrus SCADA operator console"; Tasks: startatsignin

[Run]
Filename: "{app}\voltrus-native.exe"; Description: "{cm:LaunchProgram,Voltrus SCADA}"; Flags: nowait postinstall skipifsilent runasoriginaluser

[Code]
const
  { Well-known SIDs, so the grants hold on every Windows display language. }
  SidSystem = '*S-1-5-18';
  SidAdministrators = '*S-1-5-32-544';
  SidUsers = '*S-1-5-32-545';
  StationFailedExitCode = 10;

var
  StationFailed: Boolean;
  RemovePlantData: Boolean;

function StationDir: String;
begin
  Result := ExpandConstant('{commonappdata}\Voltrus SCADA');
end;

function StationLog: String;
begin
  Result := StationDir + '\logs\setup.log';
end;

{ Runs Exe with Params hidden and waits, appending its output to the station's
  logs\setup.log. True when it exited with 0. }
function RunLogged(const Exe, Params: String): Boolean;
var
  Code: Integer;
  Line: String;
begin
  ForceDirectories(StationDir + '\logs');
  Line := AddQuotes(Exe) + ' ' + Params;
  SaveStringToFile(StationLog, '> ' + Line + #13#10, True);
  Log('Running ' + Line);
  { cmd strips the outer quotes and keeps the quoted paths inside. }
  if not Exec(ExpandConstant('{cmd}'), '/C "' + Line + ' >> ' + AddQuotes(StationLog) + ' 2>&1"',
    StationDir, SW_HIDE, ewWaitUntilTerminated, Code) then
  begin
    Log(Format('Could not run %s: %s', [Exe, SysErrorMessage(Code)]));
    Result := False;
    Exit;
  end;
  Log(Format('Exit code %d', [Code]));
  Result := Code = 0;
end;

function RunVoltrus(const Exe, Command: String): Boolean;
begin
  Result := RunLogged(Exe, Command + ' --dir ' + AddQuotes(StationDir));
end;

{ The station dir: SYSTEM and Administrators only, inherited by everything the
  server writes there (data.db, logs); Users may open the folder itself and
  read station.toml (the console's local sign-in); config.yaml (with the
  session secret) keeps no inherited access at all. }
function LockDownStation: Boolean;
var
  Icacls: String;
begin
  Icacls := ExpandConstant('{sys}\icacls.exe');
  Result := False;
  if not RunLogged(Icacls, AddQuotes(StationDir) + ' /inheritance:r /grant:r "' +
    SidSystem + ':(OI)(CI)F" "' + SidAdministrators + ':(OI)(CI)F" "' + SidUsers + ':(RX)"') then
    Exit;
  if not RunLogged(Icacls, AddQuotes(StationDir + '\station.toml') + ' /grant "' + SidUsers + ':(R)"') then
    Exit;
  Result := RunLogged(Icacls, AddQuotes(StationDir + '\config.yaml') + ' /inheritance:r /grant:r "' +
    SidSystem + ':F" "' + SidAdministrators + ':F"');
end;

{ An upgrade replaces voltrus.exe, which the running service holds open. }
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Exe: String;
begin
  Result := '';
  Exe := ExpandConstant('{app}\voltrus.exe');
  if FileExists(Exe) then
    RunVoltrus(Exe, 'service stop');
end;

procedure ReportStationFailure(const Step: String);
var
  Output: AnsiString;
begin
  StationFailed := True;
  if LoadStringFromFile(StationLog, Output) then
    Log('Station setup log:' + #13#10 + Output);
  SuppressibleMsgBox('Voltrus SCADA is installed, but this computer could not be set up as a station (' +
    Step + ' failed).' + #13#10#13#10 + 'Details are in ' + StationLog + '.' + #13#10#13#10 +
    'Fix the cause, then use "Set up this computer as a station" in the console, or run Setup again.',
    mbCriticalError, MB_OK, IDOK);
end;

{ After the files: the station (config, database, the console's credential),
  its permissions, then the service, started. }
procedure CurStepChanged(CurStep: TSetupStep);
var
  Exe: String;
begin
  if CurStep <> ssPostInstall then
    Exit;
  Exe := ExpandConstant('{app}\voltrus.exe');
  if not RunVoltrus(Exe, 'station init') then
    ReportStationFailure('voltrus station init')
  else if not LockDownStation then
    ReportStationFailure('restricting access to ' + StationDir)
  else if not RunVoltrus(Exe, 'service install') then
    ReportStationFailure('voltrus service install')
  else if not RunVoltrus(Exe, 'service start') then
    ReportStationFailure('voltrus service start');
end;

function GetCustomSetupExitCode: Integer;
begin
  if StationFailed then
    Result := StationFailedExitCode
  else
    Result := 0;
end;

function UninstallParam(const Name: String): Boolean;
var
  I: Integer;
begin
  Result := False;
  for I := 1 to ParamCount do
    if CompareText(ParamStr(I), Name) = 0 then
      Result := True;
end;

{ Before the files go: the service. The plant data stays unless asked for
  (/REMOVEDATA when silent). }
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Exe: String;
begin
  case CurUninstallStep of
    usUninstall:
      begin
        Exe := ExpandConstant('{app}\voltrus.exe');
        if FileExists(Exe) then
        begin
          if not RunVoltrus(Exe, 'service stop') then
            Log('voltrus service stop failed; see ' + StationLog);
          if not RunVoltrus(Exe, 'service uninstall') then
            Log('voltrus service uninstall failed; see ' + StationLog);
        end;
        RemovePlantData := UninstallParam('/REMOVEDATA');
        if not RemovePlantData and not UninstallSilent and DirExists(StationDir) then
          RemovePlantData := MsgBox('Also remove the plant data in ' + StationDir + '?' + #13#10#13#10 +
            'This deletes the configuration, the history and alarm database, the users and the logs. ' +
            'Keep them to reinstall or upgrade later.', mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES;
      end;
    usPostUninstall:
      if RemovePlantData then
        if not DelTree(StationDir, True, True, True) then
          Log('Could not remove ' + StationDir);
  end;
end;
