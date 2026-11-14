; Inno Setup installer script for Galeria on Windows (x64).
; Built in CI via: ISCC.exe /DAppVersion=<ver> packaging\windows\installer.iss

#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif

[Setup]
AppId={{D9A1E43B-8C72-4F6D-9B2E-6412E79A3F01}
AppName=Galeria
AppVersion={#AppVersion}
AppPublisher=Lazar Nikolov
AppPublisherURL=https://github.com/nikolovlazar/galeria
DefaultDirName={autopf}\Galeria
DefaultGroupName=Galeria
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
SetupIconFile=..\..\assets\icons\icon.ico
UninstallDisplayIcon={app}\galeria.exe
LicenseFile=..\..\LICENSE
OutputDir=..\..\target\bundle
OutputBaseFilename=Galeria-{#AppVersion}-windows-x64-setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\..\target\release\galeria.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\assets\icons\icon.ico"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Galeria"; Filename: "{app}\galeria.exe"; IconFilename: "{app}\icon.ico"
Name: "{autodesktop}\Galeria"; Filename: "{app}\galeria.exe"; IconFilename: "{app}\icon.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\galeria.exe"; Description: "{cm:LaunchProgram,Galeria}"; Flags: nowait postinstall skipifsilent
