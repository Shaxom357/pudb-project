#!/bin/sh
# KAGURA DB - rpm %preun スクリプトレット
# $1 == 0: 完全アンインストール / $1 >= 1: アップグレード（何もしない）
set -e

SERVICE_NAME="kagura-db.service"

if [ "$1" = "0" ] && [ -d /run/systemd/system ]; then
    systemctl stop "$SERVICE_NAME" || true
    systemctl disable "$SERVICE_NAME" || true
fi

exit 0
