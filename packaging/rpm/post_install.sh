#!/bin/sh
# KAGURA DB - rpm %post スクリプトレット
# $1 == 1: 新規インストール / $1 == 2: アップグレード
set -e

SERVICE_NAME="kagura-db.service"
SERVICE_USER="kagura"
SERVICE_GROUP="kagura"
CONF_DIR="/etc/kagura-db"
DATA_DIR="/var/lib/kagura-db"
LOG_DIR="/var/log/kagura-db"
ENV_FILE="$CONF_DIR/kagura.env"

getent group "$SERVICE_GROUP" >/dev/null 2>&1 || groupadd --system "$SERVICE_GROUP"
getent passwd "$SERVICE_USER" >/dev/null 2>&1 || useradd --system \
    --gid "$SERVICE_GROUP" --home-dir "$DATA_DIR" --no-create-home \
    --shell /sbin/nologin --comment "KAGURA DB service account" "$SERVICE_USER"

install -d -o root -g "$SERVICE_GROUP" -m 750 "$CONF_DIR"
install -d -o "$SERVICE_USER" -g "$SERVICE_GROUP" -m 750 "$DATA_DIR"
install -d -o "$SERVICE_USER" -g "$SERVICE_GROUP" -m 750 "$LOG_DIR"

if [ ! -f "$ENV_FILE" ]; then
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

if [ -d /run/systemd/system ]; then
    systemctl daemon-reload || true
    systemctl enable --now "$SERVICE_NAME" || true
fi

exit 0
