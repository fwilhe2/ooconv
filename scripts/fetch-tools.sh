#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later

# Fetch and build the validators the oracle tests use, into .tools/ (git-ignored):
#
#   .tools/jing/jing.jar                 RELAX NG validator for ODF (needs java)
#   .tools/ooxml-validate/ooxml-validate Open XML SDK validator (needs the .NET SDK)
#
# Pinned by version and checksum: a validator's opinion is part of the test. LibreOffice,
# gridcraft-cli and wordcraft-cli are installed system-wide and found on PATH (doc/oracles.md).
set -euo pipefail
cd "$(dirname "$0")/.."

JING_VERSION=20220510
JING_SHA256=c1887f5ea6bd90067baafb4cd0d215294781f6fd121b7905c89829d43ea9e280

mkdir -p .tools
if [ ! -f .tools/jing/jing.jar ]; then
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    curl -fsSL -o "$tmp/jing.zip" \
        "https://github.com/relaxng/jing-trang/releases/download/V${JING_VERSION}/jing-${JING_VERSION}.zip"
    echo "${JING_SHA256}  $tmp/jing.zip" | sha256sum -c -
    unzip -q "$tmp/jing.zip" -d "$tmp"
    mkdir -p .tools/jing
    cp "$tmp/jing-${JING_VERSION}/bin/"*.jar .tools/jing/
fi
echo "jing: $(java -jar .tools/jing/jing.jar 2>&1 | head -1)"

dotnet build tools/ooxml-validate -c Release -o .tools/ooxml-validate --nologo -v quiet
echo "ooxml-validate: built"

for tool in soffice gridcraft-cli wordcraft-cli; do
    if command -v "$tool" >/dev/null; then
        echo "$tool: $(command -v "$tool")"
    else
        echo "$tool: not on PATH — its oracle tests will skip"
    fi
done
