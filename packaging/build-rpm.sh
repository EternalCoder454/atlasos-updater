#!/bin/bash
# Build the Telamon Updater RPMs inside a fedora:44 container, as root.
#   packaging/build-rpm.sh <out dir> [rpmbuild options, e.g. --without app]
# The binary RPMs (no source, no debuginfo) are copied to <out dir>.
# Cargo needs network access.
# TELAMON_LOCAL_RPMS=<dir> installs the RPMs in <dir> first: the Telamon
# framework's (telamon-ui), which the app builds against and no repository has.
# TELAMON_BUILD_CACHE=<dir> (optional, such as a podman cache mount) keeps cargo's
# downloads, cargo's output and the CMake build in <dir>, and builds in a fixed
# place, so the next build only recompiles what changed.
# The names these variables had until 0.3.0, ATLAS_LOCAL_RPMS and
# ATLAS_BUILD_CACHE, still work when the new ones are not set.
set -euo pipefail

main() {
    out=${1:?usage: build-rpm.sh <out dir> [rpmbuild options]}
    shift
    rpmopts=("$@")
    
    here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
    src=$(dirname "$here")
    spec=$here/telamon-updater.spec
    version=$(awk '/^Version:/ {print $2; exit}' "$spec")
    
    local_dir=${TELAMON_LOCAL_RPMS:-${ATLAS_LOCAL_RPMS:-}}
    cache=${TELAMON_BUILD_CACHE:-${ATLAS_BUILD_CACHE:-}}

    dnf -y install rpm-build dnf5-plugins tar gzip >&2
    if [ -n "$local_dir" ]; then
        # Telamon.Ui and its fonts, not the gallery. dnf brings their
        # dependencies; rpm then puts these exact files in place even when a
        # build of the same version is installed already.
        local_rpms=("$local_dir"/telamon-ui-[0-9]*.rpm "$local_dir"/telamon-symbols-fonts-[0-9]*.rpm)
        dnf -y install "${local_rpms[@]}" >&2
        rpm -U --replacepkgs --replacefiles "${local_rpms[@]}" >&2
    fi
    # builddep reads the spec; pass --without/--with through as rpm macros
    defs=()
    for ((i = 0; i < ${#rpmopts[@]}; i++)); do
        case ${rpmopts[i]} in
            --without) defs+=(--define "_without_${rpmopts[i + 1]} 1") ;;
            --with) defs+=(--define "_with_${rpmopts[i + 1]} 1") ;;
        esac
    done
    dnf -y builddep "${defs[@]}" "$spec" >&2
    
    if [ -n "$cache" ]; then
        mkdir -p "$cache"
        cache=$(cd "$cache" && pwd)
        case $cache/ in
            "$src"/*) echo "TELAMON_BUILD_CACHE must be outside the source tree" >&2; exit 1 ;;
        esac
        # Cargo's fingerprints include absolute paths, so the build tree must
        # sit at the same path every time.
        top=$cache/rpmbuild
        rm -rf "$top"
        # Output built with another compiler or Qt can't be trusted (build
        # scripts don't track system headers): start over when they change.
        # (rpm -q fails for a package --without app doesn't install.)
        toolchain=$(rustc -vV
            rpm -q rust cargo gcc-c++ cmake corrosion qt6-qtbase-devel qt6-qtdeclarative-devel || true)
        if [ "$(cat "$cache/toolchain" 2>/dev/null)" != "$toolchain" ]; then
            rm -rf "$cache/target" "$cache/cmake"
            printf '%s\n' "$toolchain" >"$cache/toolchain"
        fi
        # The sources unpack to a directory named after the version, and
        # CMake refuses a build tree made for another source directory.
        if [ "$(cat "$cache/version" 2>/dev/null)" != "$version" ]; then
            rm -rf "$cache/cmake"
            printf '%s\n' "$version" >"$cache/version"
        fi
        # The app's QML is compiled against the installed Telamon.Ui (its
        # qmltypes and .qml files), which CMake doesn't track: rebuild the
        # CMake side, not cargo's, whenever that changes. (--without app
        # builds don't install it: cat fails, which pipefail mustn't see.)
        telamonui=$({ cat /usr/lib64/qt6/qml/Telamon/Ui/* 2>/dev/null || true; } | sha256sum)
        if [ "$(cat "$cache/telamon-ui" 2>/dev/null)" != "$telamonui" ]; then
            rm -rf "$cache/cmake"
            printf '%s\n' "$telamonui" >"$cache/telamon-ui"
        fi
        rpmopts+=(--define "_telamon_build_cache $cache")
    else
        top=$(mktemp -d)
    fi
    trap 'rm -rf "$top"' EXIT
    mkdir -p "$top"/{SOURCES,BUILD,RPMS,SRPMS,SPECS}
    tar -C "$src" \
        --exclude=./.git --exclude=./target --exclude=./out --exclude=./build \
        --transform "s,^\./,telamon-updater-$version/," \
        -czf "$top/SOURCES/telamon-updater-$version.tar.gz" .
    
    rpmbuild -bb "${rpmopts[@]}" --define "_topdir $top" "$spec"
    
    mkdir -p "$out"
    find "$top/RPMS" -name '*.rpm' ! -name '*.src.rpm' ! -name '*debuginfo*' ! -name '*debugsource*' \
        -exec cp -v {} "$out"/ \;
}

main "$@"
exit $?
