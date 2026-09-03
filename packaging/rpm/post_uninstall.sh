#!/bin/sh
# KAGURA DB - rpm %postun スクリプトレット
# $1 == 0: 完全アンインストール（設定/データ/専用ユーザーは意図的に残す。
#          dpkg の purge に相当する完全削除は手動で行う: README参照）
# $1 >= 1: アップグレード
set -e

if [ -d /run/systemd/system ]; then
    systemctl daemon-reload || true
fi

exit 0
