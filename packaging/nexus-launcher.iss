#ifndef AppVersion
  #define AppVersion "0.5.3-alpha"
#endif
#ifndef VersionNumbers
  #define VersionNumbers "0.5.3.0"
#endif
#define AppName "Straconis Software Dev Nexus Launcher"
#define StagePath "dist\nexus-launcher-windows-x86_64"

[Setup]
AppId=StraconisSoftware.DevNexusLauncher
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
VersionInfoVersion={#VersionNumbers}
AppPublisher=Straconis Software
AppPublisherURL=https://straconis.dev
AppSupportURL=https://github.com/Straconis/nexus-launcher/issues
AppUpdatesURL=https://github.com/Straconis/nexus-launcher/releases
DefaultDirName={localappdata}\Programs\Straconis Software\Dev Nexus Launcher
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
SourceDir=..
OutputDir=dist
OutputBaseFilename=nexus-launcher-setup-{#AppVersion}-windows-x86_64
SetupIconFile=assets\branding\nexus-launcher.ico
UninstallDisplayIcon={app}\nexus-launcher.ico
UninstallDisplayName={#AppName}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no

[Files]
Source: "{#StagePath}\nexus-launcher.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StagePath}\nexus-launcher.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StagePath}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StagePath}\LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StagePath}\LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StagePath}\OFL-Orbitron.txt"; DestDir: "{app}"; Flags: ignoreversion
; The catalog is user-editable. Preserve it on upgrades and uninstall.
Source: "{#StagePath}\catalog\apps.toml"; DestDir: "{app}\catalog"; Flags: onlyifdoesntexist uninsneveruninstall

[Icons]
Name: "{autoprograms}\Straconis Software\{#AppName}"; Filename: "{app}\nexus-launcher.exe"; WorkingDir: "{app}"; IconFilename: "{app}\nexus-launcher.ico"; Comment: "Straconis desktop tools and supported projects"

[Run]
Filename: "{app}\nexus-launcher.exe"; WorkingDir: "{app}"; Description: "Open {#AppName}"; Flags: nowait postinstall skipifsilent
