#!/usr/bin/env bash
# KAGURA DB - Linux アンインストールスクリプト
#
# 使い方:
#   sudo ./packaging/scripts/uninstall.sh          # サービス・バイナリのみ削除（データ/設定/ユーザーは残す）
#   sudo ./packaging/scripts/uninstall.sh --purge   # データ/設定/ログ/専用ユーザーも含めて完全削除

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

PURGE=0
for arg in "$@"; do
    case "$arg" in
        --purge) PURGE=1 ;;
        *)
            echo "[ERROR] 不明な引数です: $arg" >&2
            exit 1
            ;;
    esac
done

log() { echo "[uninstall] $*"; }

if [[ "$(id -u)" -ne 0 ]]; then
    echo "[ERROR] root権限で実行してください（例: sudo $0）" >&2
    exit 1
fi

log "サービスを停止・無効化します: $SERVICE_NAME"
systemctl disable --now "$SERVICE_NAME" 2>/dev/null || true

if [[ -f "$UNIT_DIR/$SERVICE_NAME" ]]; then
    log "systemdユニットを削除します: $UNIT_DIR/$SERVICE_NAME"
    rm -f "$UNIT_DIR/$SERVICE_NAME"
    systemctl daemon-reload
fi

if [[ -f "$INSTALL_BIN_DIR/$BIN_NAME" ]]; then
    log "バイナリを削除します: $INSTALL_BIN_DIR/$BIN_NAME"
    rm -f "$INSTALL_BIN_DIR/$BIN_NAME"
fi

if [[ "$PURGE" -eq 0 ]]; then
    cat <<EOF
[uninstall] サービス本体を削除しました。
以下は保持されています（完全に削除する場合は --purge を付けて再実行してください）:
  設定ファイル  : $CONF_DIR
  データファイル: $DATA_DIR
  ログファイル  : $LOG_DIR
  システムユーザー: $SERVICE_USER
EOF
    exit 0
fi

log "--purge が指定されたため、設定・データ・ログ・専用ユーザーも削除します"
rm -rf "$CONF_DIR" "$DATA_DIR" "$LOG_DIR"

if id -u "$SERVICE_USER" >/dev/null 2>&1; then
    log "システムユーザーを削除します: $SERVICE_USER"
    userdel "$SERVICE_USER" 2>/dev/null || true
fi

if getent group "$SERVICE_GROUP" >/dev/null 2>&1; then
    log "システムグループを削除します: $SERVICE_GROUP"
    groupdel "$SERVICE_GROUP" 2>/dev/null || true
fi

log "完全に削除しました"
