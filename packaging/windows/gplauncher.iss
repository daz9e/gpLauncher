; Inno Setup script for the Windows installer.
; Built in CI: iscc /DAppVersion=<version> /DBinDir=<dir with gpLauncher.exe> /DOutputName=<file name>

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef BinDir
  #define BinDir "..\..\out"
#endif
#ifndef OutputName
  #define OutputName "gplauncher-setup"
#endif

[Setup]
AppId={{6F3B2C1E-8A4D-4E7B-9C21-5D0F7A3E9B42}
AppName=gpLauncher
AppVersion={#AppVersion}
AppPublisher=gpLauncher
DefaultDirName={autopf}\gpLauncher
DefaultGroupName=gpLauncher
DisableProgramGroupPage=yes
; Per-user install by default, no admin prompt; the user may still choose all users.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\gpLauncher.exe
OutputDir=output
OutputBaseFilename={#OutputName}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#BinDir}\gpLauncher.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\gpLauncher"; Filename: "{app}\gpLauncher.exe"
Name: "{autodesktop}\gpLauncher"; Filename: "{app}\gpLauncher.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\gpLauncher.exe"; Description: "{cm:LaunchProgram,gpLauncher}"; Flags: nowait postinstall skipifsilent
