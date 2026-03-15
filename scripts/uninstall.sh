#!/usr/bin/env sh
set -e

RBOT_HOME="${RBOT_HOME:-$HOME/.rbot}"
BIN_DIR="${RBOT_BIN_DIR:-}"
KEEP_CONFIG="${RBOT_KEEP_CONFIG:-}"

if [ -z "$BIN_DIR" ]; then
  if [ -w /usr/local/bin ]; then
    BIN_DIR="/usr/local/bin"
  else
    BIN_DIR="$HOME/.local/bin"
  fi
fi

# Remove wrapper
if [ -f "$BIN_DIR/rbot" ]; then
  rm -f "$BIN_DIR/rbot"
fi

# Remove installed binary
if [ -f "$RBOT_HOME/bin/rbot" ]; then
  rm -f "$RBOT_HOME/bin/rbot"
fi

if [ -n "$KEEP_CONFIG" ]; then
  echo "Uninstall complete. Kept config/data at $RBOT_HOME (RBOT_KEEP_CONFIG set)."
  exit 0
fi

# Purge home
if [ -d "$RBOT_HOME" ]; then
  rm -rf "$RBOT_HOME"
fi

echo "Uninstall complete."
