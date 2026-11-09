#!/usr/bin/env bash
# Installs and launches the macOS bundle the way a user would, on a fresh
# runner: mount the .dmg, copy Scribe.app into an Applications folder, start
# it and check it stays up, then let it download the default model through its
# verified download path and transcribe the speech clip with the bundled
# whisper.cpp. The Intel slice runs the same self-test under Rosetta.
#
# Usage: smoke-macos.sh <candidate dir> <clip.wav> <expected version>
set -euo pipefail

candidate="$(cd "$1" && pwd)"
clip="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"
version="$3"
out="${RUNNER_TEMP:-/tmp}/scribe-smoke"
mkdir -p "$out"

dmg="$candidate/Scribe-macos-universal.dmg"
apps="$out/Applications"
app="$apps/Scribe.app"
exe="$app/Contents/MacOS/Scribe"
cli="$app/Contents/Resources/bin/macos/whisper-cli"
models="$HOME/Library/Application Support/com.natkins.scribe/models"

fail() {
  echo "::error::$*" >&2
  exit 1
}

# perl's alarm is the timeout (macOS has no `timeout`), so a hung process
# fails the job fast.
with_timeout() {
  local seconds="$1"
  shift
  perl -e 'alarm shift; exec @ARGV or die "exec failed: $!"' "$seconds" "$@"
}

launch_check() {
  local log="$out/launch.log"
  "$exe" > "$log" 2>&1 &
  local pid=$!
  local second
  for second in $(seq 1 15); do
    sleep 1
    if ! kill -0 "$pid" 2> /dev/null; then
      local status=0
      wait "$pid" || status=$?
      cat "$log"
      fail "Scribe exited after $second s with status $status"
    fi
  done
  echo "Scribe is still running after 15 s"
  kill -TERM "$pid" 2> /dev/null || true
  for second in $(seq 1 15); do
    kill -0 "$pid" 2> /dev/null || break
    sleep 1
  done
  kill -KILL "$pid" 2> /dev/null || true
  wait "$pid" 2> /dev/null || true
  if pgrep -x Scribe > /dev/null; then
    fail "Scribe did not stop"
  fi
}

self_test() {
  local label="$1"
  shift
  local result="$out/self-test-$label.json"
  local start end status=0
  start=$(date +%s)
  with_timeout 600 "$@" "$exe" --self-test-transcribe "$clip" "$result" || status=$?
  end=$(date +%s)
  echo "Self-test ($label) exited with $status after $((end - start)) s"
  [ -f "$result" ] || fail "The self-test ($label) wrote no result"
  cat "$result"
  echo
  jq -e '.ok == true' "$result" > /dev/null || fail "The self-test ($label) failed"
  [ "$status" -eq 0 ] || fail "The self-test ($label) reported success but exited with $status"
  jq -e '.text | test("country")' "$result" > /dev/null || fail "The transcript ($label) did not match the clip"
  jq -e --arg v "$version" '.appVersion == $v' "$result" > /dev/null ||
    fail "The self-test ($label) ran version $(jq -r .appVersion "$result"), not $version"
}

# The bundled whisper-cli on its own, per slice, with and without Metal: the
# default run of each slice must transcribe the clip; the backend lines and
# timings are printed for the record.
whisper_check() {
  local model arch gpu text start end
  model="$(find "$models" -name '*.bin' | head -n 1)"
  [ -n "$model" ] || fail "No model was downloaded to $models"
  for arch in arm64 x86_64; do
    for gpu in "" --no-gpu; do
      start=$(date +%s)
      text="$(with_timeout 300 arch "-$arch" "$cli" -m "$model" -f "$clip" --no-timestamps $gpu \
        2> "$out/whisper-$arch${gpu}.log")" || fail "whisper-cli ($arch ${gpu:-gpu on}) could not transcribe the clip"
      end=$(date +%s)
      echo "::group::whisper-cli $arch ${gpu:-(gpu on)}: $((end - start)) s"
      grep -iE 'metal|gpu|backend|system_info|total time' "$out/whisper-$arch${gpu}.log" || true
      echo "$text"
      echo "::endgroup::"
      grep -q country <<< "$text" || fail "whisper-cli ($arch ${gpu:-gpu on}) transcript did not match the clip"
    done
  done
}

(cd "$candidate" && shasum -a 256 --check SHA256SUMS.txt)
[ -f "$dmg" ] || fail "Missing $dmg"

mnt="$out/dmg"
mkdir -p "$mnt" "$apps"
hdiutil attach -nobrowse -readonly -mountpoint "$mnt" "$dmg"
[ -d "$mnt/Scribe.app" ] || fail "The .dmg has no Scribe.app"
[ -L "$mnt/Applications" ] || echo "Note: the .dmg has no Applications shortcut"
ditto "$mnt/Scribe.app" "$app"
hdiutil detach "$mnt"
codesign --verify --deep --strict "$app"
plist_version="$(plutil -extract CFBundleShortVersionString raw -o - "$app/Contents/Info.plist")"
[ "$plist_version" = "$version" ] || fail "Scribe.app is version $plist_version, not $version"
"$cli" --help > /dev/null 2>&1 || fail "whisper-cli --help failed"

launch_check
self_test arm64 env

# The runner is a new install, so the app registered the default hotkeys at
# startup: each one must be a shortcut macOS accepts.
hotkeys="$(cat "$HOME/Library/Logs/com.natkins.scribe"/*.log | grep -E 'hotkey bindings|Hotkey registration failed' || true)"
echo "$hotkeys"
if [ -z "$hotkeys" ] || grep -q 'Hotkey registration failed' <<< "$hotkeys"; then
  fail "A default hotkey did not register"
fi

whisper_check
self_test x86_64 arch -x86_64

echo "Scribe.app installed from the .dmg, launched and transcribed the clip on both slices."
