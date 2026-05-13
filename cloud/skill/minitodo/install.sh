#!/usr/bin/env bash
# Install the minitodo skill.
#
# 默认安装到 ~/.claude/skills/minitodo/（Claude Code）。
# --target openclaw 装到 ~/.openclaw/workspace/skills/minitodo/。
# --target both 同时安装两份（用同一份 config.toml 时需自行 symlink）。
#
# Linux/macOS. For Windows run install.ps1 instead.

set -euo pipefail

SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET="claude"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --target)
            TARGET="${2:-claude}"
            shift 2
            ;;
        --target=*)
            TARGET="${1#--target=}"
            shift
            ;;
        -h|--help)
            sed -n '2,8p' "$0"
            exit 0
            ;;
        *)
            echo "未知参数: $1" >&2
            exit 2
            ;;
    esac
done

case "${TARGET}" in
    claude)  DESTS=("${HOME}/.claude/skills/minitodo") ;;
    openclaw) DESTS=("${HOME}/.openclaw/workspace/skills/minitodo") ;;
    both)    DESTS=("${HOME}/.claude/skills/minitodo" "${HOME}/.openclaw/workspace/skills/minitodo") ;;
    *)
        echo "无效 --target: ${TARGET}（可选 claude / openclaw / both）" >&2
        exit 2
        ;;
esac

for DEST_DIR in "${DESTS[@]}"; do
    echo ">> installing minitodo skill into ${DEST_DIR}"
    mkdir -p "${DEST_DIR}"

    cp -f "${SRC_DIR}/SKILL.md"            "${DEST_DIR}/SKILL.md"
    cp -f "${SRC_DIR}/minitodo.py"         "${DEST_DIR}/minitodo.py"
    cp -f "${SRC_DIR}/config.example.toml" "${DEST_DIR}/config.example.toml"

    chmod +x "${DEST_DIR}/minitodo.py" 2>/dev/null || true

    if [[ ! -f "${DEST_DIR}/config.toml" ]]; then
        cp "${SRC_DIR}/config.example.toml" "${DEST_DIR}/config.toml"
        echo "!! ${DEST_DIR}/config.toml created from example. Please edit it to fill in"
        echo "   'endpoint' (e.g. https://minitodo.example.com) and 'api_key'."
    else
        echo ">> ${DEST_DIR}/config.toml already exists, kept untouched."
    fi
done

# Sanity check: Python + requests
if command -v python3 >/dev/null 2>&1; then
    PY=python3
elif command -v python >/dev/null 2>&1; then
    PY=python
else
    echo "!! WARNING: Python 3 not found in PATH. Install Python 3.10+ to use the skill."
    exit 0
fi

if ! "${PY}" -c 'import requests' >/dev/null 2>&1; then
    echo "!! 'requests' not installed for ${PY}. Run:"
    echo "     ${PY} -m pip install requests"
fi

# tomli only needed on <3.11
if ! "${PY}" -c 'import sys; sys.exit(0 if sys.version_info >= (3,11) else 1)' >/dev/null 2>&1; then
    if ! "${PY}" -c 'import tomli' >/dev/null 2>&1; then
        echo "!! Python <3.11 detected; install tomli:"
        echo "     ${PY} -m pip install tomli"
    fi
fi

echo ">> done. Test with:"
echo "     ${PY} ${DESTS[0]}/minitodo.py health --json"
