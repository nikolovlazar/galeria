#!/bin/sh
# Exercise the updater entirely against a disposable local release and app bundle.
set -eu

fail() {
  echo "not ok - $*" >&2
  exit 1
}

# The updater detaches its helper into another process group. Match only
# processes whose arguments mention this unique sandbox, never other Galeria
# instances or services. Stop whole groups so a helper's ditto/sleep exits too.
sandbox_processes() {
  python3 - "$tmp" "$server_pid" "$$" "$1" <<'PY'
import os
import re
import signal
import subprocess
import sys
import time

sandbox, server_pid, shell_pid, action = sys.argv[1:]
exclude = {os.getpid(), int(server_pid or 0), int(shell_pid)}
own_group = os.getpgrp()

def matches():
    try:
        result = subprocess.run(
            ["ps", "-axww", "-o", "pid=", "-o", "pgid=", "-o", "command="],
            check=True, stdout=subprocess.PIPE, text=True,
        )
    except (OSError, subprocess.CalledProcessError) as error:
        print(f"could not inspect sandbox processes: {error}", file=sys.stderr)
        sys.exit(2)
    records = []
    for line in result.stdout.splitlines():
        match = re.match(r"^\s*(\d+)\s+(\d+)\s+(.*)$", line)
        if match:
            records.append([int(match.group(1)), int(match.group(2)), match.group(3)])
        elif records:
            # A shell's -c script can span several lines in ps output.
            records[-1][2] += " " + line
    return [(pid, pgid) for pid, pgid, command in records
            if pid not in exclude and sandbox in command]

if action == "running":
    sys.exit(0 if matches() else 1)

for _ in range(40):
    found = matches()
    if not found:
        break
    for pid, pgid in found:
        try:
            if pgid != own_group:
                os.killpg(pgid, signal.SIGKILL)
            else:
                os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
    time.sleep(0.05)
else:
    sys.exit("could not stop sandbox processes")
PY
}

tmp=
server_pid=
apply_group=
cleanup() {
  result=$?
  trap - 0
  if [ -n "$apply_group" ]; then
    # Stop the CLI's session, then find the separately detached swap helper.
    python3 - "$apply_group" <<'PY' || :
import os
import signal
import sys

try:
    os.killpg(int(sys.argv[1]), signal.SIGKILL)
except ProcessLookupError:
    pass
PY
  fi
  if [ -n "$tmp" ]; then
    if ! sandbox_processes stop; then
      echo "not ok - could not stop sandbox processes" >&2
      result=1
    fi
  fi
  if [ -n "$server_pid" ]; then
    kill "$server_pid" 2>/dev/null || :
    wait "$server_pid" 2>/dev/null || :
  fi
  if [ -n "$tmp" ]; then
    rm -rf "$tmp"
  fi
  exit "$result"
}
trap cleanup 0
trap 'exit 1' HUP INT TERM

[ "$(uname -s)" = Darwin ] || fail "this smoke test requires macOS"
for tool in python3 ditto codesign plutil shasum; do
  command -v "$tool" >/dev/null 2>&1 || fail "required command not found: $tool"
done

root=$(CDPATH= cd "$(dirname "$0")/../.." && pwd -P)
cd "$root"
binary=${GALERIA_BIN:-target/debug/galeria}
[ -x "$binary" ] || fail "executable not found at $binary; run cargo build first or set GALERIA_BIN"

tmp=$(mktemp -d "${TMPDIR:-/tmp}/galeria-update-smoke.XXXXXXXX") || fail "could not create sandbox"
tmp=$(CDPATH= cd "$tmp" && pwd -P)
app="$tmp/Galeria.app"
new_app="$tmp/new/Galeria.app"
mkdir -p "$app/Contents/MacOS" "$new_app/Contents/MacOS" \
  "$new_app/Contents/Resources" "$tmp/web" "$tmp/home"
cp "$binary" "$app/Contents/MacOS/galeria"
cp "$binary" "$new_app/Contents/MacOS/galeria"

write_plist() {
  cat > "$1/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>galeria</string>
  <key>CFBundleIdentifier</key><string>com.nikolovlazar.galeria</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$2</string>
</dict>
</plist>
PLIST
}
write_plist "$app" 0.2.0
write_plist "$new_app" 9.9.9
printf 'updated\n' > "$new_app/Contents/Resources/smoke-marker"
if ! codesign --force --sign - "$app" >"$tmp/codesign.log" 2>&1; then
  fail "could not sign initial app: $(cat "$tmp/codesign.log")"
fi
if ! codesign --force --sign - "$new_app" >"$tmp/codesign.log" 2>&1; then
  fail "could not sign update app: $(cat "$tmp/codesign.log")"
fi

zip="$tmp/web/Galeria-macos-arm64.zip"
if ! ditto -c -k --sequesterRsrc --keepParent "$new_app" "$zip" >"$tmp/ditto.log" 2>&1; then
  fail "could not create update zip: $(cat "$tmp/ditto.log")"
fi
# Give the zip a comment so one byte can be changed without damaging its
# contents or code signature: rejection then specifically tests the feed hash.
python3 - "$zip" <<'PY'
import sys
import zipfile

with zipfile.ZipFile(sys.argv[1], "a") as archive:
    archive.comment = b"galeria updater smoke"
PY
sha256=$(shasum -a 256 "$zip" | awk '{print $1}')
size=$(wc -c < "$zip" | tr -d '[:space:]')

# An ephemeral port avoids collisions with other local services. Disable proxy
# settings so the updater and readiness probe cannot leave the loopback host.
unset HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy
NO_PROXY=127.0.0.1,localhost
no_proxy=$NO_PROXY
export NO_PROXY no_proxy
python3 -u -m http.server 0 --bind 127.0.0.1 --directory "$tmp/web" >"$tmp/server.log" 2>&1 &
server_pid=$!
port=
ready=0
attempt=0
while [ "$attempt" -lt 100 ]; do
  port=$(sed -n 's/^Serving HTTP on .* port \([0-9][0-9]*\) .*/\1/p' "$tmp/server.log" | head -n 1)
  if [ -n "$port" ] && python3 - "$port" <<'PY'
import socket
import sys

try:
    with socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=0.2):
        pass
except OSError:
    sys.exit(1)
PY
  then
    ready=1
    break
  fi
  kill -0 "$server_pid" 2>/dev/null || fail "local feed server exited: $(cat "$tmp/server.log")"
  attempt=$((attempt + 1))
  sleep 0.1
done
[ "$ready" -eq 1 ] || fail "local feed server did not become ready: $(cat "$tmp/server.log")"

python3 - "$tmp/web" "$port" "$sha256" "$size" <<'PY'
import json
import os
import sys

web, port, digest, size = sys.argv[1:]
asset = {
    "url": f"http://127.0.0.1:{port}/Galeria-macos-arm64.zip",
    "sha256": digest,
    "size": int(size),
}
for name, version in (("latest.json", "9.9.9"), ("older.json", "0.1.0")):
    with open(os.path.join(web, name), "w", encoding="utf-8") as feed:
        json.dump({"version": version, "platforms": {"macos-aarch64": asset}}, feed)
PY
feed="http://127.0.0.1:$port/latest.json"
older_feed="http://127.0.0.1:$port/older.json"
installed="$app/Contents/MacOS/galeria"
plist="$app/Contents/Info.plist"
marker="$app/Contents/Resources/smoke-marker"

check() {
  if GALERIA_HOME="$tmp/home" GALERIA_UPDATE_FEED="$1" \
      "$installed" update --check --json >"$tmp/check.json" 2>"$tmp/check.log"; then
    :
  else
    code=$?
    fail "check exit code $code: $(cat "$tmp/check.log")"
  fi
  python3 - "$tmp/check.json" "$2" <<'PY'
import json
import sys

try:
    with open(sys.argv[1], encoding="utf-8") as source:
        status = json.load(source)
    state = status["state"]
    actual = state["state"]
except (OSError, ValueError, KeyError, TypeError) as error:
    sys.exit(f"not ok - check JSON: {error}")

expected = sys.argv[2]
if actual != expected:
    sys.exit(f"not ok - check JSON state: expected {expected!r}, got {actual!r}; "
             f"error: {state.get('error')!r}")
print(f"ok - check reports {expected}")
if expected == "available":
    if state.get("version") != "9.9.9":
        sys.exit(f"not ok - check JSON version: expected '9.9.9', got {state.get('version')!r}")
    print("ok - available version is 9.9.9")
    if status.get("canInstall") is not True:
        sys.exit(f"not ok - check JSON canInstall: expected true, got {status.get('canInstall')!r}")
    print("ok - installation is available")
PY
}
check "$feed" available
check "$older_feed" upToDate

# Isolate the CLI so cleanup can stop it independently if interrupted. Its
# detached helper is tracked separately by its sandbox destination argument.
apply() {
  GALERIA_HOME="$tmp/home" GALERIA_UPDATE_FEED="$feed" \
    python3 - "$installed" "$1" >"$2" 2>&1 <<'PY' &
import os
import sys

os.setsid()
os.execv(sys.argv[1], [sys.argv[1], "update", "--apply", sys.argv[2], "--no-relaunch"])
PY
  apply_group=$!
  wait "$apply_group"
}
sandbox_busy() {
  if sandbox_processes running; then
    return 0
  else
    code=$?
    [ "$code" -eq 1 ] || fail "could not inspect swap helper processes"
    return 1
  fi
}
if apply "$zip" "$tmp/apply.log"; then
  echo "ok - valid zip apply exited successfully"
else
  code=$?
  fail "apply exit code $code: $(cat "$tmp/apply.log")"
fi

attempt=0
swapped=0
while [ "$attempt" -lt 100 ]; do
  version=$(plutil -extract CFBundleShortVersionString raw "$plist" 2>/dev/null || :)
  if [ "$version" = 9.9.9 ] && [ -f "$marker" ] && \
      codesign --verify --deep --strict "$app" >/dev/null 2>&1 && \
      ! sandbox_busy; then
    swapped=1
    break
  fi
  attempt=$((attempt + 1))
  sleep 0.1
done
if [ "$swapped" -ne 1 ]; then
  helper_log="$tmp/home/cache/updates/helper.log"
  if [ -f "$helper_log" ]; then
    details=$(cat "$helper_log")
  else
    details=$(cat "$tmp/apply.log")
  fi
  fail "version not swapped to 9.9.9 with a signed marker bundle (last version: ${version:-missing}): $details"
fi
apply_group=
echo "ok - installed bundle version is 9.9.9"
echo "ok - installed bundle contains the marker"

bad_zip="$tmp/tampered.zip"
cp "$zip" "$bad_zip"
python3 - "$bad_zip" <<'PY'
import sys

with open(sys.argv[1], "r+b") as archive:
    archive.seek(-1, 2)
    byte = archive.read(1)
    archive.seek(-1, 2)
    archive.write(bytes([byte[0] ^ 1]))
PY
if apply "$bad_zip" "$tmp/tampered.log"; then
  fail "tampered zip accepted (apply exit code 0)"
else
  echo "ok - tampered zip rejected"
fi
attempt=0
while sandbox_busy; do
  [ "$attempt" -lt 100 ] || fail "tampered zip apply left a running helper"
  attempt=$((attempt + 1))
  sleep 0.1
done
apply_group=
version=$(plutil -extract CFBundleShortVersionString raw "$plist" 2>/dev/null || :)
[ "$version" = 9.9.9 ] && [ -f "$marker" ] || fail "version changed after tampered zip apply (last version: ${version:-missing})"
echo "ok - installed version remains 9.9.9"
