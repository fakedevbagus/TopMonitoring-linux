#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
MODE="${1:-check}"
CANDIDATE="$(cat "$ROOT/SOURCE_CANDIDATE" 2>/dev/null || printf 'unknown')"
ORIGINAL_CONFIG_HOME="${XDG_CONFIG_HOME:-${HOME}/.config}"
STATE_ROOT="${TOPMONITORING_TEST_ROOT:-${ROOT}/.test-state/no-wayland}"
ARTIFACT_ROOT="${TOPMONITORING_TEST_ARTIFACTS:-${ROOT}/test-artifacts/no-wayland}"
mkdir -p "$STATE_ROOT" "$ARTIFACT_ROOT"

usage() {
  cat <<'EOF'
Usage: ./scripts/test-no-wayland.sh [check|smoke|manual|clean-state]

  check       Format, test, Clippy, and release-build the no-Wayland target.
  smoke       Launch the release candidate with isolated config briefly.
  manual      Launch the release candidate with isolated config until stopped.
  clean-state Remove isolated test config/cache/data and previous logs.

Environment:
  TOPMONITORING_COPY_CONFIG=1   Copy the currently installed config into the
                                isolated test profile before launch.
  TOPMONITORING_TEST_SECONDS=15 Smoke-test duration.
  TOPMONITORING_TEST_ROOT=...   Override isolated XDG state directory.
EOF
}

if [[ "$MODE" == "clean-state" ]]; then
  rm -rf "$STATE_ROOT" "$ARTIFACT_ROOT"
  echo "Removed isolated no-Wayland test state."
  exit 0
fi
if [[ "$MODE" != "check" && "$MODE" != "smoke" && "$MODE" != "manual" ]]; then
  usage >&2
  exit 2
fi

for command in cargo rustc pkg-config python3; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "Missing required command: $command" >&2
    exit 127
  fi
done

export XDG_CONFIG_HOME="$STATE_ROOT/config"
export XDG_CACHE_HOME="$STATE_ROOT/cache"
export XDG_DATA_HOME="$STATE_ROOT/data"
mkdir -p "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_DATA_HOME"

if [[ "${TOPMONITORING_COPY_CONFIG:-0}" == "1" ]]; then
  source_config="$ORIGINAL_CONFIG_HOME/topmonitoring/config.toml"
  destination="$XDG_CONFIG_HOME/topmonitoring/config.toml"
  if [[ -f "$source_config" && ! -f "$destination" ]]; then
    mkdir -p "$(dirname "$destination")"
    cp -- "$source_config" "$destination"
    chmod 600 "$destination"
    echo "Copied installed config into isolated test state."
  fi
fi

printf 'Candidate: %s\nMode: %s\nSession: %s\nBinary under test: %s\nIsolated config: %s\n' \
  "$CANDIDATE" "$MODE" "${XDG_SESSION_TYPE:-unknown}" "$ROOT/target/release/topmonitoring" \
  "$XDG_CONFIG_HOME/topmonitoring/config.toml"

if [[ "$MODE" == "check" ]]; then
  cargo fmt --check
  cargo test --locked --no-default-features
  cargo clippy --locked --all-targets --no-default-features -- -D warnings
  cargo build --release --locked --no-default-features
  PYTHONDONTWRITEBYTECODE=1 python3 scripts/check_source.py
  PYTHONDONTWRITEBYTECODE=1 python3 scripts/check_release_docs.py
  bash -n install.sh uninstall.sh scripts/test-no-wayland.sh scripts/package-no-wayland.sh
  echo "No-Wayland terminal gate: PASS"
  exit 0
fi

if [[ -z "${DISPLAY:-}" && -z "${WAYLAND_DISPLAY:-}" ]]; then
  echo "No graphical display detected. Run smoke/manual from a desktop terminal." >&2
  exit 2
fi
if pgrep -x -u "$(id -u)" topmonitoring >/dev/null 2>&1; then
  cat >&2 <<'EOF'
A TopMonitoring process is already running. Stop it before testing so the GTK
single-instance application opens the candidate binary:

  pkill -x topmonitoring

This stops the process only; it does not uninstall your stable package.
EOF
  exit 2
fi
if [[ "${XDG_SESSION_TYPE:-unknown}" != "x11" ]]; then
  echo "WARNING: a no-Wayland build cannot reserve screen space on Wayland."
  echo "Use an X11 session for the authoritative docking test."
fi

cargo build --release --locked --no-default-features
binary="$ROOT/target/release/topmonitoring"
log="$ARTIFACT_ROOT/${MODE}-$(date +%Y%m%d-%H%M%S).log"

if [[ "$MODE" == "manual" ]]; then
  echo "Launching candidate. Press Ctrl+C after completing the manual checklist."
  echo "Log: $log"
  set +e
  RUST_BACKTRACE=1 "$binary" 2>&1 | tee "$log"
  result=${PIPESTATUS[0]}
  set -e
  case "$result" in
    0|130|143)
      echo "Manual run finished. Record the checklist result separately."
      exit 0
      ;;
    *)
      echo "Candidate failed with exit code $result; inspect $log" >&2
      exit "$result"
      ;;
  esac
fi

seconds="${TOPMONITORING_TEST_SECONDS:-15}"
echo "Running ${seconds}s smoke test. Log: $log"
set +e
RUST_BACKTRACE=1 timeout --signal=TERM --kill-after=3s "${seconds}s" \
  "$binary" 2>&1 | tee "$log"
result=${PIPESTATUS[0]}
set -e
case "$result" in
  124|137|143)
    echo "No-Wayland GUI smoke test: PASS (process stayed alive until timeout)"
    ;;
  0)
    echo "Candidate exited before the smoke-test timeout; inspect $log" >&2
    exit 1
    ;;
  *)
    echo "Candidate failed with exit code $result; inspect $log" >&2
    exit "$result"
    ;;
esac
