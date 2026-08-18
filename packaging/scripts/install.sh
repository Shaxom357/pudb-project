#!/usr/bin/env bash
# KAGURA DB - Linux インストールスクリプト
#
# ソースからリリースビルドし、一般的な Linux パッケージと同様に
#   - 専用の非rootシステムユーザー (kagura) での実行
#   - FHS準拠のディレクトリ配置 (/usr/bin, /etc, /var/lib, /var/log)
#   - systemd ユニットの登録・有効化・起動
# を行う。再実行しても安全（冪等）。
#
# 使い方:
#   sudo ./packaging/scripts/install.sh [--no-enable]
#
#   --no-enable  systemd への登録のみ行い、enable/start は行わない

set -euo pipefail

BIN_NAME="kagura-db"
SERVICE_NAME="kagura-db.service"
SERVICE_USER="kagura"
SERVICE_GROUP="kagura"

INSTALL_BIN_DIR="/usr/bin"
CONF_DIR="/etc/kagura-db"
DATA_DIR="/var/lib/kagura-db"
LOG_DIR="/var/log/kagura-db"
UNIT_DIR="/etc/systemd/system"

ENABLE_SERVICE=1
for arg in "$@"; do
    case "$arg" in
        --no-enable) ENABLE_SERVICE=0 ;;
        *)
            echo "[ERROR] 不明な引数です: $arg" >&2
            exit 1
            ;;
    esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
PACKAGING_DIR="$REPO_ROOT/packaging"

log() { echo "[install] $*"; }

if [[ "$(id -u)" -ne 0 ]]; then
    echo "[ERROR] root権限で実行してください（例: sudo $0）" >&2
    exit 1
fi

# rustup 経由の cargo は既定でツールチェーン設定を $HOME/.rustup 配下に置くため、
# 単に `sudo ./install.sh` すると root の $HOME が見えて「ツールチェーン未設定」等で
# 失敗しやすい。sudo実行時は元ユーザー ($SUDO_USER) の環境でビルドする。
BUILD_USER="${SUDO_USER:-$(id -un)}"

if ! sudo -u "$BUILD_USER" -H bash -lc 'command -v cargo' >/dev/null 2>&1; then
    echo "[ERROR] cargo が見つかりません（ユーザー: $BUILD_USER）。Rust (https://rustup.rs) をインストールしてください。" >&2
    exit 1
fi

# ---- 1. リリースビルド ----
log "リリースビルド中... (cargo build --release -p db_client, 実行ユーザー: $BUILD_USER)"
sudo -u "$BUILD_USER" -H bash -lc "cd '$REPO_ROOT' && cargo build --release -p db_client"

BUILT_BIN="$REPO_ROOT/target/release/db_client"
if [[ ! -x "$BUILT_BIN" ]]; then
    echo "[ERROR] ビルド成果物が見つかりません: $BUILT_BIN" >&2
    exit 1
fi

# ---- 2. 専用システムユーザー/グループ作成 ----
if ! getent group "$SERVICE_GROUP" >/dev/null 2>&1; then
    log "システムグループを作成します: $SERVICE_GROUP"
    groupadd --system "$SERVICE_GROUP"
fi

if ! id -u "$SERVICE_USER" >/dev/null 2>&1; then
    log "システムユーザーを作成します: $SERVICE_USER"
    useradd --system \
        --gid "$SERVICE_GROUP" \
        --home-dir "$DATA_DIR" \
        --no-create-home \
        --shell /usr/sbin/nologin \
        --comment "KAGURA DB service account" \
        "$SERVICE_USER"
fi

# ---- 3. ディレクトリ作成 ----
log "ディレクトリを作成します: $CONF_DIR, $DATA_DIR, $LOG_DIR"
install -d -o root -g "$SERVICE_GROUP" -m 750 "$CONF_DIR"
install -d -o "$SERVICE_USER" -g "$SERVICE_GROUP" -m 750 "$DATA_DIR"
install -d -o "$SERVICE_USER" -g "$SERVICE_GROUP" -m 750 "$LOG_DIR"

# ---- 4. バイナリ配置 ----
log "バイナリを配置します: $INSTALL_BIN_DIR/$BIN_NAME"
install -o root -g root -m 755 "$BUILT_BIN" "$INSTALL_BIN_DIR/$BIN_NAME"

# ---- 5. 環境設定ファイル (初回のみ生成、マスターキーをランダム生成) ----
ENV_FILE="$CONF_DIR/kagura.env"
if [[ -f "$ENV_FILE" ]]; then
    log "既存の設定ファイルを保持します: $ENV_FILE"
else
    log "設定ファイルを生成します: $ENV_FILE (マスターキーをランダム生成)"
    if command -v openssl >/dev/null 2>&1; then
        MASTER_KEY="$(openssl rand -hex 32)"
    else
        MASTER_KEY="$(head -c32 /dev/urandom | od -An -tx1 | tr -d ' \n')"
    fi

    install -o root -g "$SERVICE_GROUP" -m 640 /dev/null "$ENV_FILE"
    {
        echo "# KAGURA DB 環境設定ファイル（自動生成: $(date -Iseconds)）"
        echo "DB_FILE=$DATA_DIR/db_data.kdb"
        echo "DB_ADDR=0.0.0.0:3000"
        echo "KAGURA_MASTER_KEY=$MASTER_KEY"
        echo "KAGURA_LOG_FILE=$LOG_DIR/kagura.log"
    } > "$ENV_FILE"
    chmod 640 "$ENV_FILE"
    chown root:"$SERVICE_GROUP" "$ENV_FILE"
fi

# 参考用の雛形ファイルも配置（値は空のまま・上書きしない）
install -o root -g "$SERVICE_GROUP" -m 640 "$PACKAGING_DIR/env/kagura.env.example" "$CONF_DIR/kagura.env.example"

# ---- 6. systemd ユニット登録 ----
log "systemdユニットを配置します: $UNIT_DIR/$SERVICE_NAME"
install -o root -g root -m 644 "$PACKAGING_DIR/systemd/$SERVICE_NAME" "$UNIT_DIR/$SERVICE_NAME"
systemctl daemon-reload

if [[ "$ENABLE_SERVICE" -eq 1 ]]; then
    log "サービスを有効化・起動します: $SERVICE_NAME"
    systemctl enable --now "$SERVICE_NAME"
else
    log "--no-enable が指定されたため、有効化・起動はスキップしました"
fi

cat <<EOF

インストールが完了しました。

  起動状態確認 : systemctl status $SERVICE_NAME
  起動         : sudo systemctl start $SERVICE_NAME
  停止         : sudo systemctl stop $SERVICE_NAME
  再起動       : sudo systemctl restart $SERVICE_NAME
  自動起動設定 : sudo systemctl enable $SERVICE_NAME
  ログ (journal): journalctl -u $SERVICE_NAME -f
  アプリログ    : $LOG_DIR/kagura.log
  設定ファイル  : $ENV_FILE
  データファイル: $DATA_DIR/db_data.kdb

EOF
