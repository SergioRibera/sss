#!/bin/sh
# sss / sss_code installer.
#
# Detects (arch, os, distro), composes the right release-asset filename
# and installs it natively. Tarball fallback drops the bare binary into
# $HOME/.local/bin so the script works on any POSIX shell.
#
# Tags use a `<binary>/v<version>` layout
# (`sss_cli/v0.2.1`, `sss_code/v0.3.1`); pick which binary + variant
# you want before running.
#
# Usage:
#   curl -fsSL https://github.com/SergioRibera/sss/releases/latest/download/install.sh | sh
#   curl -fsSL .../install.sh | sh -s -- --binary sss_code
#   curl -fsSL .../install.sh | sh -s -- --variant nvidia
#
# Flags:
#   --binary sss|sss_code       which CLI (default: sss)
#   --variant system|nvidia|rocm|noocr   sss only (default: system)
#   --version vX.Y.Z            pin a specific release tag
#   --format FMT                force a packaging format
#   --dir DIR                   install dir for tarball fallback
#   --uninstall                 remove the installed package
#   -h, --help                  this help
#
# Env vars: BINARY, VARIANT, VERSION, FORMAT, INSTALL_DIR, SUDO, REPO.

set -eu

REPO=${REPO:-SergioRibera/sss}
BINARY=${BINARY:-sss}
VARIANT=${VARIANT:-system}
VERSION=${VERSION:-}
FORMAT=${FORMAT:-}
INSTALL_DIR=${INSTALL_DIR:-"$HOME/.local/bin"}
UNINSTALL=0

usage() {
  cat <<'EOF'
sss / sss_code installer.

Detects (arch, os, distro), composes the right release-asset filename
and installs it natively. Tarball fallback drops the bare binary into
$HOME/.local/bin so the script works on any POSIX shell.

Usage:
  curl -fsSL https://github.com/SergioRibera/sss/releases/latest/download/install.sh | sh
  curl -fsSL .../install.sh | sh -s -- --binary sss_code
  curl -fsSL .../install.sh | sh -s -- --variant nvidia

Flags:
  --binary sss|sss_code              which CLI (default: sss)
  --variant system|nvidia|rocm|noocr sss only (default: system)
  --version vX.Y.Z                   pin a specific release tag
  --format FMT                       force a packaging format
  --dir DIR                          install dir for tarball fallback
  --uninstall                        remove the installed package
  -h, --help                         this help

Env vars: BINARY, VARIANT, VERSION, FORMAT, INSTALL_DIR, SUDO, REPO.
EOF
}

die() { echo "install.sh: $*" >&2; exit 1; }

while [ "$#" -gt 0 ]; do
  case "$1" in
    --binary)    BINARY="$2"; shift 2 ;;
    --variant)   VARIANT="$2"; shift 2 ;;
    --version)   VERSION="${2#v}"; shift 2 ;;
    --format)    FORMAT="$2"; shift 2 ;;
    --dir)       INSTALL_DIR="$2"; shift 2 ;;
    --uninstall) UNINSTALL=1; shift ;;
    -h|--help)   usage; exit 0 ;;
    *)           die "unknown arg: $1" ;;
  esac
done

case "$BINARY" in
  sss|sss_code) ;;
  *) die "--binary must be 'sss' or 'sss_code' (got '$BINARY')" ;;
esac

# sss_code only ships the default variant. Silently coerce.
[ "$BINARY" = "sss_code" ] && VARIANT=system

case "$VARIANT" in
  system|nvidia|rocm|noocr) ;;
  *) die "--variant must be system|nvidia|rocm|noocr (got '$VARIANT')" ;;
esac

OS=$(uname -s)
ARCH=$(uname -m)
case "$ARCH" in amd64) ARCH=x86_64 ;; arm64) ARCH=aarch64 ;; esac

# rocm slice is x86_64-linux only.
if [ "$VARIANT" = "rocm" ] && [ "$ARCH:$OS" != "x86_64:Linux" ]; then
  die "rocm variant only ships for x86_64 Linux"
fi
# nvidia variant is Linux-only.
if [ "$VARIANT" = "nvidia" ] && [ "$OS" != "Linux" ]; then
  die "nvidia variant only ships for Linux"
fi

# Resolve the latest tag for the binary when the user didn't pin one.
# Tag streams follow the crate name, not the binary name:
#   binary `sss`      → crate `sss_cli` → tags `sss_cli/v*`
#   binary `sss_code` → crate `sss_code` → tags `sss_code/v*`
case "$BINARY" in
  sss)      TAG_PREFIX="sss_cli/v" ;;
  sss_code) TAG_PREFIX="sss_code/v" ;;
esac
if [ -z "$VERSION" ]; then
  if command -v curl >/dev/null 2>&1; then
    api_get() { curl -fsSL "$1"; }
  elif command -v wget >/dev/null 2>&1; then
    api_get() { wget -qO- "$1"; }
  else
    die "need curl or wget on PATH"
  fi
  # The /latest endpoint returns the most recent published release,
  # which mixes binaries. Walk the tag list and grab the highest
  # `<BINARY>/v...` entry instead.
  VERSION=$(
    api_get "https://api.github.com/repos/$REPO/releases?per_page=30" \
      | awk -F'"' -v p="$TAG_PREFIX" '
          $2=="tag_name" && index($4, p)==1 {
            sub(p, "", $4); print $4; exit
          }
        '
  )
  [ -n "$VERSION" ] || die "could not resolve latest $BINARY release tag"
fi

TAG="${TAG_PREFIX}${VERSION}"
RELEASE_URL="https://github.com/${REPO}/releases/download/${TAG}"

# Package basename used in filenames.
#   sss      → sss / sss-nvidia / sss-rocm / sss-noocr
#   sss_code → sss_code
case "$BINARY" in
  sss)
    case "$VARIANT" in
      system) PKG=sss ;;
      *)      PKG="sss-${VARIANT}" ;;
    esac
    ;;
  sss_code) PKG=sss_code ;;
esac

# Bin name installed by tarball formats. archlinux/deb/rpm bundles
# always drop the canonical `sss` / `sss_code` regardless of variant.
BIN="$BINARY"

deb_arch()   { case "$1" in x86_64) echo amd64 ;; aarch64) echo arm64 ;; esac; }
arch_tuple() {
  # `<arch>-<os>` slug used by tar.gz / tar.zst filenames.
  case "$2" in
    Linux)  echo "$1-linux" ;;
    Darwin) echo "$1-darwin" ;;
  esac
}
darwin_arch() { case "$1" in x86_64) echo amd64 ;; aarch64) echo arm64 ;; esac; }

# Compose `<pkg>-<ver>-...` asset filename for a given format.
file_for() {
  fmt=$1
  case "$fmt" in
    deb)       echo "${PKG}_${VERSION}_$(deb_arch "$ARCH").deb" ;;
    rpm)       echo "${PKG}-${VERSION}-1.${ARCH}.rpm" ;;
    archlinux) echo "${PKG}-${VERSION}-1-${ARCH}.pkg.tar.zst" ;;
    tar.gz)    echo "${PKG}-${VERSION}-$(arch_tuple "$ARCH" "$OS").tar.gz" ;;
    tar.zst)   echo "${PKG}-${VERSION}-$(arch_tuple "$ARCH" "$OS").tar.zst" ;;
    pkg)       [ "$OS" = "Darwin" ] || return 1
               echo "${PKG}-${VERSION}-$(darwin_arch "$ARCH").pkg" ;;
    dmg)       [ "$OS" = "Darwin" ] || return 1
               echo "${PKG}-${VERSION}-$(darwin_arch "$ARCH").dmg" ;;
    *) return 1 ;;
  esac
}

detect_format() {
  if [ -n "$FORMAT" ]; then echo "$FORMAT"; return; fi
  case "$OS" in
    Linux)
      if [ -r /etc/os-release ]; then
        # shellcheck disable=SC1091
        . /etc/os-release
        family=" ${ID:-} ${ID_LIKE:-} "
        case "$family" in
          *" debian "*|*" ubuntu "*) echo deb; return ;;
          *" fedora "*|*" rhel "*|*" centos "*|*" suse "*|*" opensuse-tumbleweed "*|*" opensuse-leap "*)
            echo rpm; return ;;
          *" arch "*|*" manjaro "*|*" endeavouros "*) echo archlinux; return ;;
        esac
      fi
      if   command -v dpkg   >/dev/null 2>&1; then echo deb
      elif command -v rpm    >/dev/null 2>&1; then echo rpm
      elif command -v pacman >/dev/null 2>&1; then echo archlinux
      else echo tar.gz
      fi
      ;;
    Darwin) echo pkg ;;
    *)      die "unsupported OS: $OS" ;;
  esac
}

SUDO=${SUDO:-}
if [ -z "$SUDO" ] && [ "$(id -u)" -ne 0 ] && command -v sudo >/dev/null 2>&1; then
  SUDO=sudo
fi

uninstall_pkg() {
  fmt=$1
  case "$fmt" in
    deb)       $SUDO dpkg -r "$PKG" ;;
    rpm)       $SUDO rpm -e "$PKG" ;;
    archlinux) $SUDO pacman -R --noconfirm "$PKG" ;;
    pkg|dmg)
      # macOS installs land under /opt/<bin>. There's no Apple receipt
      # to consult reliably, so we sweep the known install layout.
      $SUDO rm -rf "/opt/$BIN" "/Applications/$BIN.app" ;;
    tar.gz|tar.zst) rm -f "$INSTALL_DIR/$BIN" ;;
    *) die "don't know how to uninstall format '$fmt'" ;;
  esac
}

install_pkg() {
  fmt=$1; src=$2
  case "$fmt" in
    deb)
      if command -v apt-get >/dev/null 2>&1; then
        $SUDO apt-get install -y "$src"
      else
        $SUDO dpkg -i "$src"
      fi
      ;;
    rpm)
      if   command -v dnf    >/dev/null 2>&1; then $SUDO dnf install -y "$src"
      elif command -v yum    >/dev/null 2>&1; then $SUDO yum install -y "$src"
      elif command -v zypper >/dev/null 2>&1; then
        $SUDO zypper --non-interactive install --allow-unsigned-rpm "$src"
      else $SUDO rpm -i --force "$src"
      fi
      ;;
    archlinux) $SUDO pacman -U --noconfirm "$src" ;;
    tar.gz|tar.zst)
      ex=$(mktemp -d)
      tar -xf "$src" -C "$ex"
      bin=$(find "$ex" -type f -name "$BIN" | head -n1)
      [ -n "$bin" ] || die "binary '$BIN' missing inside tarball"
      mkdir -p "$INSTALL_DIR"
      install -m 0755 "$bin" "$INSTALL_DIR/$BIN"
      rm -rf "$ex"
      case ":$PATH:" in
        *":$INSTALL_DIR:"*) ;;
        *) echo "note: $INSTALL_DIR is not on \$PATH" ;;
      esac
      ;;
    pkg) $SUDO installer -pkg "$src" -target / ;;
    dmg)
      mount=$(hdiutil attach -nobrowse -readonly "$src" | tail -n1 | awk '{print $NF}')
      $SUDO cp -R "$mount"/*.app /Applications/
      hdiutil detach "$mount" -quiet
      ;;
    *) die "unsupported format: $fmt" ;;
  esac
}

FMT=$(detect_format)
FILE=$(file_for "$FMT") || die "no asset for $ARCH/$OS/$FMT"

if [ "$UNINSTALL" = "1" ]; then
  uninstall_pkg "$FMT"
  echo "Removed $PKG ($FMT)."
  exit 0
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

URL="$RELEASE_URL/$FILE"
echo "Downloading $URL"

if command -v curl >/dev/null 2>&1; then
  curl -fsSL "$URL" -o "$tmp/bundle"
elif command -v wget >/dev/null 2>&1; then
  wget -qO "$tmp/bundle" "$URL"
else
  die "need curl or wget on PATH"
fi

# Verify against SHA256SUMS when both the manifest and `sha256sum` are
# available. Missing manifest is not fatal — older releases skipped it.
if command -v sha256sum >/dev/null 2>&1; then
  sums=$tmp/sums
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$RELEASE_URL/SHA256SUMS" -o "$sums" 2>/dev/null || true
  else
    wget -qO "$sums" "$RELEASE_URL/SHA256SUMS" 2>/dev/null || true
  fi
  if [ -s "$sums" ]; then
    expected=$(awk -v f="$FILE" '$2==f {print $1; exit}' "$sums")
    if [ -n "$expected" ]; then
      actual=$(sha256sum "$tmp/bundle" | awk '{print $1}')
      [ "$expected" = "$actual" ] || die "SHA256 mismatch for $FILE"
      echo "SHA256 verified."
    fi
  fi
fi

install_pkg "$FMT" "$tmp/bundle"
echo "Installed $PKG $VERSION ($FMT)."
