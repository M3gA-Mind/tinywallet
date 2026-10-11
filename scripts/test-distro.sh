#!/usr/bin/env bash
set -euo pipefail

image="${1:?usage: test-distro.sh IMAGE --crate ...}"
shift
case "$image" in
  fedora:43@sha256:a651ddf48ea28a06ed4e1e6519f51c9f47e7a5a138722ade87369b8fbb7e5b42|fedora:44@sha256:43b29f65a41eb9c35e1cd5323e3bdf3b655c2357a9f4f1ff2f9c2798e5045d80|archlinux:base-devel@sha256:996c3a1d6b0d87b01242f6fcd8cfa3ad3eece1a67ab5c8f108e20af1d7b97bdd|menci/archlinuxarm:base-devel@sha256:e9caa68ff4162ca297ed6098375c04db6a825a58c0f0e446a7c90cb280310296) ;;
  *) echo "unsupported module build image: $image" >&2; exit 2 ;;
esac

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
crate="" version="" rust_target="" host="" output="release-assets" extras='[]'
while (($#)); do
  case "$1" in
    --crate) crate="$2"; shift 2 ;;
    --version) version="$2"; shift 2 ;;
    --target) rust_target="$2"; shift 2 ;;
    --host) host="$2"; shift 2 ;;
    --extra-files-json) extras="$2"; shift 2 ;;
    --output-dir) output="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
[[ "$crate" == tinywallet-module && -n "$version" && -n "$rust_target" && -n "$host" ]] || {
  echo "module crate, version, target, and host are required" >&2
  exit 2
}
[[ "$rust_target" == x86_64-unknown-linux-gnu || "$rust_target" == aarch64-unknown-linux-gnu ]] || {
  echo "unsupported distro target: $rust_target" >&2
  exit 2
}
[[ "$output" != /* && "$output" != .. && "$output" != ../* && "$output" != */../* && "$output" != */.. ]] || {
  echo "output directory must stay inside the repository" >&2
  exit 2
}
jq -e 'type == "array" and all(.[]; type == "string" and test("^docs/[A-Za-z0-9._/-]+$") and all(split("/"); . != "." and . != ".."))' <<<"$extras" >/dev/null || {
  echo "extra files must be repository-local docs paths without traversal" >&2
  exit 2
}

mkdir -p "$root/$output"
output_path="$(cd "$root/$output" && pwd -P)"
[[ "$output_path" == "$root"/* ]] || { echo "output directory must stay inside the repository" >&2; exit 2; }
docker run --rm \
  --volume "$root:/workspace" \
  --workdir /workspace \
  --env MODULE_CRATE="$crate" \
  --env MODULE_VERSION="$version" \
  --env MODULE_TARGET="$rust_target" \
  --env MODULE_HOST="$host" \
  --env EXTRA_PACKAGED_FILES="$extras" \
  --env HOST_UID="$(id -u)" --env HOST_GID="$(id -g)" \
  --env OUTPUT_DIR="${output_path#"$root"/}" \
  "$image" bash /workspace/scripts/build-module-in-distro.sh
