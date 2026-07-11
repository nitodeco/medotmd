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
signature="$archive.sig"
manifest="medotmd-$target.manifest"
manifest_signature="$manifest.sig"
release_signing_public_key='-----BEGIN PUBLIC KEY-----
MCowBQYDK2VwAyEA+vCqLtCTcrwVqJFp+zb6+KUpIZqJi+5VcSu/L/b2+94=
-----END PUBLIC KEY-----'

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

release_download_prefix="https://github.com/$repo/releases/download/"

if [ "$version" = "latest" ]; then
  latest_release_headers_path="$tmp_dir/latest-release-headers"

  if ! curl -fsS -D "$latest_release_headers_path" -o /dev/null \
    "https://github.com/$repo/releases/latest/download/$manifest"; then
    echo "failed to resolve the latest medotmd release" >&2
    exit 1
  fi

  latest_release_location="$(
    awk 'tolower($1) == "location:" { sub(/\r$/, "", $2); print $2; exit }' \
      "$latest_release_headers_path"
  )"

  case "$latest_release_location" in
    "$release_download_prefix"*"/$manifest")
      release_tag="${latest_release_location#"$release_download_prefix"}"
      release_tag="${release_tag%"/$manifest"}"
      ;;
    *)
      echo "latest medotmd release did not resolve to a versioned release tag" >&2
      exit 1
      ;;
  esac
else
  release_tag="$version"
fi

case "$release_tag" in
  v[0-9]*)
    ;;
  *)
    echo "invalid medotmd release tag: $release_tag" >&2
    exit 1
    ;;
esac

case "$release_tag" in
  *[!0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz.+-]*)
    echo "invalid medotmd release tag: $release_tag" >&2
    exit 1
    ;;
esac

release_version="${release_tag#v}"
base_url="$release_download_prefix$release_tag"

curl -fsSL "$base_url/$archive" -o "$tmp_dir/$archive"
curl -fsSL "$base_url/$archive.sha256" -o "$tmp_dir/$archive.sha256"
curl -fsSL "$base_url/$signature" -o "$tmp_dir/$signature"
curl -fsSL "$base_url/$manifest" -o "$tmp_dir/$manifest"
curl -fsSL "$base_url/$manifest_signature" -o "$tmp_dir/$manifest_signature"

if ! command -v openssl >/dev/null 2>&1; then
  echo "missing openssl" >&2
  exit 1
fi

printf '%s\n' "$release_signing_public_key" > "$tmp_dir/release-signing-public-key.pem"
openssl pkeyutl -verify -rawin -pubin \
  -inkey "$tmp_dir/release-signing-public-key.pem" \
  -in "$tmp_dir/$manifest" \
  -sigfile "$tmp_dir/$manifest_signature" >/dev/null

if ! awk -F= '
  BEGIN {
    is_valid = 1
    expected_fields["version"] = 1
    expected_fields["tag"] = 1
    expected_fields["target"] = 1
    expected_fields["archive"] = 1
    expected_fields["sha256"] = 1
  }
  NF != 2 || $2 == "" || !($1 in expected_fields) || seen_fields[$1]++ {
    is_valid = 0
    next
  }
  END {
    exit !(is_valid && NR == 5 && seen_fields["version"] && seen_fields["tag"] && seen_fields["target"] && seen_fields["archive"] && seen_fields["sha256"])
  }
' "$tmp_dir/$manifest"; then
  echo "release manifest is invalid" >&2
  exit 1
fi

manifest_version="$(awk -F= '$1 == "version" { print $2 }' "$tmp_dir/$manifest")"
manifest_tag="$(awk -F= '$1 == "tag" { print $2 }' "$tmp_dir/$manifest")"
manifest_target="$(awk -F= '$1 == "target" { print $2 }' "$tmp_dir/$manifest")"
manifest_archive="$(awk -F= '$1 == "archive" { print $2 }' "$tmp_dir/$manifest")"
manifest_sha256="$(awk -F= '$1 == "sha256" { print $2 }' "$tmp_dir/$manifest")"

if [ "$manifest_version" != "$release_version" ] \
  || [ "$manifest_tag" != "$release_tag" ] \
  || [ "$manifest_target" != "$target" ] \
  || [ "$manifest_archive" != "$archive" ]; then
  echo "release manifest does not match the requested release" >&2
  exit 1
fi

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

if command -v shasum >/dev/null 2>&1; then
  archive_sha256="$(shasum -a 256 "$tmp_dir/$archive" | awk '{ print $1 }')"
else
  archive_sha256="$(sha256sum "$tmp_dir/$archive" | awk '{ print $1 }')"
fi

if [ "$archive_sha256" != "$manifest_sha256" ]; then
  echo "release archive does not match the signed manifest" >&2
  exit 1
fi

openssl pkeyutl -verify -rawin -pubin \
  -inkey "$tmp_dir/release-signing-public-key.pem" \
  -in "$tmp_dir/$archive" \
  -sigfile "$tmp_dir/$signature" >/dev/null

tar -xzf "$tmp_dir/$archive" -C "$tmp_dir"
mkdir -p "$install_dir"
cp "$tmp_dir/medotmd" "$install_dir/medotmd"
chmod +x "$install_dir/medotmd"

echo "installed medotmd to $install_dir/medotmd"
echo "run: medotmd init"
