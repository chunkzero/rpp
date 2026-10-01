#!/bin/sh
set -eu

version=${1:?Usage: install.sh VERSION [ARCHIVE]}
case "$version" in
  *[!0-9A-Za-z.-]* | .* | *..* | "") echo "Invalid version" >&2; exit 1 ;;
esac
if [ "$(uname -s)" != Linux ] || [ "$(uname -m)" != x86_64 ]; then
  echo "rpp release archives support Linux x64." >&2
  exit 1
fi

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

name="rpp-$version-linux-x64"
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT
trap 'exit 1' HUP INT TERM
if [ "$#" -ge 2 ]; then
  cp "$2" "$temporary/$name.tar.gz"
  cp "$2.sha256" "$temporary/$name.tar.gz.sha256"
else
  url=${RPP_RELEASE_URL:-"https://github.com/chunkzero/rpp/releases/download/v$version"}
  case "$url" in
    https://* | http://127.0.0.1[:/]*) ;;
    *) echo "RPP_RELEASE_URL must use https:// (or http://127.0.0.1)." >&2; exit 1 ;;
  esac
  curl --fail --location --tlsv1.2 "$url/$name.tar.gz" -o "$temporary/$name.tar.gz"
  curl --fail --location --tlsv1.2 "$url/$name.tar.gz.sha256" -o "$temporary/$name.tar.gz.sha256"
fi

cd "$temporary"
expected=$(cut -d ' ' -f 1 "$name.tar.gz.sha256")
actual=$(sha256sum "$name.tar.gz" | cut -d ' ' -f 1)
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
mv "$name" "$destination"
ln -sfn "$destination/rpp" "$prefix/bin/rpp"
echo "Installed rpp $version. Add $prefix/bin to PATH."
