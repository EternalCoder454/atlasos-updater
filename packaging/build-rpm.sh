#!/bin/bash
# Build the Atlas RPMs inside a fedora:44 container, as root.
#   packaging/build-rpm.sh <out dir> [rpmbuild options, e.g. --without app]
# The binary RPMs (no source, no debuginfo) are copied to <out dir>.
# Cargo needs network access.
set -euo pipefail

main() {
    out=${1:?usage: build-rpm.sh <out dir> [rpmbuild options]}
    shift
    rpmopts=("$@")
    
    here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
    src=$(dirname "$here")
    spec=$here/atlas.spec
    version=$(awk '/^Version:/ {print $2; exit}' "$spec")
    
    dnf -y install rpm-build dnf5-plugins tar gzip >&2
    # builddep reads the spec; pass --without/--with through as rpm macros
    defs=()
    for ((i = 0; i < ${#rpmopts[@]}; i++)); do
        case ${rpmopts[i]} in
            --without) defs+=(--define "_without_${rpmopts[i + 1]} 1") ;;
            --with) defs+=(--define "_with_${rpmopts[i + 1]} 1") ;;
        esac
    done
    dnf -y builddep "${defs[@]}" "$spec" >&2
    
    top=$(mktemp -d)
    trap 'rm -rf "$top"' EXIT
    mkdir -p "$top"/{SOURCES,BUILD,RPMS,SRPMS,SPECS}
    tar -C "$src" \
        --exclude=./.git --exclude=./target --exclude=./out --exclude=./build \
        --transform "s,^\./,atlas-$version/," \
        -czf "$top/SOURCES/atlas-$version.tar.gz" .
    
    rpmbuild -bb "${rpmopts[@]}" --define "_topdir $top" "$spec"
    
    mkdir -p "$out"
    find "$top/RPMS" -name '*.rpm' ! -name '*.src.rpm' ! -name '*debuginfo*' ! -name '*debugsource*' \
        -exec cp -v {} "$out"/ \;
}

main "$@"
exit $?
