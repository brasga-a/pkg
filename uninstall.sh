#!/bin/sh
# pkg uninstaller - Universal rootless package manager for Linux
# https://pkg.atlantic.sh
#
# Usage:
#   curl -fsSL https://pkg.atlantic.sh/uninstall | sh
#   curl -fsSL https://pkg.atlantic.sh/uninstall | bash -s -- --purge --yes
#
# Environment variables:
#   PKG_INSTALL_DIR     Custom binary directory (default: auto-detect)
#   PKG_PURGE           Set to '1' to purge data, store, and configuration
#   PKG_YES             Set to '1' to skip confirmation prompts
#   PKG_NO_MODIFY_PATH  Set to '1' to skip modifying shell rc files
#   PKG_QUIET           Set to '1' to suppress non-error output

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
        printf "${COLOR_CYAN}${COLOR_BOLD}  ██████╔╝█████╔╝ ██║  ███╗${COLOR_RESET}  Uninstaller\n"
        printf "${COLOR_CYAN}${COLOR_BOLD}  ██╔═══╝ ██╔═██╗ ██║   ██║${COLOR_RESET}  https://pkg.atlantic.sh\n"
        printf "${COLOR_CYAN}${COLOR_BOLD}  ██║     ██║  ██╗╚██████╔╝${COLOR_RESET}\n"
        printf "${COLOR_CYAN}${COLOR_BOLD}  ╚═╝     ╚═╝  ╚═╝ ╚═════╝ ${COLOR_RESET}\n\n"
    fi

    # CLI option parsing
    INSTALL_DIR="${PKG_INSTALL_DIR:-}"
    PURGE="${PKG_PURGE:-0}"
    ASSUME_YES="${PKG_YES:-0}"
    NO_MODIFY_PATH="${PKG_NO_MODIFY_PATH:-0}"
    DRY_RUN=0
    PKG_QUIET="${PKG_QUIET:-0}"

    while [ $# -gt 0 ]; do
        case "$1" in
            --dir|-d)
                INSTALL_DIR="$2"
                shift 2
                ;;
            --purge|-p)
                PURGE=1
                shift
                ;;
            --yes|-y)
                ASSUME_YES=1
                shift
                ;;
            --no-modify-path)
                NO_MODIFY_PATH=1
                shift
                ;;
            --dry-run)
                DRY_RUN=1
                shift
                ;;
            --quiet|-q)
                PKG_QUIET=1
                shift
                ;;
            --help|-h)
                cat <<'EOF'
pkg uninstaller

USAGE:
    curl -fsSL https://pkg.atlantic.sh/uninstall | sh
    curl -fsSL https://pkg.atlantic.sh/uninstall | bash -s -- [OPTIONS]

OPTIONS:
    -d, --dir <DIR>       Directory containing the pkg binary to remove
    -p, --purge           Remove all user data, package store, cache, and config
    -y, --yes             Assume yes to all prompts (non-interactive mode)
    --dry-run             Show what would be removed without deleting anything
    --no-modify-path      Skip removing pkg PATH entries from shell rc files
    -q, --quiet           Suppress non-essential output
    -h, --help            Show this help message

ENVIRONMENT VARIABLES:
    PKG_INSTALL_DIR       Target binary directory
    PKG_PURGE             Set to '1' to purge package store and config
    PKG_YES               Set to '1' to skip confirmation prompts
    PKG_NO_MODIFY_PATH    Set to '1' to skip modifying shell rc files
    PKG_QUIET             Silent mode (errors only)
EOF
                exit 0
                ;;
            *)
                log_warn "Unknown flag: $1"
                shift
                ;;
        esac
    done

    prompt_confirm() {
        _msg="$1"
        _default="${2:-n}" # y or n
        if [ "$ASSUME_YES" = "1" ]; then
            return 0
        fi
        if [ -c /dev/tty ]; then
            printf "%s" "$_msg" > /dev/tty
            read -r _resp < /dev/tty || _resp=""
        else
            if [ "$_default" = "y" ]; then
                return 0
            else
                return 1
            fi
        fi
        case "$_resp" in
            [yY]|[yY][eE][sS]) return 0 ;;
            [nN]|[nN][oO]) return 1 ;;
            "")
                if [ "$_default" = "y" ]; then
                    return 0
                else
                    return 1
                fi
                ;;
            *) return 1 ;;
        esac
    }

    # 1. Locate pkg executable(s)
    FOUND_BINARIES=""
    add_binary_if_found() {
        _b="$1"
        if [ -f "$_b" ] || [ -L "$_b" ]; then
            case " $FOUND_BINARIES " in
                *" $_b "*) ;;
                *) FOUND_BINARIES="$FOUND_BINARIES $_b" ;;
            esac
        fi
    }

    if [ -n "$INSTALL_DIR" ]; then
        add_binary_if_found "${INSTALL_DIR}/pkg"
    else
        # Auto-detect binary locations
        if command -v pkg >/dev/null 2>&1; then
            CMD_PKG="$(command -v pkg || true)"
            add_binary_if_found "$CMD_PKG"
        fi
        add_binary_if_found "${XDG_BIN_HOME:-$HOME/.local/bin}/pkg"
        add_binary_if_found "/usr/local/bin/pkg"
        add_binary_if_found "/usr/bin/pkg"
    fi

    # Trim leading space
    FOUND_BINARIES="$(echo "$FOUND_BINARIES" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')"

    # 2. Identify Data, Configuration and Cache Directories
    DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/pkg"
    CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/pkg"
    CACHE_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/pkg"
    STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/pkg"

    # 3. Interactive Confirmations
    if [ "$DRY_RUN" = "0" ] && [ "$ASSUME_YES" = "0" ]; then
        if [ -z "$FOUND_BINARIES" ] && [ ! -d "$DATA_DIR" ] && [ ! -d "$CONFIG_DIR" ]; then
            log_warn "No pkg binary or data directory detected on this system."
            if ! prompt_confirm "Proceed with checking shell configuration anyway? [y/N]: " "n"; then
                exit 0
            fi
        else
            if ! prompt_confirm "Are you sure you want to uninstall pkg? [y/N]: " "n"; then
                log_info "Uninstallation cancelled by user."
                exit 0
            fi

            if [ "$PURGE" = "0" ] && { [ -d "$DATA_DIR" ] || [ -d "$CONFIG_DIR" ]; }; then
                printf "\n${COLOR_YELLOW}${COLOR_BOLD}Notice:${COLOR_RESET} Package store and configuration exist at:\n"
                [ -d "$DATA_DIR" ] && printf "  - %s\n" "$DATA_DIR"
                [ -d "$CONFIG_DIR" ] && printf "  - %s\n" "$CONFIG_DIR"
                if prompt_confirm "Do you also want to purge all stored packages and configuration? [y/N]: " "n"; then
                    PURGE=1
                else
                    log_info "Preserving user package store and configuration."
                fi
            fi
        fi
    fi

    if [ "$DRY_RUN" = "1" ]; then
        log_info "${COLOR_YELLOW}[DRY-RUN MODE]${COLOR_RESET} No files will be modified or removed."
    fi

    # 4. Remove Binary Executable(s)
    safe_remove_file() {
        _file="$1"
        _label="$2"
        if [ -z "$_file" ] || [ "$_file" = "/" ]; then
            return 1
        fi
        if [ -f "$_file" ] || [ -L "$_file" ]; then
            if [ "$DRY_RUN" = "1" ]; then
                log_info "Would remove $_label: ${COLOR_CYAN}${_file}${COLOR_RESET}"
            else
                if rm -f "$_file" 2>/dev/null; then
                    log_success "Removed $_label: ${COLOR_BOLD}${_file}${COLOR_RESET}"
                else
                    log_warn "Failed to remove '$_file'. You may need root/sudo privileges."
                fi
            fi
        fi
    }

    if [ -n "$FOUND_BINARIES" ]; then
        for BIN in $FOUND_BINARIES; do
            safe_remove_file "$BIN" "executable"
        done
    else
        log_info "No pkg binary found to remove."
    fi

    # 5. Purge Data, Config, and Cache Directories
    safe_remove_dir() {
        _dir="$1"
        _label="$2"
        if [ -z "$_dir" ] || [ "$_dir" = "/" ] || [ "$_dir" = "$HOME" ] || [ "$_dir" = "/usr" ] || [ "$_dir" = "/usr/local" ] || [ "$_dir" = "/var" ]; then
            log_error "Refusing to remove unsafe directory path: '$_dir'"
            return 1
        fi
        if [ -d "$_dir" ]; then
            if [ "$DRY_RUN" = "1" ]; then
                log_info "Would remove $_label: ${COLOR_CYAN}${_dir}${COLOR_RESET}"
            else
                if rm -rf "$_dir" 2>/dev/null; then
                    log_success "Removed $_label: ${COLOR_BOLD}${_dir}${COLOR_RESET}"
                else
                    log_warn "Failed to remove '$_dir'. Check permissions."
                fi
            fi
        fi
    }

    if [ "$PURGE" = "1" ]; then
        log_info "Purging pkg data, isolated store, and configurations..."
        safe_remove_dir "$DATA_DIR" "data & store directory"
        safe_remove_dir "$CONFIG_DIR" "configuration directory"
        safe_remove_dir "$CACHE_DIR" "cache directory"
        safe_remove_dir "$STATE_DIR" "state directory"
    else
        if [ -d "$DATA_DIR" ] || [ -d "$CONFIG_DIR" ]; then
            log_info "Kept data & configuration intact (use --purge to delete)."
        fi
    fi

    # 6. Clean Up Shell RC Files
    remove_from_rc() {
        _rc_file="$1"
        if [ -f "$_rc_file" ] && grep -Fq "pkg-managed" "$_rc_file" 2>/dev/null; then
            if [ "$DRY_RUN" = "1" ]; then
                log_info "Would clean pkg PATH configuration from: ${COLOR_CYAN}${_rc_file}${COLOR_RESET}"
                return 0
            fi
            _tmp_rc="${_rc_file}.pkg_clean.$$"
            awk '
                /# pkg-managed paths/ { skip=1; next }
                skip > 0 { skip--; next }
                /profiles\/default\/bin/ && /export PATH=.*pkg/ { next }
                /fish_add_path.*pkg/ { next }
                { print }
            ' "$_rc_file" > "$_tmp_rc" && mv -f "$_tmp_rc" "$_rc_file"
            log_success "Cleaned PATH configuration in ${COLOR_CYAN}${_rc_file}${COLOR_RESET}"
        fi
    }

    if [ "$NO_MODIFY_PATH" = "0" ]; then
        log_info "Checking shell configuration files for pkg PATH entries..."
        remove_from_rc "${HOME}/.zshrc"
        remove_from_rc "${HOME}/.bashrc"
        remove_from_rc "${HOME}/.bash_profile"
        remove_from_rc "${HOME}/.profile"
        remove_from_rc "${HOME}/.config/fish/config.fish"
    fi

    # 7. Summary
    if [ "$DRY_RUN" = "1" ]; then
        printf "\n${COLOR_YELLOW}${COLOR_BOLD}Dry-run completed.${COLOR_RESET} No changes were made.\n\n"
    else
        printf "\n${COLOR_GREEN}${COLOR_BOLD}pkg has been successfully uninstalled.${COLOR_RESET}\n"
        if [ "$NO_MODIFY_PATH" = "0" ]; then
            printf "Note: Any active terminal sessions will continue to have pkg in their current PATH until restarted.\n"
        fi
        printf "\n"
    fi
}

main "$@"
