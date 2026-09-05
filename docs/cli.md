# kdb コマンドラインクライアント

KAGURA DB インストール後に使える専用の Linux コマンド `kdb`（クレート: `kdb_cli`）。
`curl` で毎回 `Authorization: Bearer <token>` ヘッダーを組み立てなくても、一般的な DB クライアント
（`mysql` / `psql` / `redis-cli` 等）と同様の感覚でログイン・セッション管理ができる。

認証の仕組み自体は [authentication.md](authentication.md) を参照してください。

## 入手方法

[installation.md](installation.md#linux-へのインストールsystemd連携)（`install.sh` / `.deb` / `.rpm` の
いずれか）を行うと、`db_client` 本体とあわせて `/usr/bin/kdb` に自動配置され、そのまま
`kdb login ...` を実行できる（`sudo ./packaging/scripts/install.sh` の完了メッセージにコマンド例が出力される）。

ソースからビルドしただけ（`cargo build` のみ）の場合は `/usr/bin` 等の `PATH` に配置されて
いないため `kdb: command not found` になる。以下のいずれかで実行する。

```bash
# (a) target/release から直接実行する
cargo build --release -p kdb_cli
./target/release/kdb login -u kagura -p root -a localhost:3000

# (b) ~/.cargo/bin にインストールする（通常rustupの設定で$PATHに含まれる）
cargo install --path kdb_cli
kdb login -u kagura -p root -a localhost:3000
```

## サブコマンド

| コマンド | 内容 |
|---------|------|
| `kdb login -u <ユーザー名> -p <パスワード> -a <接続先>` | `POST /auth/login` でログインし、成功したらセッションを保存する |
| `kdb logout` | `POST /auth/logout` でサーバー側のトークンを失効させ、ローカルのセッションファイルを削除する |
| `kdb whoami` | 保存済みセッションのユーザー名・接続先を表示し、`GET /db/info` でトークンがまだ有効か確認する |
| `kdb shell-init bash` \| `zsh` | シェルプロンプトにログイン状態を表示する連携スクリプトを出力する（詳細は下記「シェルプロンプトへのログイン状態表示」参照） |
| `kdb prompt` | 現在のログイン状態を短いタグ文字列として出力する。`shell-init` が生成するプロンプトフックが毎回のプロンプト描画時に内部的に呼び出すためのもので、通常は直接使わない |
| `kdb in-memory-mode enable` \| `disable` | 「全データオンメモリ」の有効/無効を切り替える（`PUT /settings`）。**管理者ロール `kagura`** でログイン中のみ・`.kdb` モードのサーバーのみ |
| `kdb in-memory-size <値>` | オンメモリ容量の上限を設定する（`PUT /settings`）。数値の後ろに `MB` / `GB` を付けると絶対サイズ、`%` を付けると搭載メモリに対する割合（例: `kdb in-memory-size 5GB` / `kdb in-memory-size 20%`）。同上の権限・モード制限 |

`in-memory-mode disable`（＝容量制限モード）にすると、直近使われていない行の列データから `.kdb` へ退避してメモリ使用量を上限内に抑える（アクセス時に読み直し、再アクセスで立ち退き順が更新される）。設定はサーバー側の `<DB_FILE>.memory.json` に保存され再起動をまたいで保持される。Web UI（設定ビュー）からも同じ操作ができる。

## login オプション

| オプション | 環境変数 | 内容 |
|-----------|---------|------|
| `-u`, `--user <ユーザー名>` | `KDB_USER` | ログインユーザー名（必須） |
| `-p`, `--password <パスワード>` | `KDB_PASSWORD` | パスワード。**省略した場合は非表示入力（エコーバックなし）で対話的にプロンプトする**（シェル履歴・`ps`コマンドへの平文パスワード漏えいを避けるため） |
| `-a`, `--address <接続先>` | `KDB_ADDR` | 接続先（必須）。`127.0.0.1:3000` のようにスキームを省略した場合は `http://` を補完する。`https://host` も指定可能 |
| `-k`, `--insecure` | - | TLS証明書の検証をスキップする（自己署名証明書向け。信頼できる接続先のみで使用すること） |

## 使用例

```bash
# パスワードを対話的に入力してログイン（推奨）
kdb login -u kagura -a 127.0.0.1:3000
Password: ********
# -> ログインしました: user='kagura' server='http://127.0.0.1:3000'
# -> 接続先: KAGURA DB Engine v3.3.0 (storage: kdb)

# 環境変数からも指定可能（CI等での非対話実行向け）
KDB_USER=kagura KDB_PASSWORD=your-password KDB_ADDR=127.0.0.1:3000 kdb login

# 現在のログイン状態を確認
kdb whoami

# ログアウト（ローカルセッションを削除し、サーバー側トークンも失効させる）
kdb logout
```

セッションは `~/.config/kdb/session.json`（トークンを含むためパーミッション600）に保存される。
現バージョンでは `login` / `logout` / `whoami` / `shell-init` / `prompt` / `in-memory-mode` /
`in-memory-size` を提供する。SQL実行など他のサブコマンドは今後追加予定。

## シェルプロンプトへのログイン状態表示

`kdb login` はあくまで別プロセス（子プロセス）としてサーバーへログインするだけなので、
それ自体はターミナルのシェルプロンプト（`PS1`）を書き換えられない。そこで `kdb` は
`kdb shell-init bash`（`zsh` も同様）で、シェル側にプロンプトフックを仕込むための
連携スクリプトを出力する。

シェルの設定ファイルに1行追加するだけで、以後すべての新しいターミナルで自動的に
現在のログイン状態がプロンプトへ反映される（`kube-ps1` 等と同様の、既存プロンプトへの
タグ付加方式）。

```bash
# ~/.bashrc （bashの場合）
eval "$(kdb shell-init bash)"

# ~/.zshrc （zshの場合）
eval "$(kdb shell-init zsh)"
```

設定後、ターミナルを開き直すか `source ~/.bashrc`（zshなら `source ~/.zshrc`）すると、
ログイン中は既存のプロンプトの先頭にタグが付加される。

```
$ kdb login -u kagura -p root -a localhost:3000
ログインしました: user='kagura' server='http://localhost:3000'
(kdb:kagura@localhost:3000) [ec2-user@ip-10-0-0-5 scripts]$ kdb logout
ログアウトしました: user='kagura' server='http://localhost:3000'
[ec2-user@ip-10-0-0-5 scripts]$
```

仕組みは、プロンプト文字列（`PS1`/`PROMPT`）に `$(__kdb_ps1)` というコマンド置換を
埋め込んでおき、プロンプトが描画されるたびに `kdb prompt`（＝ローカルのセッションファイルの
有無を見るだけの軽量チェック）が呼ばれてタグの表示/非表示が切り替わる、というもの。
`shell-init` の出力は既に同じフックが仕込まれている場合は再度追加しない（`.bashrc` を
何度 `source` しても、または `install.sh`/deb/rpm で再インストールしても多重化しない）。

> **注意**: このタグはローカルのセッションファイルの有無のみで判定しており、サーバーへの
> 通信は行わない（毎回のプロンプト描画でサーバーに問い合わせるのは実用的でないため）。
> そのため、トークンの期限切れ（発行から24時間）やサーバー側での失効後も、`kdb logout`を
> 実行するかセッションファイルが削除されるまではタグが表示され続ける。正確なログイン有効性は
> `kdb whoami` で確認すること。
