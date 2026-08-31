#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [[ "${1:-}" != "--confirmed-manual" ]]; then
  cat >&2 <<'EOF'
Packaging is intentionally gated behind manual testing.

Run, in order:
  ./scripts/test-no-wayland.sh check
  ./scripts/test-no-wayland.sh smoke
  ./scripts/test-no-wayland.sh manual

After completing docs/TESTING.md, package with:
  ./scripts/package-no-wayland.sh --confirmed-manual
EOF
  exit 2
fi

if [[ -e "$ROOT/.git" ]]; then
  project_git_root="$(git rev-parse --show-toplevel)"
  if [[ "$(realpath "$project_git_root")" != "$(realpath "$ROOT")" ]]; then
    echo "Project .git metadata does not resolve to the project root." >&2
    exit 2
  fi
  if [[ -n "$(git status --porcelain)" ]]; then
    echo "Refusing to package a dirty project working tree." >&2
    git status --short >&2
    exit 2
  fi
else
  echo "Project Git metadata not present; treating this as a source archive."
  echo "An unrelated parent Git repository is intentionally ignored."
  echo "Verify the source archive's published SHA-256 before packaging."
fi
for command in cargo dpkg-deb python3; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "Missing required command: $command" >&2
    exit 127
  fi
done
if ! cargo deb --version >/dev/null 2>&1; then
  echo "cargo-deb is required. Install it once with: cargo install cargo-deb --locked" >&2
  exit 127
fi

PYTHONDONTWRITEBYTECODE=1 python3 scripts/check_release_docs.py
./scripts/test-no-wayland.sh check
mkdir -p target/debian
rm -f target/debian/*.deb target/debian/*.deb.sha256 target/debian/*.deb.contents.txt
cargo deb --locked --no-default-features --variant no-wayland

package="$(find target/debian -maxdepth 1 -type f -name '*.deb' -printf '%T@ %p\n' \
  | sort -nr | head -n1 | cut -d' ' -f2-)"
if [[ -z "$package" || ! -f "$package" ]]; then
  echo "cargo-deb completed without a .deb output." >&2
  exit 1
fi

dpkg-deb --info "$package"
package_name="$(dpkg-deb -f "$package" Package)"
depends="$(dpkg-deb -f "$package" Depends)"
conflicts="$(dpkg-deb -f "$package" Conflicts)"
replaces="$(dpkg-deb -f "$package" Replaces)"
[[ "$package_name" == "topmonitoring" ]] || {
  echo "Unexpected Debian package name: $package_name" >&2
  exit 1
}
if grep -q 'libgtk4-layer-shell' <<<"$depends"; then
  echo "The no-Wayland package unexpectedly depends on layer shell." >&2
  exit 1
fi
for relationship in "$conflicts" "$replaces"; do
  if ! grep -Eq '(^|,)[[:space:]]*topmonitoring-no-wayland([[:space:](,]|$)' <<<"$relationship"; then
    echo "Missing topmonitoring-no-wayland migration relationship." >&2
    exit 1
  fi
done
echo "Debian migration and no-Wayland dependency metadata: PASS"
dpkg-deb --contents "$package" | tee "${package}.contents.txt"
grep -q 'usr/bin/topmonitoring' "${package}.contents.txt"
grep -q 'io.github.fakedevbagus.TopMonitoring.desktop' "${package}.contents.txt"
grep -q 'io.github.fakedevbagus.TopMonitoring.metainfo.xml' "${package}.contents.txt"
if command -v lintian >/dev/null 2>&1; then
  lintian "$package"
else
  echo "lintian not installed; skipping optional Debian policy lint."
fi
sha256sum "$package" | tee "${package}.sha256"
echo "No-Wayland package ready: $package"
