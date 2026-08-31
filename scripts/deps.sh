#!/usr/bin/env bash
# Murk build dependency check / install.
#
# Source of truth is the pkg-config module list: module names are identical on
# every distribution, package names are not. The package table below exists only
# to print a ready-to-paste install command.
set -euo pipefail

# --- pkg-config modules Murk links against ----------------------------------
MODULES=(
  gtk+-3.0
  webkit2gtk-4.1
  javascriptcoregtk-4.1
  libsoup-3.0
  mpv
  epoxy
  # Pulled in by tao (the windowing layer under Tauri) on every Linux build,
  # not by anything Murk asks for directly. Debian's webkit dev package happens
  # to drag it in transitively and ALT's does not, which is exactly the kind of
  # difference this list exists to paper over.
  dbus-1
)

# libmpv2 speaks client API 2.x, i.e. libmpv.so.2 (mpv >= 0.36).
# Note: `pkg-config --modversion mpv` reports the *client API* version (2.x),
# not the mpv release version, so the gate is on 2.0.
MPV_MIN_API=2.0

# --- distribution detection --------------------------------------------------
# ALT Linux ships apt-get with rpm-flavoured, ALT-specific package names and has
# no ID_LIKE, so any "has apt => use Debian names" heuristic silently installs
# the wrong thing there. Match on ID first, and match altlinux before anything
# tries to sniff a package manager.
detect_distro() {
  if [ -r /etc/os-release ]; then
    # shellcheck disable=SC1091
    . /etc/os-release
    case "${ID:-}" in
      altlinux)                 echo alt     ; return ;;
      debian|ubuntu|linuxmint|pop|elementary)
                                echo debian  ; return ;;
      fedora|rhel|centos|rocky|almalinux)
                                echo fedora  ; return ;;
      arch|manjaro|endeavouros) echo arch    ; return ;;
    esac
    for like in ${ID_LIKE:-}; do
      case "$like" in
        debian) echo debian ; return ;;
        fedora|rhel) echo fedora ; return ;;
        arch)   echo arch   ; return ;;
      esac
    done
  fi
  echo unknown
}

packages_for() {
  case "$1" in
    alt)
      echo "libgtk+3-devel libwebkit2gtk4.1-devel libsoup3.0-devel \
libayatana-appindicator3-devel librsvg-devel libmpv-devel libepoxy-devel \
libdbus-devel gcc gcc-c++ make pkg-config"
      ;;
    debian)
      echo "libgtk-3-dev libwebkit2gtk-4.1-dev libsoup-3.0-dev \
libayatana-appindicator3-dev librsvg2-dev libmpv-dev libepoxy-dev \
libdbus-1-dev build-essential pkg-config"
      ;;
    fedora)
      echo "gtk3-devel webkit2gtk4.1-devel libsoup3-devel \
libayatana-appindicator-gtk3-devel librsvg2-devel mpv-libs-devel libepoxy-devel \
dbus-devel gcc gcc-c++ make pkgconf-pkg-config"
      ;;
    arch)
      echo "gtk3 webkit2gtk-4.1 libsoup3 libayatana-appindicator librsvg mpv \
libepoxy dbus base-devel"
      ;;
  esac
}

# Root already, or in a container: `sudo` would not merely be redundant, it is
# usually not installed at all. The CI matrix runs this as root.
sudo_prefix() {
  if [ "$(id -u)" -eq 0 ]; then
    echo ""
  elif command -v sudo >/dev/null 2>&1; then
    echo "sudo "
  else
    echo ""
  fi
}

install_cmd_for() {
  local distro="$1" pkgs s
  pkgs="$(packages_for "$distro")"
  s="$(sudo_prefix)"
  case "$distro" in
    # ALT uses apt-get too, but with its own package names, hence the separate
    # branch rather than a shared "apt" case.
    alt)    echo "${s}apt-get update && ${s}apt-get install -y $pkgs" ;;
    debian) echo "${s}apt-get update && ${s}apt-get install -y $pkgs" ;;
    fedora) echo "${s}dnf install -y $pkgs" ;;
    arch)   echo "${s}pacman -Sy --needed --noconfirm $pkgs" ;;
  esac
}

# --- checks ------------------------------------------------------------------
missing=()

check_modules() {
  local ok=0
  if ! command -v pkg-config >/dev/null 2>&1; then
    echo "  pkg-config itself is missing" >&2
    missing+=(pkg-config)
    return 1
  fi
  for m in "${MODULES[@]}"; do
    if pkg-config --exists "$m" 2>/dev/null; then
      printf '  \033[32m ok \033[0m %-24s %s\n' "$m" "$(pkg-config --modversion "$m")"
    else
      printf '  \033[31mmiss\033[0m %-24s\n' "$m"
      missing+=("$m")
      ok=1
    fi
  done
  return $ok
}

check_mpv_version() {
  pkg-config --exists mpv 2>/dev/null || return 0   # already reported as missing
  if ! pkg-config --atleast-version="$MPV_MIN_API" mpv; then
    echo
    echo "  libmpv client API $(pkg-config --modversion mpv) is too old:" >&2
    echo "  Murk needs libmpv.so.2 (client API >= $MPV_MIN_API, mpv >= 0.36)." >&2
    echo "  Distributions still on libmpv.so.1 (Ubuntu 22.04, Debian 12) should" >&2
    echo "  use the Flatpak build instead." >&2
    return 1
  fi
}

usage() {
  cat <<EOF
usage: scripts/deps.sh [--check | --install | --list]

  --check    (default) report which pkg-config modules are missing and print
             the install command for this distribution. Changes nothing.
  --install  run that install command.
  --list     print the required pkg-config modules, one per line.
EOF
}

main() {
  local mode="${1:---check}"
  case "$mode" in
    --list) printf '%s\n' "${MODULES[@]}"; return 0 ;;
    --check|--install) ;;
    -h|--help) usage; return 0 ;;
    *) usage >&2; return 2 ;;
  esac

  local distro
  distro="$(detect_distro)"
  echo "distribution: $distro"
  echo "checking pkg-config modules:"

  local mods_ok=0 ver_ok=0
  check_modules || mods_ok=1
  check_mpv_version || ver_ok=1

  if [ "$mods_ok" -eq 0 ] && [ "$ver_ok" -eq 0 ]; then
    echo
    echo "all build dependencies present."
    return 0
  fi

  if [ "$distro" = unknown ]; then
    echo
    echo "Unrecognised distribution. Install the development packages providing" >&2
    echo "these pkg-config modules, then re-run this script:" >&2
    printf '  %s\n' "${MODULES[@]}" >&2
    return 1
  fi

  local cmd
  cmd="$(install_cmd_for "$distro")"

  if [ "$mode" = --install ]; then
    echo
    echo "+ $cmd"
    eval "$cmd"
    echo
    echo "re-checking:"
    missing=()
    check_modules && check_mpv_version && { echo; echo "all build dependencies present."; return 0; }
    return 1
  fi

  echo
  echo "install with:"
  echo "  $cmd"
  return 1
}

main "$@"
