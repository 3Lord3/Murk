#!/usr/bin/env bash
# Verify that built packages actually require libmpv.
#
# Tauri's bundler does NOT run dpkg-shlibdeps or rpm's ELF dependency generator;
# it writes a fixed list plus whatever `bundle.linux.*.depends` contains. So the
# dependency has to be declared, and this script is what catches it being wrong.
# See src-tauri/PACKAGING.md.
#
# For rpm the requirement must be the *soname*: rpm derives `Provides:` from
# sonames, so `libmpv.so.2()(64bit)` is satisfied by whatever the distribution
# happens to call the package (`mpv-libs` on Fedora), and the package does not
# have to be re-edited when a name changes.
set -euo pipefail

bundle_dir="${1:-src-tauri/target/release/bundle}"
status=0

check() {
  local kind="$1" file="$2" deps
  case "$kind" in
    rpm) deps="$(rpm -qpR "$file" 2>/dev/null || true)" ;;
    deb) deps="$(dpkg -I "$file" 2>/dev/null | sed -n 's/^ Depends: //p' || true)" ;;
  esac

  printf '\n%s: %s\n' "$kind" "$(basename "$file")"
  if [ -z "$deps" ]; then
    echo "  !! no dependencies at all: the generator did not run" >&2
    status=1
    return
  fi

  case "$kind" in
    rpm)
      if grep -q "libmpv\.so\.2" <<<"$deps"; then
        echo "  ok  libmpv required by soname"
      else
        echo "  !! no libmpv.so.2 requirement; this rpm installs and then fails to start" >&2
        echo "     got: $deps" >&2
        status=1
      fi
      # Naming the package rather than the soname is the failure this whole
      # approach exists to avoid: it works on one distribution and not the other.
      if grep -Eq '(^|[[:space:]])(mpv-libs|libmpv2)([[:space:]]|$)' <<<"$deps"; then
        echo "  !! rpm names a distribution-specific package instead of a soname" >&2
        status=1
      fi
      ;;
    deb)
      # dpkg has no soname-level virtual packages, and deb targets only
      # Debian and Ubuntu, which agree on the name.
      if grep -q "libmpv2" <<<"$deps"; then
        echo "  ok  libmpv2 required"
      else
        echo "  !! no libmpv dependency; this deb installs and then fails to start" >&2
        echo "     got: $deps" >&2
        status=1
      fi
      ;;
  esac

  if grep -q "libepoxy" <<<"$deps"; then
    echo "  ok  libepoxy required (dlopen'd, so no ELF trace to derive it from)"
  else
    echo "  !! libepoxy is not required; Murk dlopens it and will fail at startup" >&2
    status=1
  fi
}

shopt -s nullglob
found=0
for f in "$bundle_dir"/rpm/*.rpm; do check rpm "$f"; found=1; done
for f in "$bundle_dir"/deb/*.deb; do check deb "$f"; found=1; done

if [ "$found" -eq 0 ]; then
  echo "no packages found under $bundle_dir, run 'pnpm tauri build' first" >&2
  exit 2
fi

exit $status
