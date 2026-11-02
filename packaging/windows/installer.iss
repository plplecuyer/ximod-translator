; Installateur Windows de XIMOD Translator (S-02, R-S02-03).
; Compilé par le workflow de livraison :
;   ISCC.exe /DAppVersion=1.2.3 /DStageDir=<dossier livré> /DOutDir=<sortie> installer.iss
; Installation par défaut dans Program Files (mode utilisateur proposé sans
; droits administrateur), raccourcis Menu Démarrer et bureau (option),
; association .ximdb facultative, mise à jour par-dessus (AppId fixe),
; désinstallation limitée au dossier d'installation : le dossier de
; configuration de l'utilisateur (%APPDATA%\XIMOD Translator) survit.
; Aucune base vanilla n'est livrée (D-06).

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef StageDir
  #define StageDir "..\..\dist\stage"
#endif
#ifndef OutDir
  #define OutDir "..\..\dist\out"
#endif

[Setup]
AppId={{7B4E6C1A-3F2D-4A8B-9C5E-2D1F0A9B8C7E}
AppName=XIMOD Translator
AppVersion={#AppVersion}
AppVerName=XIMOD Translator {#AppVersion}
AppPublisher=XIMOD Team
AppPublisherURL=https://github.com/ximod/ximod-translator
DefaultDirName={autopf}\XIMOD Translator
DefaultGroupName=XIMOD Translator
DisableProgramGroupPage=yes
PrivilegesRequired=admin
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutDir}
OutputBaseFilename=XIMOD-Translator-{#AppVersion}-setup
SetupIconFile={#StageDir}\assets\icons\ximod-translator.ico
UninstallDisplayIcon={app}\ximod-translator.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ChangesAssociations=yes

[Languages]
Name: "french"; MessagesFile: "compiler:Languages\French.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "assocximdb"; Description: "Associer les dictionnaires .ximdb à XIMOD Translator"; GroupDescription: "Associations :"; Flags: unchecked

[Files]
Source: "{#StageDir}\ximod-translator.exe"; DestDir: "{app}"; Flags: ignoreversion
; La ligne de commande (D-06) : exécutable console, même dossier.
Source: "{#StageDir}\ximod-cli.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\docs\*"; DestDir: "{app}\docs"; Flags: ignoreversion recursesubdirs createallsubdirs skipifsourcedoesntexist
Source: "{#StageDir}\assets\*"; DestDir: "{app}\assets"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#StageDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "{#StageDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "{#StageDir}\CHANGELOG.md"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "{#StageDir}\Manuals\*"; DestDir: "{app}\Manuals"; Flags: ignoreversion recursesubdirs createallsubdirs skipifsourcedoesntexist

[Dirs]
; Dossiers de bases inscriptibles par l'application (mode portable) ; en mode
; utilisateur (défaut sous Program Files) elle écrit dans %APPDATA%.
Name: "{app}\assets\bases\jeux"
Name: "{app}\assets\bases\mods"

[Icons]
Name: "{group}\XIMOD Translator"; Filename: "{app}\ximod-translator.exe"
Name: "{group}\{cm:UninstallProgram,XIMOD Translator}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\XIMOD Translator"; Filename: "{app}\ximod-translator.exe"; Tasks: desktopicon

[Registry]
Root: HKA; Subkey: "Software\Classes\.ximdb"; ValueType: string; ValueName: ""; ValueData: "XIMOD.Dictionary"; Flags: uninsdeletevalue; Tasks: assocximdb
Root: HKA; Subkey: "Software\Classes\XIMOD.Dictionary"; ValueType: string; ValueName: ""; ValueData: "Dictionnaire XIMOD"; Flags: uninsdeletekey; Tasks: assocximdb
Root: HKA; Subkey: "Software\Classes\XIMOD.Dictionary\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\ximod-translator.exe,0"; Tasks: assocximdb
Root: HKA; Subkey: "Software\Classes\XIMOD.Dictionary\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\ximod-translator.exe"" ""%1"""; Tasks: assocximdb

[Run]
Filename: "{app}\ximod-translator.exe"; Description: "{cm:LaunchProgram,XIMOD Translator}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; Seulement ce que l'installateur a créé sous {app} ; jamais %APPDATA%.
Type: filesandordirs; Name: "{app}\assets\bases"
