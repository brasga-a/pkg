#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PACKAGE_DIR="${SCRIPT_DIR}/package"
OUTPUT_DEB="${SCRIPT_DIR}/hello-world_1.0.0_amd64.deb"

chmod 0755 "${PACKAGE_DIR}/usr/bin/hello-world"
chmod 0755 "${PACKAGE_DIR}/DEBIAN"
chmod 0644 "${PACKAGE_DIR}/DEBIAN/control"

dpkg-deb --build --root-owner-group "${PACKAGE_DIR}" "${OUTPUT_DEB}"
echo "Pacote gerado com sucesso em: ${OUTPUT_DEB}"
