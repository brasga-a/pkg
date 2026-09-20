#!/bin/sh
# pkg installer - Universal rootless package manager for Linux
# https://pkg.atlantic.sh
#
# Usage:
#   curl -fsSL https://pkg.atlantic.sh/install | sh
#   curl -fsSL https://pkg.atlantic.sh/install | bash -s -- --version 0.1.0-beta.1
#
# Environment variables:
#   PKG_INSTALL_DIR     Destination directory (default: ~/.local/bin or /usr/local/bin for root)
#   PKG_VERSION         Specific version to install (default: latest)
#   PKG_BASE_URL        Base URL for releases (default: https://pkg.atlantic.sh/releases)
#   PKG_NO_MODIFY_PATH  Skip updating shell configuration files (~/.bashrc, ~/.zshrc, etc.)
#   PKG_QUIET           Suppress non-error output

set -eu

main() {
    # Detect terminal colors safely
    if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
        ESC=$(printf '\033')
        COLOR_RESET="${ESC}[0m"
        COLOR_BOLD="${ESC}[1m"
        COLOR_GREEN="${ESC}[32m"
        COLOR_BLUE="${ESC}[34m"
        COLOR_CYAN="${ESC}[36m"
        COLOR_YELLOW="${ESC}[33m"
        COLOR_RED="${ESC}[31m"
    else
        COLOR_RESET=""
        COLOR_BOLD=""
        COLOR_GREEN=""
        COLOR_BLUE=""
        COLOR_CYAN=""
        COLOR_YELLOW=""
        COLOR_RED=""
    fi

    log_info() {
        if [ "${PKG_QUIET:-0}" != "1" ]; then
            printf "${COLOR_BLUE}${COLOR_BOLD}==>${COLOR_RESET} %s\n" "$*"
        fi
    }

    log_success() {
        if [ "${PKG_QUIET:-0}" != "1" ]; then
            printf "${COLOR_GREEN}${COLOR_BOLD}  ✓${COLOR_RESET} %s\n" "$*"
        fi
    }

    log_warn() {
        printf "${COLOR_YELLOW}${COLOR_BOLD}Warning:${COLOR_RESET} %s\n" "$*" >&2
    }

    log_error() {
        printf "${COLOR_RED}${COLOR_BOLD}Error:${COLOR_RESET} %s\n" "$*" >&2
    }

    abort() {
        log_error "$*"
        exit 1
    }

    # Print ASCII banner
    if [ "${PKG_QUIET:-0}" != "1" ]; then
        printf "\n"
        printf "${COLOR_CYAN}${COLOR_BOLD}  ██████╗ ██╗  ██╗ ██████╗ ${COLOR_RESET}\n"
        printf "${COLOR_CYAN}${COLOR_BOLD}  ██╔══██╗██║ ██╔╝██╔════╝ ${COLOR_RESET}\n"
        printf "${COLOR_CYAN}${COLOR_BOLD}  ██████╔╝█████╔╝ ██║  ███╗${COLOR_RESET}  Universal Rootless Package Manager\n"
        printf "${COLOR_CYAN}${COLOR_BOLD}  ██╔═══╝ ██╔═██╗ ██║   ██║${COLOR_RESET}  for Linux & AI Agents\n"
        printf "${COLOR_CYAN}${COLOR_BOLD}  ██║     ██║  ██╗╚██████╔╝${COLOR_RESET}  https://pkg.atlantic.sh\n"
        printf "${COLOR_CYAN}${COLOR_BOLD}  ╚═╝     ╚═╝  ╚═╝ ╚═════╝ ${COLOR_RESET}\n\n"
    fi

    # CLI option parsing
    VERSION="${PKG_VERSION:-latest}"
    INSTALL_DIR="${PKG_INSTALL_DIR:-}"
    BASE_URL="${PKG_BASE_URL:-https://pkg.atlantic.sh/releases}"
    GITHUB_REPO="brasga-a/pkg"
    NO_MODIFY_PATH="${PKG_NO_MODIFY_PATH:-0}"

    while [ $# -gt 0 ]; do
        case "$1" in
            --version|-v)
                VERSION="$2"
                shift 2
                ;;
            --dir|-d)
                INSTALL_DIR="$2"
                shift 2
                ;;
            --no-modify-path)
                NO_MODIFY_PATH="1"
                shift
                ;;
            --quiet|-q)
                PKG_QUIET="1"
                shift
                ;;
            --help|-h)
                cat <<'EOF'
pkg installer

USAGE:
    curl -fsSL https://pkg.atlantic.sh/install | sh
    curl -fsSL https://pkg.atlantic.sh/install | bash -s -- [OPTIONS]

OPTIONS:
    -v, --version <VERSION>   Version to install (default: latest)
    -d, --dir <DIR>           Installation directory (default: ~/.local/bin)
    --no-modify-path          Do not edit shell rc files to add pkg to PATH
    -q, --quiet               Suppress non-essential output
    -h, --help                Show this help message

ENVIRONMENT VARIABLES:
    PKG_INSTALL_DIR           Target binary directory
    PKG_VERSION               Target version to install
    PKG_BASE_URL              Release mirror base URL
    PKG_NO_MODIFY_PATH        Skip modifying ~/.bashrc, ~/.zshrc, etc.
    PKG_QUIET                 Silent mode (errors only)

UNINSTALL:
    curl -fsSL https://pkg.atlantic.sh/uninstall | sh
EOF
                exit 0
                ;;
            *)
                log_warn "Unknown flag: $1"
                shift
                ;;
        esac
    done

    # 1. Operating System Validation
    OS="$(uname -s)"
    if [ "$OS" != "Linux" ]; then
        abort "pkg is exclusively designed for Linux environments. Detected OS: $OS"
    fi

    # 2. Architecture Detection
    ARCH_RAW="$(uname -m)"
    case "$ARCH_RAW" in
        x86_64|amd64)
            TARGET_ARCH="x86_64"
            TARGET_TRIPLE="x86_64-unknown-linux-gnu"
            ;;
        aarch64|arm64|armv8*)
            TARGET_ARCH="aarch64"
            TARGET_TRIPLE="aarch64-unknown-linux-gnu"
            ;;
        riscv64)
            TARGET_ARCH="riscv64"
            TARGET_TRIPLE="riscv64gc-unknown-linux-gnu"
            ;;
        *)
            abort "Unsupported architecture: $ARCH_RAW. Supported architectures: x86_64, aarch64, riscv64."
            ;;
    esac

    log_info "Detected platform: Linux (${COLOR_CYAN}${TARGET_ARCH}${COLOR_RESET})"

    # 3. HTTP Client Setup
    DOWNLOADER=""
    if command -v curl >/dev/null 2>&1; then
        DOWNLOADER="curl"
    elif command -v wget >/dev/null 2>&1; then
        DOWNLOADER="wget"
    else
        abort "Neither 'curl' nor 'wget' was found in PATH. Please install one to proceed."
    fi

    download_file() {
        _url="$1"
        _dest="$2"
        if [ "$DOWNLOADER" = "curl" ]; then
            curl -fsSL "$_url" -o "$_dest"
        else
            wget -qO "$_dest" "$_url"
        fi
    }

    url_exists() {
        _url="$1"
        if [ "$DOWNLOADER" = "curl" ]; then
            curl -fsIL "$_url" >/dev/null 2>&1
        else
            wget --spider -q "$_url" >/dev/null 2>&1
        fi
    }

    # 4. Resolve Target Version
    if [ "$VERSION" = "latest" ]; then
        log_info "Resolving latest release version..."
        LATEST_VERSION=""

        # Check latest.txt on pkg.atlantic.sh
        if [ -n "$BASE_URL" ] && url_exists "${BASE_URL}/latest.txt"; then
            LATEST_VERSION="$(download_file "${BASE_URL}/latest.txt" - 2>/dev/null | tr -d ' \r\n' || true)"
        fi

        # Fallback: GitHub Releases API
        if [ -z "$LATEST_VERSION" ]; then
            if [ "$DOWNLOADER" = "curl" ]; then
                LATEST_VERSION="$(curl -fsSL "https://api.github.com/repos/${GITHUB_REPO}/releases/latest" 2>/dev/null | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/' || true)"
            else
                LATEST_VERSION="$(wget -qO- "https://api.github.com/repos/${GITHUB_REPO}/releases/latest" 2>/dev/null | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/' || true)"
            fi
        fi

        if [ -n "$LATEST_VERSION" ]; then
            VERSION="$LATEST_VERSION"
        fi
    fi

    VERSION_TAG="$VERSION"
    VERSION_RAW="${VERSION#v}"

    log_info "Target release: ${COLOR_CYAN}${VERSION_TAG}${COLOR_RESET}"

    # 5. Resolve Destination Directory
    if [ -z "$INSTALL_DIR" ]; then
        if [ "$(id -u)" = "0" ]; then
            INSTALL_DIR="/usr/local/bin"
        else
            INSTALL_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
        fi
    fi

    # Set up safe temporary directory
    TMP_DIR="$(mktemp -d 2>/dev/null || mktemp -d -t 'pkg-install')"
    cleanup() {
        rm -rf "$TMP_DIR"
    }
    trap cleanup EXIT INT TERM

    # 6. Candidate Artifact Search
    ASSET_NAMES="
        pkg-${VERSION_TAG}-${TARGET_TRIPLE}.tar.gz
        pkg-${VERSION_RAW}-${TARGET_TRIPLE}.tar.gz
        pkg-${TARGET_TRIPLE}.tar.gz
        pkg-linux-${TARGET_ARCH}.tar.gz
        pkg-${TARGET_TRIPLE}
        pkg-linux-${TARGET_ARCH}
        pkg
    "

    FOUND_URL=""
    FOUND_ASSET=""

    for ASSET in $ASSET_NAMES; do
        ASSET="$(echo "$ASSET" | tr -d ' ')"
        [ -z "$ASSET" ] && continue

        # 1. BASE_URL/<version>/<asset>
        URL="${BASE_URL}/${VERSION_TAG}/${ASSET}"
        if url_exists "$URL"; then
            FOUND_URL="$URL"
            FOUND_ASSET="$ASSET"
            break
        fi

        # 2. BASE_URL/<asset>
        URL="${BASE_URL}/${ASSET}"
        if url_exists "$URL"; then
            FOUND_URL="$URL"
            FOUND_ASSET="$ASSET"
            break
        fi

        # 3. GitHub release download URL
        GH_URL="https://github.com/${GITHUB_REPO}/releases/download/${VERSION_TAG}/${ASSET}"
        if url_exists "$GH_URL"; then
            FOUND_URL="$GH_URL"
            FOUND_ASSET="$ASSET"
            break
        fi
    done

    # Default fallback target if head check is disallowed or server uses direct redirects
    if [ -z "$FOUND_URL" ]; then
        FOUND_ASSET="pkg-linux-${TARGET_ARCH}.tar.gz"
        FOUND_URL="${BASE_URL}/${VERSION_TAG}/${FOUND_ASSET}"
    fi

    log_info "Downloading pkg from: ${COLOR_CYAN}${FOUND_URL}${COLOR_RESET}..."
    DOWNLOAD_DEST="${TMP_DIR}/${FOUND_ASSET}"
    if ! download_file "$FOUND_URL" "$DOWNLOAD_DEST"; then
        # Try generic unversioned link
        FALLBACK_URL="${BASE_URL}/pkg-linux-${TARGET_ARCH}.tar.gz"
        log_warn "Versioned download failed, trying generic endpoint: ${FALLBACK_URL}..."
        if ! download_file "$FALLBACK_URL" "$DOWNLOAD_DEST"; then
            abort "Unable to download pkg from ${FOUND_URL} or ${FALLBACK_URL}."
        fi
    fi
    log_success "Download finished."

    # 7. Checksum Verification
    CHECKSUM_URL="${FOUND_URL}.sha256"
    if url_exists "$CHECKSUM_URL"; then
        log_info "Validating cryptographic SHA256 checksum..."
        CHECKSUM_FILE="${TMP_DIR}/checksum.sha256"
        if download_file "$CHECKSUM_URL" "$CHECKSUM_FILE"; then
            EXPECTED_HASH="$(awk '{print $1}' "$CHECKSUM_FILE" | head -n1)"
            
            ACTUAL_HASH=""
            if command -v sha256sum >/dev/null 2>&1; then
                ACTUAL_HASH="$(sha256sum "$DOWNLOAD_DEST" | awk '{print $1}')"
            elif command -v shasum >/dev/null 2>&1; then
                ACTUAL_HASH="$(shasum -a 256 "$DOWNLOAD_DEST" | awk '{print $1}')"
            elif command -v openssl >/dev/null 2>&1; then
                ACTUAL_HASH="$(openssl dgst -sha256 "$DOWNLOAD_DEST" | awk '{print $NF}')"
            fi

            if [ -n "$ACTUAL_HASH" ] && [ -n "$EXPECTED_HASH" ]; then
                if [ "$ACTUAL_HASH" = "$EXPECTED_HASH" ]; then
                    log_success "Checksum verified: ${COLOR_GREEN}${EXPECTED_HASH}${COLOR_RESET}"
                else
                    abort "Checksum mismatch! Expected: ${EXPECTED_HASH}, Computed: ${ACTUAL_HASH}"
                fi
            fi
        fi
    fi

    # 8. Extraction
    EXTRACTED_BINARY=""
    case "$FOUND_ASSET" in
        *.tar.gz|*.tgz)
            tar -xzf "$DOWNLOAD_DEST" -C "$TMP_DIR"
            if [ -f "${TMP_DIR}/pkg" ]; then
                EXTRACTED_BINARY="${TMP_DIR}/pkg"
            else
                EXTRACTED_BINARY="$(find "$TMP_DIR" -type f -name "pkg" | head -n1 || true)"
            fi
            ;;
        *)
            EXTRACTED_BINARY="$DOWNLOAD_DEST"
            ;;
    esac

    if [ -z "$EXTRACTED_BINARY" ] || [ ! -f "$EXTRACTED_BINARY" ]; then
        abort "Could not find 'pkg' executable inside downloaded artifact."
    fi

    # 9. Installation to Destination
    mkdir -p "$INSTALL_DIR"
    TARGET_PATH="${INSTALL_DIR}/pkg"
    
    TMP_TARGET="${TARGET_PATH}.tmp.$$"
    cp "$EXTRACTED_BINARY" "$TMP_TARGET"
    chmod 755 "$TMP_TARGET"
    mv -f "$TMP_TARGET" "$TARGET_PATH"

    log_success "Installed binary: ${COLOR_BOLD}${TARGET_PATH}${COLOR_RESET}"

    # 10. Verification
    INSTALLED_VER=""
    if [ -x "$TARGET_PATH" ]; then
        INSTALLED_VER="$("$TARGET_PATH" --version 2>/dev/null || true)"
    fi

    if [ -n "$INSTALLED_VER" ]; then
        log_success "Verified executable: ${COLOR_GREEN}${COLOR_BOLD}${INSTALLED_VER}${COLOR_RESET}"
    else
        log_warn "Executable installed, but '${TARGET_PATH} --version' failed."
    fi

    # 11. Environment and PATH Configuration
    PROFILE_BIN_DIR="${HOME}/.local/share/pkg/profiles/default/bin"
    PATH_NEEDS_INSTALL_DIR=0
    PATH_NEEDS_PROFILE_DIR=0

    case ":${PATH}:" in
        *":${INSTALL_DIR}:"*) ;;
        *) PATH_NEEDS_INSTALL_DIR=1 ;;
    esac

    case ":${PATH}:" in
        *":${PROFILE_BIN_DIR}:"*) ;;
        *) PATH_NEEDS_PROFILE_DIR=1 ;;
    esac

    update_shell_rc() {
        _rc_file="$1"
        _line="$2"
        if [ -f "$_rc_file" ]; then
            if ! grep -Fq "pkg-managed paths" "$_rc_file" 2>/dev/null; then
                printf "\n# pkg-managed paths (CLI binary and active profile packages)\n%s\n" "$_line" >> "$_rc_file"
                log_success "Configured PATH in ${COLOR_CYAN}${_rc_file}${COLOR_RESET}"
                return 0
            fi
        fi
        return 1
    }

    if [ "$PATH_NEEDS_INSTALL_DIR" = "1" ] || [ "$PATH_NEEDS_PROFILE_DIR" = "1" ]; then
        PATH_EXPORT="export PATH=\"${INSTALL_DIR}:${PROFILE_BIN_DIR}:\$PATH\""

        if [ "$NO_MODIFY_PATH" = "0" ]; then
            log_info "Checking shell environment for PATH integration..."
            UPDATED=0
            
            SHELL_NAME="$(basename "${SHELL:-bash}")"
            case "$SHELL_NAME" in
                zsh)
                    update_shell_rc "${HOME}/.zshrc" "$PATH_EXPORT" && UPDATED=1
                    ;;
                bash)
                    if [ -f "${HOME}/.bashrc" ]; then
                        update_shell_rc "${HOME}/.bashrc" "$PATH_EXPORT" && UPDATED=1
                    elif [ -f "${HOME}/.bash_profile" ]; then
                        update_shell_rc "${HOME}/.bash_profile" "$PATH_EXPORT" && UPDATED=1
                    elif [ -f "${HOME}/.profile" ]; then
                        update_shell_rc "${HOME}/.profile" "$PATH_EXPORT" && UPDATED=1
                    fi
                    ;;
                fish)
                    FISH_CONF="${HOME}/.config/fish/config.fish"
                    if [ -d "${HOME}/.config/fish" ]; then
                        FISH_LINE="fish_add_path ${INSTALL_DIR} ${PROFILE_BIN_DIR}"
                        if [ -f "$FISH_CONF" ] && ! grep -Fq "pkg-managed" "$FISH_CONF" 2>/dev/null; then
                            printf "\n# pkg-managed paths\n%s\n" "$FISH_LINE" >> "$FISH_CONF"
                            log_success "Configured PATH in ${COLOR_CYAN}${FISH_CONF}${COLOR_RESET}"
                            UPDATED=1
                        fi
                    fi
                    ;;
                *)
                    if [ -f "${HOME}/.profile" ]; then
                        update_shell_rc "${HOME}/.profile" "$PATH_EXPORT" && UPDATED=1
                    fi
                    ;;
            esac
        fi

        printf "\n${COLOR_YELLOW}${COLOR_BOLD}Notice:${COLOR_RESET} To use 'pkg' immediately in your current terminal, run:\n"
        printf "  ${COLOR_CYAN}%s${COLOR_RESET}\n\n" "$PATH_EXPORT"
    fi

    # 12. Summary and Welcome
    printf "${COLOR_GREEN}${COLOR_BOLD}pkg has been successfully installed!${COLOR_RESET}\n\n"
    printf "Quick start:\n"
    printf "  ${COLOR_CYAN}pkg sync${COLOR_RESET}                     # Synchronize repository indexes\n"
    printf "  ${COLOR_CYAN}pkg search <package>${COLOR_RESET}         # Search for packages across distributions\n"
    printf "  ${COLOR_CYAN}pkg install <package>${COLOR_RESET}        # Rootless installation into isolated store\n"
    printf "  ${COLOR_CYAN}pkg mcp${COLOR_RESET}                      # Start Model Context Protocol server for AI agents\n\n"
    printf "Documentation & Guides: ${COLOR_BLUE}https://pkg.atlantic.sh${COLOR_RESET}\n\n"
}

main "$@"
