#!/usr/bin/env sh
set -e

REPO="${RBOT_REPO:-null12138/rbot}"
VERSION="${RBOT_VERSION:-latest}"
RBOT_HOME="${RBOT_HOME:-$HOME/.rbot}"
BIN_DIR="${RBOT_BIN_DIR:-}"
SESSION="${RBOT_TMUX_SESSION:-rbot}"
KEEP_CONFIG="${RBOT_KEEP_CONFIG:-}"

usage() {
  cat <<'EOF'
Usage: rbot.sh <command> [args]

Commands:
  install            Install rbot (default)
  update             Update rbot
  uninstall          Uninstall rbot
  start [args...]    Start in tmux (RBOT_TMUX_SESSION)
  stop               Stop tmux session
  restart [args...]  Restart tmux session
  status             Show running status
  logs [lines]       Show tmux logs (default 200)
  run [args...]      Run in foreground
  help               Show this help

Env:
  RBOT_REPO, RBOT_VERSION, RBOT_HOME, RBOT_BIN_DIR, RBOT_TMUX_SESSION, RBOT_KEEP_CONFIG
EOF
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1
}

ensure_bin_dir() {
  if [ -z "$BIN_DIR" ]; then
    if [ -w /usr/local/bin ]; then
      BIN_DIR="/usr/local/bin"
    else
      BIN_DIR="$HOME/.local/bin"
    fi
  fi
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

detect_target() {
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
}

fetch_release() {
  api_url="https://api.github.com/repos/${REPO}/releases/tags/${VERSION}"
  fetch_json "$api_url"
}

install_or_update() {
  mode="$1"
  ensure_bin_dir
  detect_target

  json="$(fetch_release)"
  asset="rbot-${TARGET}.tar.gz"
  url="$(printf "%s" "$json" | grep -oE "https://[^\" ]*${asset}" | head -n1)"

  if [ -z "$url" ]; then
    asset="rbot-${TARGET}.zip"
    url="$(printf "%s" "$json" | grep -oE "https://[^\" ]*${asset}" | head -n1)"
  fi

  if [ -z "$url" ]; then
    echo "error: no release asset for ${TARGET} (tag: ${VERSION})" >&2
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

  if [ "$mode" = "install" ] && [ ! -f "$RBOT_HOME/config/config.toml" ]; then
    echo "Running rbot init..."
    RBOT_HOME="$RBOT_HOME" "$BIN_DIR/rbot" init
  fi

  if ! echo "$PATH" | tr ':' '\n' | grep -qx "$BIN_DIR"; then
    echo "NOTE: add $BIN_DIR to your PATH to use 'rbot'"
  fi

  if [ "$mode" = "install" ]; then
    label="Install"
  else
    label="Update"
  fi
  echo "$label complete. Run: rbot"
}

uninstall() {
  ensure_bin_dir
  if [ -f "$BIN_DIR/rbot" ]; then
    rm -f "$BIN_DIR/rbot"
  fi
  if [ -f "$RBOT_HOME/bin/rbot" ]; then
    rm -f "$RBOT_HOME/bin/rbot"
  fi
  if [ -n "$KEEP_CONFIG" ]; then
    echo "Uninstall complete. Kept config/data at $RBOT_HOME (RBOT_KEEP_CONFIG set)."
    exit 0
  fi
  if [ -d "$RBOT_HOME" ]; then
    rm -rf "$RBOT_HOME"
  fi
  echo "Uninstall complete."
}

ensure_tmux() {
  if ! need_cmd tmux; then
    echo "error: tmux is required for this command" >&2
    exit 1
  fi
}

session_exists() {
  tmux has-session -t "$SESSION" 2>/dev/null
}

start_tmux() {
  ensure_tmux
  if session_exists; then
    echo "rbot is already running in tmux session: $SESSION"
    exit 0
  fi
  if [ ! -x "$RBOT_HOME/bin/rbot" ]; then
    echo "error: rbot is not installed at $RBOT_HOME/bin/rbot" >&2
    exit 1
  fi
  cmd="$RBOT_HOME/bin/rbot"
  if [ "$#" -gt 0 ]; then
    cmd="$cmd $*"
  fi
  tmux new-session -d -s "$SESSION" "RBOT_HOME=$RBOT_HOME $cmd"
  echo "Started in tmux session: $SESSION"
}

stop_tmux() {
  ensure_tmux
  if session_exists; then
    tmux kill-session -t "$SESSION"
    echo "Stopped tmux session: $SESSION"
  else
    echo "rbot is not running"
  fi
}

status_tmux() {
  if ! need_cmd tmux; then
    echo "tmux not installed"
    exit 1
  fi
  if session_exists; then
    echo "running (tmux session: $SESSION)"
  else
    echo "stopped"
  fi
}

logs_tmux() {
  ensure_tmux
  lines="${1:-200}"
  if ! session_exists; then
    echo "rbot is not running"
    exit 1
  fi
  tmux capture-pane -t "$SESSION" -p -S -"$lines"
}

run_foreground() {
  if [ ! -x "$RBOT_HOME/bin/rbot" ]; then
    echo "error: rbot is not installed at $RBOT_HOME/bin/rbot" >&2
    exit 1
  fi
  RBOT_HOME="$RBOT_HOME" exec "$RBOT_HOME/bin/rbot" "$@"
}

cmd="${1:-install}"
shift || true

case "$cmd" in
  install) install_or_update install ;;
  update) install_or_update update ;;
  uninstall) uninstall ;;
  start) start_tmux "$@" ;;
  stop) stop_tmux ;;
  restart) stop_tmux || true; start_tmux "$@" ;;
  status) status_tmux ;;
  logs) logs_tmux "$@" ;;
  run) run_foreground "$@" ;;
  help|-h|--help) usage ;;
  *)
    echo "error: unknown command: $cmd" >&2
    usage
    exit 1
    ;;
esac
