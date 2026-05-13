#!/usr/bin/env bash
# Install the minitodo skill into ~/.claude/skills/minitodo/.
#
# Linux/macOS. For Windows run install.ps1 instead.

set -euo pipefail

SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEST_DIR="${HOME}/.claude/skills/minitodo"

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
echo "     ${PY} ${DEST_DIR}/minitodo.py health --json"
