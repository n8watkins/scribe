#!/usr/bin/env bash
# Installs and launches the Linux bundles the way a user would, on a fresh
# runner that has none of the build's -dev packages: the .deb through apt
# (which also proves its declared dependencies resolve), then the AppImage
# through FUSE. Each one must start and stay up, and must transcribe the
# speech clip end to end with its own bundled whisper.cpp.
#
# Usage: smoke-linux.sh <candidate dir> <clip.wav> <expected version>
# Run it under a display and a session bus, as a desktop has them:
#   xvfb-run -a dbus-run-session -- bash smoke-linux.sh ...
set -euo pipefail

candidate="$(realpath "$1")"
clip="$(realpath "$2")"
version="$3"
out="${RUNNER_TEMP:-/tmp}/scribe-smoke"
mkdir -p "$out"

deb="$candidate/Scribe-linux-amd64.deb"
appimage="$candidate/Scribe-linux-x86_64.AppImage"
data="${XDG_DATA_HOME:-$HOME/.local/share}/com.natkins.scribe"

fail() {
  echo "::error::$*" >&2
  exit 1
}

# Starts the app in its own process group, checks it is still running after
# 15 seconds, then stops the whole group (the AppImage runtime and the app it
# mounted) so the single-instance guard does not hand the next launch to it.
launch_check() {
  local label="$1"
  shift
  local log="$out/launch-$label.log"
  setsid "$@" > "$log" 2>&1 &
  local pid=$!
  local second
  for second in $(seq 1 15); do
    sleep 1
    if ! kill -0 "$pid" 2> /dev/null; then
      local status=0
      wait "$pid" || status=$?
      cat "$log"
      fail "Scribe ($label) exited after $second s with status $status"
    fi
  done
  echo "Scribe ($label) is still running after 15 s"
  kill -TERM -- "-$pid" 2> /dev/null || true
  for second in $(seq 1 15); do
    kill -0 "$pid" 2> /dev/null || break
    sleep 1
  done
  kill -KILL -- "-$pid" 2> /dev/null || true
  wait "$pid" 2> /dev/null || true
  if pgrep -x Scribe > /dev/null; then
    fail "Scribe ($label) did not stop"
  fi
}

# The installed app downloads the default model through its verified download
# path (the first run only), transcribes the clip with the bundled whisper.cpp,
# writes the result and exits.
self_test() {
  local label="$1"
  shift
  local result="$out/self-test-$label.json"
  local status=0
  timeout --kill-after=30 600 "$@" --self-test-transcribe "$clip" "$result" || status=$?
  echo "Self-test ($label) exited with $status"
  [ -f "$result" ] || fail "The self-test ($label) wrote no result"
  cat "$result"
  echo
  jq -e '.ok == true' "$result" > /dev/null || fail "The self-test ($label) failed"
  [ "$status" -eq 0 ] || fail "The self-test ($label) reported success but exited with $status"
  jq -e '.text | test("country")' "$result" > /dev/null || fail "The transcript ($label) did not match the clip"
  jq -e --arg v "$version" '.appVersion == $v' "$result" > /dev/null ||
    fail "The self-test ($label) ran version $(jq -r .appVersion "$result"), not $version"
}

# A bundled whisper-cli on its own: it must start without any library the
# bundle does not ship and transcribe the clip with the downloaded model.
whisper_check() {
  local label="$1" cli="$2"
  [ -x "$cli" ] || fail "The $label bundle has no executable whisper-cli at $cli"
  "$cli" --help > /dev/null 2>&1 || fail "whisper-cli --help failed in the $label bundle"
  local model
  model="$(find "$data/models" -name '*.bin' | head -n 1)"
  [ -n "$model" ] || fail "No model was downloaded to $data/models"
  local text
  text="$(timeout 300 "$cli" -m "$model" -f "$clip" --no-timestamps 2> "$out/whisper-$label.log")" ||
    fail "whisper-cli ($label) could not transcribe the clip"
  echo "whisper-cli ($label): $text"
  grep -q country <<< "$text" || fail "whisper-cli ($label) transcript did not match the clip"
}

(cd "$candidate" && sha256sum --check SHA256SUMS.txt)
[ -f "$deb" ] || fail "Missing $deb"
[ -f "$appimage" ] || fail "Missing $appimage"

echo "::group::Install the .deb with apt"
sudo apt-get install -y "$deb"
echo "::endgroup::"
package="$(dpkg-deb --field "$deb" Package)"
installed="$(dpkg-query --showformat '${Version}' --show "$package")"
[ "$installed" = "$version" ] || fail "apt installed $package $installed, not $version"
exe="$(dpkg -L "$package" | grep -E '^/usr/bin/[^/]+$' | head -n 1)"
[ -x "$exe" ] || fail "The .deb installed no executable in /usr/bin"
deb_cli="$(dpkg -L "$package" | grep -E '/bin/linux/whisper-cli$' | head -n 1)"
echo "Installed $package $installed: $exe"

launch_check deb "$exe"
self_test deb "$exe"
whisper_check deb "$deb_cli"

echo "::group::Remove the .deb"
sudo apt-get remove -y "$package"
echo "::endgroup::"
[ ! -e "$exe" ] || fail "$exe remained after apt remove"

# The AppImage the way a user runs it: marked executable and mounted through
# FUSE (libfuse2, which the install docs ask for), from a home folder.
app="$HOME/Applications/Scribe.AppImage"
mkdir -p "$HOME/Applications"
cp "$appimage" "$app"
chmod +x "$app"
launch_check appimage "$app"
self_test appimage "$app"
(cd "$out" && "$app" --appimage-extract > /dev/null)
whisper_check appimage "$(find "$out/squashfs-root" -path '*/bin/linux/whisper-cli' | head -n 1)"
rm -rf "$out/squashfs-root"

echo "The .deb and the AppImage installed, launched and transcribed the clip."
