; KAGURA DB - Inno Setup インストーラースクリプト
;
; Windows 向けの .exe インストーラー（GUIウィザード）と、Inno Setup が自動生成する
; アンインストーラー（unins000.exe、コントロールパネルの「プログラムと機能」に登録）を作成する。
;
; 事前準備（Windows上で実行）:
;   cargo build --release -p db_client
;
; ビルド方法（Inno Setup Compiler が必要: https://jrsoftware.org/isinfo.php）:
;   iscc packaging\windows\installer.iss
;   → dist\KaguraDB-Setup-<version>.exe が生成される
;
; 要検証: 本リポジトリのサンドボックス環境は Linux のため、Inno Setup Compiler での
; 実ビルド確認ができていません。Windows + Inno Setup 環境で一度ビルド・実行確認してください。

#define MyAppName "KAGURA DB"
#define MyAppVersion "2.5.0"
#define MyAppPublisher "Shaxom357"
#define MyAppURL "https://github.com/Shaxom357/pudb-project"
#define MyAppExeName "kagura-db.exe"

[Setup]
AppId={{B6C1F6A2-6E7B-4B9E-9B1D-8E6D7B7B0F10}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
DefaultDirName={autopf}\KaguraDB
DefaultGroupName=KAGURA DB
DisableProgramGroupPage=yes
OutputDir=..\..\dist
OutputBaseFilename=KaguraDB-Setup-{#MyAppVersion}
Compression=lzma
SolidCompression=yes
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=admin
UninstallDisplayIcon={app}\{#MyAppExeName}
WizardStyle=modern

[Languages]
Name: "japanese"; MessagesFile: "compiler:Languages\Japanese.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Files]
Source: "..\..\target\release\db_client.exe"; DestDir: "{app}"; DestName: "{#MyAppExeName}"; Flags: ignoreversion
Source: "run-kagura.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "register-task.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "unregister-task.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "kagura.env.example"; DestDir: "{app}"; Flags: ignoreversion

[Dirs]
Name: "{commonappdata}\KaguraDB\config"
Name: "{commonappdata}\KaguraDB\data"
Name: "{commonappdata}\KaguraDB\logs"

[Run]
; インストール完了後、環境設定ファイルの生成とタスクスケジューラへの登録を行う
; （systemd の enable --now に相当）。register-task.ps1 は $PSScriptRoot 基準で
; kagura.env.example を探すため {app} にコピー済みのスクリプトから実行する。
Filename: "powershell.exe"; \
    Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\register-task.ps1"""; \
    WorkingDir: "{app}"; StatusMsg: "サービスを登録・起動しています..."; Flags: runhidden waituntilterminated

[UninstallRun]
; アンインストール時にタスクスケジューラ登録を解除する（データ/設定/ログは保持したまま）
Filename: "powershell.exe"; \
    Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\unregister-task.ps1"""; \
    WorkingDir: "{app}"; Flags: runhidden waituntilterminated

[UninstallDelete]
Type: files; Name: "{app}\*.ps1"

[Code]
procedure InitializeWizard;
begin
  WizardForm.WelcomeLabel2.Caption :=
    'KAGURA DB (Rust製データベースエンジン) をインストールします。' + #13#10 + #13#10 +
    'インストール後、タスクスケジューラに登録され PC 起動時に自動的に開始されます。' + #13#10 +
    '設定/データ/ログは C:\ProgramData\KaguraDB 以下に保存され、アンインストール時にも' + #13#10 +
    '既定では削除されません（手動削除、または packaging\windows\uninstall.ps1 -Purge を利用してください）。';
end;
