#!/usr/bin/env bash
# Package target/release/rpp and the bundled TypeScript compiler as a Linux x64 release archive.
set -euo pipefail

typescript_version=7.0.2
typescript_sha256=da2513f4b95176d6dde8b51aab7afe8a927656c9d277369793f77f7e59371c08
native_sha256=7ecad6f67377e831856367ab062ef394f21506a611405bf8ac0ff039348637d3
registry=https://registry.npmjs.org

output=target/dist
while [ "$#" -gt 0 ]; do
  case "$1" in
    --output) output=${2:?--output needs a directory}; shift 2 ;;
    *) echo "Usage: package.sh [--output DIR]" >&2; exit 1 ;;
  esac
done

repository=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repository"
if [ "$(uname -s)" != Linux ] || [ "$(uname -m)" != x86_64 ]; then
  echo "Release archives support Linux x64." >&2
  exit 1
fi

version=${RPP_RELEASE_VERSION:-$(sed -n '/^\[workspace\.package\]/,/^\[/{s/^version = "\(.*\)"$/\1/p}' Cargo.toml)}
if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  echo "Release archives require a semantic version, got '$version'." >&2
  exit 1
fi
executable=target/release/rpp
actual=$("$executable" --version)
if [ "$actual" != "rpp $version" ]; then
  echo "Expected 'rpp $version', got '$actual'." >&2
  exit 1
fi

name=rpp-$version-linux-x64
mkdir -p "$output"
output=$(cd "$output" && pwd)
archive=$output/$name.tar.gz
if [ -e "$archive" ] || [ -e "$archive.sha256" ]; then
  echo "Refusing to replace $archive; use a fresh output directory." >&2
  exit 1
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
fetch() {
  curl --fail --silent --show-error --location --retry 3 --proto '=https' --tlsv1.2 \
    "$registry/$1" -o "$work/$2"
  echo "$3  $work/$2" | sha256sum --check --quiet
}
fetch "typescript/-/typescript-$typescript_version.tgz" typescript.tgz "$typescript_sha256"
fetch "@typescript/typescript-linux-x64/-/typescript-linux-x64-$typescript_version.tgz" \
  native.tgz "$native_sha256"

root=$work/$name
compiler=$root/toolchain/typescript/$typescript_version
mkdir -p "$compiler" "$work/typescript" "$work/native"
tar -xzf "$work/typescript.tgz" -C "$work/typescript" --strip-components=1
tar -xzf "$work/native.tgz" -C "$work/native" --strip-components=1
install -m 755 "$executable" "$root/rpp"
install -m 644 LICENSE-MIT LICENSE-APACHE "$root/"
printf '{"version":"%s","source":"%s"}\n' "$version" "${GITHUB_SHA:-$(git rev-parse HEAD)}" > "$root/release.json"
cp "$work"/native/lib/* "$compiler/"
install -m 644 "$work/typescript/LICENSE" "$work/typescript/NOTICE.txt" "$compiler/"
chmod 755 "$compiler/tsc"
find "$root" -type d -exec chmod 755 {} +
find "$root" -type f ! -perm /111 -exec chmod 644 {} +

tar --sort=name --owner=0 --group=0 --numeric-owner --mtime=@0 -C "$work" -cf - "$name" |
  gzip -n >"$work/$name.tar.gz"
mv "$work/$name.tar.gz" "$archive"
(cd "$output" && sha256sum "$name.tar.gz" >"$name.tar.gz.sha256")
echo "$archive"
