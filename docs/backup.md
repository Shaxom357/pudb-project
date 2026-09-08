# バックアップと復元

KAGURA DB を **別サーバーへの移設** や **新規／再インストール環境での災害復旧** に備えて、
`.kdb` 本体・サイドカーファイル・`auth.json` を1つの `.kbak` アーカイブにまとめる機能です。

- SQL: `BACKUP TO '...'` / `RESTORE FROM '...'`（Web UI の SQL Query ビュー・`/sql` API）
- CLI: `kdb backup` / `kdb restore` / `kdb keyinfo`
- 実行できるのは **管理者ロール `kagura`** のみ。**トランザクション中は不可**（`409`）。

> **バージョン 4.10.0 の範囲**: SQL 文と `kdb` CLI、マスターキー表示エンドポイントを提供します。
> Web 管理 UI のバックアップカードと REST エンドポイント（`POST /backup` 等）、`PUT /settings`
> からの世代設定変更は後続バージョンで追加予定です。

---

## アーカイブに入るもの

| logical 名 | 実ファイル | 内容 |
|-----------|-----------|------|
| `kdb` | `<DB_FILE>` | データ本体（`.kdb` は **暗号化されたまま** 収録） |
| `indexes` | `<DB_FILE>.indexes.json` | 二次インデックス定義 |
| `schema` | `<DB_FILE>.schema.json` | 任意スキーマ定義 |
| `memory` | `<DB_FILE>.memory.json` | 「全データオンメモリ」設定 |
| `auth` | `<KAGURA_AUTH_FILE>`（既定 `auth.json`） | ユーザー・パスワードハッシュ・権限（既定で同梱。`WITHOUT AUTH` で除外） |

さらに `manifest.json` を同梱します（形式バージョン・KAGURA バージョン・作成日時・作成者・
レコード件数・**マスターキー指紋**・各ファイルの SHA-256・`WITH KEY` 時のみ旧マスターキー）。

存在しないファイルは自動的に省かれます。`.kbak` は独自コンテナ形式（非圧縮。`.kdb` は
暗号化済みで圧縮の余地がほとんどないため）で、全ファイルを SHA-256 で検証します。

---

## マスターキーの扱い（最重要）

`.kdb` は `HChaCha20(マスターキー, ファイルsalt)` で暗号化されています。`salt` は `.kdb`
ヘッダーに入っているのでバックアップに含まれますが、**マスターキー本体はプログラム内にも
データ内にも存在しません**（`KAGURA_MASTER_KEY` 環境変数、パッケージ版では
`/etc/kagura-db/kagura.env`）。

したがって **`.kbak` だけでは復元できません**。次のいずれかが必要です。

1. **`BACKUP TO '...' WITH KEY`** — `manifest.json` に作成時のマスターキー（hex64）を同梱。
   `.kbak` 単体で復旧できますが、アーカイブが平文データと同等の機密になります。**取り扱い注意**。
2. **マスターキーを別途保管** — `kdb keyinfo --reveal`（管理者のみ）や `kagura.env` から
   控えておき、`RESTORE FROM '...' OLD KEY '<hex64>'` で渡す。

`.kbak` とマスターキーは **別々の場所** に保管してください。両方を同じ場所に置くと暗号化の
意味が失われます。

### リキー（復元先のキーが異なる場合）

新規インストールした KAGURA DB は **新しいランダムなマスターキー** を生成します。この環境へ
別サーバーのバックアップを復元すると、`RESTORE` は作成時のキー（`WITH KEY` の同梱値、
または `OLD KEY` 指定値）で `.kdb` を復号し、**復元先の現在のキーで再暗号化（リキー）** して
から配置します。以後はその環境のキーで運用します。

作成時と復元先のキー指紋が一致する場合（＝同じキー）は、リキーせずそのまま配置します。

---

## 災害復旧の手順（サーバー A 全損 → サーバー B へ）

1. **事前に**（A が生きているうちに）バックアップを取得しておく:

   ```sql
   BACKUP TO '/var/lib/kagura-db/backups/'
   ```

   `WITH KEY` を付けない場合は、あわせて `kdb keyinfo --reveal` でマスターキーを控え、
   `.kbak` とは別の安全な場所に保管する。

2. サーバー B に KAGURA DB を通常インストールする（B 用の新しいマスターキーが生成される）。

3. `.kbak` を B の任意の場所へ配置し、復元する:

   - `.kbak` が `WITH KEY` の場合:

     ```bash
     kdb login -u kagura -a http://localhost:3000
     kdb restore /path/to/kagura-backup-XXXX.kbak
     ```

   - `WITH KEY` でない場合は、A のマスターキーを渡す:

     ```bash
     kdb restore /path/to/kagura-backup-XXXX.kbak --old-key <hex64>
     ```

   - サーバーを起動する前に適用したい場合は直接モード:

     ```bash
     kdb restore /path/to/kagura-backup-XXXX.kbak --direct \
       --env-file /etc/kagura-db/kagura.env [--old-key <hex64>]
     ```

4. **サーバー B を再起動する**（サーバー経由の `RESTORE` は再起動するまで反映されない）。

   - `RESTORE` は現ファイルを `pre-restore-<timestamp>/` に退避してから置き換える。
   - 適用後、稼働中プロセスは書き込み系リクエストを `409` で拒否する（メモリ上の
     復元前データでディスクを上書きしないため）。SELECT は可能。

メジャーバージョンが異なる `.kbak`（例: `3.x` を `4.x` へ）は復元を拒否します。

---

## SQL 構文

```sql
BACKUP TO '<保存先>' [WITH KEY] [WITHOUT AUTH]
RESTORE FROM '<.kbak のパス>' [OLD KEY '<hex64>']
```

- `<保存先>` がディレクトリ（既存 or 末尾 `/`、または拡張子なし）なら
  `kagura-backup-<YYYYMMDD-HHMMSS>.kbak` を自動命名。ファイル名ならそのまま使う。
- `WITH KEY`: マスターキーを同梱（`.kdb` モードのみ意味を持つ）。
- `WITHOUT AUTH`: `auth.json` を同梱しない（既定は同梱）。

---

## CLI

```bash
# バックアップ（稼働中サーバー経由）
kdb backup --out '/var/lib/kagura-db/backups/'
kdb backup --out '/tmp/mybackup.kbak' --with-key
kdb backup --out '/backups/' --no-auth

# バックアップ（サーバー停止中・直接ファイル操作）
kdb backup --direct --env-file /etc/kagura-db/kagura.env --out '/backups/'

# 復元（サーバー経由。反映には再起動が必要）
kdb restore /backups/kagura-backup-20260908-120000.kbak
kdb restore /backups/kagura-backup-20260908-120000.kbak --old-key <hex64>

# 復元（サーバー停止中・直接）
kdb restore /backups/....kbak --direct --env-file /etc/kagura-db/kagura.env

# マスターキーの指紋（と --reveal でキー本体。管理者ロールのみ）
kdb keyinfo
kdb keyinfo --reveal
```

`--direct` はサーバー停止中に使ってください。稼働中に実行すると、サーバーが後で書き戻す
内容と食い違う可能性があります。

---

## 世代管理

サーバーローカルの保存先（`BACKUP TO` にディレクトリを渡したときの出力先）は、
バックアップ成功のたびに剪定されます。

- **保持世代数の上限**（既定 7）を超える古いものを削除
- **保持日数**（既定 30 日）より古いものを削除

いずれも `0` で「無制限」。設定は `<DB_FILE>.backup.json` に保存されます
（値の変更 UI／API は後続バージョンで追加）。ブラウザからダウンロードしたファイルや
`--direct` の出力は剪定対象外です。

---

## マスターキーの表示 API

```
GET /settings/master-key        （管理者ロール kagura のみ）
  -> { "master_key": "<hex64>", "fingerprint": "<指紋>" }
```

呼び出すと監査ログに「マスターキーを表示しました (by 'kagura')」と記録されます
（**キー値はログに残りません**）。

---

## 定期バックアップ

本バージョンでは専用機能は用意していません。cron / systemd timer / Windows タスク
スケジューラから `kdb backup --out '<保存先ディレクトリ>/'` を定期実行してください
（世代管理は上記のとおり自動で行われます）。
