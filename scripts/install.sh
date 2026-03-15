#!/usr/bin/env sh
set -e

REPO="null12138/rbot"
VERSION="${RBOT_VERSION:-latest}"
RBOT_HOME="${RBOT_HOME:-$HOME/.rbot}"
BIN_DIR="${RBOT_BIN_DIR:-}"

if [ -z "$BIN_DIR" ]; then
  if [ -w /usr/local/bin ]; then
    BIN_DIR="/usr/local/bin"
  else
    BIN_DIR="$HOME/.local/bin"
  fi
fi

need_cmd() {
  command -v "$1" >/dev/null 2>&1
}

download() {
  url="$1"
  out="$2"
  if need_cmd curl; then
    curl -fsSL "$url" -o "$out"
  elif need_cmd wget; then
    wget -qO "$out" "$url"
  else
    echo "error: curl or wget is required" >&2
    exit 1
  fi
}

fetch_json() {
  url="$1"
  if need_cmd curl; then
    curl -fsSL "$url"
  elif need_cmd wget; then
    wget -qO- "$url"
  else
    echo "error: curl or wget is required" >&2
    exit 1
  fi
}

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH_RAW="$(uname -m)"
case "$ARCH_RAW" in
  x86_64|amd64) ARCH="x86_64" ;;
  aarch64|arm64) ARCH="aarch64" ;;
  *)
    echo "error: unsupported arch: $ARCH_RAW" >&2
    exit 1
    ;;
 esac

case "$OS" in
  linux)
    TARGET="${ARCH}-unknown-linux-gnu"
    ;;
  darwin)
    TARGET="${ARCH}-apple-darwin"
    ;;
  freebsd)
    if [ "$ARCH" != "x86_64" ]; then
      echo "error: freebsd only supports x86_64 releases currently" >&2
      exit 1
    fi
    TARGET="${ARCH}-unknown-freebsd"
    ;;
  *)
    echo "error: unsupported OS: $OS" >&2
    exit 1
    ;;
 esac

API_URL="https://api.github.com/repos/${REPO}/releases/latest"
if [ "$VERSION" != "latest" ]; then
  API_URL="https://api.github.com/repos/${REPO}/releases/tags/${VERSION}"
fi

json="$(fetch_json "$API_URL")"
asset="rbot-${TARGET}.tar.gz"
url="$(printf "%s" "$json" | grep -oE "https://[^\" ]*${asset}" | head -n1)"

if [ -z "$url" ]; then
  asset="rbot-${TARGET}.zip"
  url="$(printf "%s" "$json" | grep -oE "https://[^\" ]*${asset}" | head -n1)"
fi

if [ -z "$url" ]; then
  echo "error: no release asset for ${TARGET}" >&2
  exit 1
fi

tmpdir="$(mktemp -d)"
cleanup() { rm -rf "$tmpdir"; }
trap cleanup EXIT

archive="$tmpdir/$asset"

download "$url" "$archive"

case "$asset" in
  *.tar.gz)
    tar -xzf "$archive" -C "$tmpdir"
    ;;
  *.zip)
    if ! need_cmd unzip; then
      echo "error: unzip is required to extract $asset" >&2
      exit 1
    fi
    unzip -q "$archive" -d "$tmpdir"
    ;;
  *)
    echo "error: unknown archive format: $asset" >&2
    exit 1
    ;;
 esac

mkdir -p "$RBOT_HOME/bin" "$RBOT_HOME/config" "$RBOT_HOME/skills" "$RBOT_HOME/data" "$RBOT_HOME/memory" "$BIN_DIR"

if [ -f "$tmpdir/rbot" ]; then
  cp "$tmpdir/rbot" "$RBOT_HOME/bin/rbot"
elif [ -f "$tmpdir/rbot-${TARGET}" ]; then
  cp "$tmpdir/rbot-${TARGET}" "$RBOT_HOME/bin/rbot"
else
  echo "error: extracted binary not found" >&2
  exit 1
fi
chmod +x "$RBOT_HOME/bin/rbot"

cat > "$BIN_DIR/rbot" <<'EOS'
#!/usr/bin/env sh
set -e
RBOT_HOME="${RBOT_HOME:-$HOME/.rbot}"
cd "$RBOT_HOME"
exec "$RBOT_HOME/bin/rbot" "$@"
EOS
chmod +x "$BIN_DIR/rbot"

if [ ! -f "$RBOT_HOME/config/config.toml" ]; then
  echo "Running rbot init..."
  RBOT_HOME="$RBOT_HOME" "$BIN_DIR/rbot" init
fi

if ! echo "$PATH" | tr ':' '\n' | grep -qx "$BIN_DIR"; then
  echo "NOTE: add $BIN_DIR to your PATH to use 'rbot'"
fi

echo "Install complete. Run: rbot"
