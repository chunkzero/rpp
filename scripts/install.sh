#!/bin/sh
set -eu

version=${1:?Usage: install.sh VERSION [ARCHIVE]}
case "$version" in
  *[!0-9A-Za-z.-]* | .* | *..* | "") echo "Invalid version" >&2; exit 1 ;;
esac
case "$(uname -s):$(uname -m)" in
  Linux:x86_64) platform=linux-x64 ;;
  Linux:aarch64 | Linux:arm64) platform=linux-arm64 ;;
  Darwin:x86_64) platform=darwin-x64 ;;
  Darwin:arm64) platform=darwin-arm64 ;;
  *) echo "Unsupported rpp platform; use mise on Windows." >&2; exit 1 ;;
esac

prefix=${RPP_INSTALL_DIR:-"$HOME/.local"}
case "$prefix" in
  /*) ;;
  *) prefix="$(pwd)/$prefix" ;;
esac
destination="$prefix/share/rpp/$version"
if [ -e "$destination" ] || [ -L "$destination" ]; then
  echo "Already installed: $destination" >&2
  exit 1
fi
if [ -e "$prefix/bin/rpp" ] && [ ! -L "$prefix/bin/rpp" ]; then
  echo "Refusing to replace $prefix/bin/rpp; choose another RPP_INSTALL_DIR." >&2
  exit 1
fi

name="rpp-$version-$platform"
temporary=$(mktemp -d)
installed=
cleanup() {
  rm -rf "$temporary"
  if [ -n "$installed" ]; then rm -rf "$destination"; fi
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
if [ "$#" -ge 2 ]; then
  cp "$2" "$temporary/$name.tar.gz"
  cp "$2.sha256" "$temporary/$name.tar.gz.sha256"
else
  url=${RPP_RELEASE_URL:-"https://github.com/chunkzero/rpp/releases/download/v$version"}
  authority=${url#*://}
  authority=${authority%%/*}
  case "$authority" in
    *@*) echo "RPP_RELEASE_URL must not contain credentials." >&2; exit 1 ;;
  esac
  case "$url" in
    https://*) transport="--location --proto =https --proto-redir =https --tlsv1.2" ;;
    http://*)
      host=${authority%%:*}
      port=${authority#"$host"}
      port=${port#:}
      case "$host:$port" in
        127.0.0.1: | localhost: | 127.0.0.1:[0-9]* | localhost:[0-9]*) ;;
        *) echo "RPP_RELEASE_URL must use https:// (or http://127.0.0.1)." >&2; exit 1 ;;
      esac
      case "$port" in
        *[!0-9]*) echo "RPP_RELEASE_URL has an invalid port." >&2; exit 1 ;;
      esac
      transport="--proto =http --max-redirs 0" ;;
    *) echo "RPP_RELEASE_URL must use https:// (or http://127.0.0.1)." >&2; exit 1 ;;
  esac
  # shellcheck disable=SC2086
  curl --fail $transport "$url/$name.tar.gz" -o "$temporary/$name.tar.gz"
  # shellcheck disable=SC2086
  curl --fail $transport "$url/$name.tar.gz.sha256" -o "$temporary/$name.tar.gz.sha256"
fi

cd "$temporary"
expected=$(cut -d ' ' -f 1 "$name.tar.gz.sha256")
case "$expected" in
  *[!0-9a-f]*) expected= ;;
esac
if [ "${#expected}" -ne 64 ]; then
  echo "rpp checksum file is invalid." >&2
  exit 1
fi
actual=$(if command -v sha256sum >/dev/null 2>&1; then sha256sum "$name.tar.gz"; else shasum -a 256 "$name.tar.gz"; fi) || { echo "Could not compute the rpp checksum." >&2; exit 1; }
actual=${actual%% *}
if [ "$expected" != "$actual" ]; then
  echo "rpp checksum does not match." >&2
  exit 1
fi
tar -xzf "$name.tar.gz"
if [ "$("$temporary/$name/rpp" --version)" != "rpp $version" ]; then
  echo "rpp version does not match." >&2
  exit 1
fi
mkdir -p "$prefix/share/rpp" "$prefix/bin"
installed=1
mv "$name" "$destination"
ln -sfn "$destination/rpp" "$prefix/bin/rpp"
installed=
echo "Installed rpp $version. Add $prefix/bin to PATH."
