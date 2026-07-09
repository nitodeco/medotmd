#!/usr/bin/env sh
set -eu

repo="nitodeco/medotmd"
install_dir="${MEDOTMD_INSTALL_DIR:-$HOME/.local/bin}"
version="${MEDOTMD_VERSION:-latest}"

uname_s="$(uname -s)"
uname_m="$(uname -m)"

case "$uname_s:$uname_m" in
  Linux:x86_64)
    target="x86_64-unknown-linux-gnu"
    ;;
  Darwin:x86_64)
    target="x86_64-apple-darwin"
    ;;
  Darwin:arm64)
    target="aarch64-apple-darwin"
    ;;
  *)
    echo "unsupported platform: $uname_s $uname_m" >&2
    exit 1
    ;;
esac

archive="medotmd-$target.tar.gz"

if [ "$version" = "latest" ]; then
  base_url="https://github.com/$repo/releases/latest/download"
else
  base_url="https://github.com/$repo/releases/download/$version"
fi

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

curl -fsSL "$base_url/$archive" -o "$tmp_dir/$archive"
curl -fsSL "$base_url/$archive.sha256" -o "$tmp_dir/$archive.sha256"

(
  cd "$tmp_dir"
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 -c "$archive.sha256"
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum -c "$archive.sha256"
  else
    echo "missing shasum or sha256sum" >&2
    exit 1
  fi
)

tar -xzf "$tmp_dir/$archive" -C "$tmp_dir"
mkdir -p "$install_dir"
cp "$tmp_dir/medotmd" "$install_dir/medotmd"
chmod +x "$install_dir/medotmd"

echo "installed medotmd to $install_dir/medotmd"
echo "run: medotmd init"
