#!/bin/sh
set -eu

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd -P)
vellum=$(CDPATH= cd -- "$repo/../vellum" && pwd -P)
scratch=$(mktemp -d "${TMPDIR:-/tmp}/vellum-tui-live.XXXXXX")
chmod 700 "$scratch"
socket="$scratch/service.sock"
control="$scratch/control.sock"
source_file="$scratch/source.txt"
expected_file="$scratch/expected.txt"
daemon_target="$scratch/daemon-target"
daemon="$daemon_target/debug/vellum-daemon"
tui_target="$scratch/tui-target"
tui="$tui_target/debug/vellum-tui"
log="$scratch/daemon.log"
pid=

cleanup() {
  if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
    if [ -S "$control" ]; then
      "$daemon" --shutdown "$control" >/dev/null 2>&1 || true
    fi
    wait "$pid" 2>/dev/null || true
  fi
  rm -rf "$scratch"
}
trap cleanup EXIT HUP INT TERM

printf 'alpha' >"$source_file"
printf 'Xalpha' >"$expected_file"
cargo build --manifest-path "$vellum/Cargo.toml" --package vellum-daemon \
  --bin vellum-daemon --locked --offline --target-dir "$daemon_target"
"$daemon" --socket "$socket" --control "$control" --allow-edit --allow-save \
  -- "$source_file" >"$log" 2>&1 &
pid=$!

attempt=0
while [ "$attempt" -lt 100 ]; do
  if "$daemon" --status "$control" 2>/dev/null | grep -q '"readiness":"ready"'; then
    break
  fi
  if ! kill -0 "$pid" 2>/dev/null; then
    sed -n '1,120p' "$log" >&2
    echo 'FAIL: daemon exited before readiness' >&2
    exit 1
  fi
  attempt=$((attempt + 1))
  sleep 0.05
done
if [ "$attempt" -eq 100 ]; then
  sed -n '1,120p' "$log" >&2
  echo 'FAIL: daemon readiness timeout' >&2
  exit 1
fi

cargo build --manifest-path "$repo/Cargo.toml" -p vellum-tui-cli \
  --bin vellum-tui --locked --offline --target-dir "$tui_target"
headless_status=$("$tui" headless --service "$socket")
printf '%s' "$headless_status" | python3 -c '
import json, sys
report = json.load(sys.stdin)
assert report == {
    "version": 1, "mode": "headless", "complete": True,
    "peer": "vellum", "capability": "workspace.inventory.v1",
    "status": "current", "documents": 1, "roots": 1,
    "disconnect": "clean",
}
'

VELLUM_LIVE_SOCKET="$socket" cargo test --manifest-path "$repo/Cargo.toml" \
  --test live_service --locked --offline -- --ignored --nocapture
cmp -s "$source_file" "$expected_file" || {
  echo 'FAIL: service save did not publish exact source bytes' >&2
  exit 1
}
"$daemon" --shutdown "$control" >/dev/null
wait "$pid"
pid=
echo 'PASS: real daemon -> headless status + TUI controller open/edit/save/disconnect'
