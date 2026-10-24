#!/usr/bin/env bash
# Online entry point: keep the implementation and recovery logic in Python.
# A function definition prevents a truncated download from running its body.
patchhub_install() (
    set -eu
    installer=$(curl -fsSL https://raw.githubusercontent.com/Mabinogi-Cichol/patchhub/main/scripts/install-decky.py)
    if [ -z "$installer" ]; then
        printf '%s\n' '安装器下载内容为空，已停止。' >&2
        exit 1
    fi
    python3 -c "$installer" "$@"
)
patchhub_install "$@"
