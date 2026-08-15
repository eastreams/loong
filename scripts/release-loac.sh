#!/usr/bin/env bash
set -euo pipefail

die() {
    printf 'release-loac: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat >&2 <<'EOF'
usage:
  scripts/release-loac.sh check
  scripts/release-loac.sh verify-package <loac|loac-macros>
  scripts/release-loac.sh tag
  scripts/release-loac.sh version
  scripts/release-loac.sh tag-name
  scripts/release-loac.sh crate-status <crate>
EOF
    exit 2
}

repository_root=$(git rev-parse --show-toplevel)
cd "$repository_root"

release_values() {
    cargo metadata --locked --no-deps --format-version 1 |
        python3 -c '
import json
import sys

packages = {package["name"]: package for package in json.load(sys.stdin)["packages"]}
try:
    actor = packages["loac"]
    macros = packages["loac-macros"]
except KeyError as error:
    raise SystemExit(f"missing actor package: {error.args[0]}")

actor_version = actor["version"]
macros_version = macros["version"]
if actor_version != macros_version:
    raise SystemExit(
        f"loac ({actor_version}) and loac-macros ({macros_version}) must share a version"
    )

runtime_dependency = next(
    (
        dependency
        for dependency in actor["dependencies"]
        if dependency["name"] == "loac-macros" and dependency["kind"] is None
    ),
    None,
)
expected = "^" + macros_version
if runtime_dependency is None:
    raise SystemExit("loac must have a normal loac-macros dependency")
dependency_req = runtime_dependency["req"]
if dependency_req != expected:
    raise SystemExit(
        "loac normal loac-macros dependency must require "
        f"{expected}, got {dependency_req}"
    )
print(actor_version, "loac-v" + actor_version)
'
}

release_version() {
    local version tag
    read -r version tag <<<"$(release_values)"
    printf '%s\n' "$version"
}

release_tag() {
    local version tag
    read -r version tag <<<"$(release_values)"
    printf '%s\n' "$tag"
}

check_clean_checkout() {
    [[ "$(git branch --show-current)" == "rewrite" ]] ||
        die "release must run from the rewrite branch"
    [[ -z "$(git status --porcelain)" ]] ||
        die "release checkout is not clean"
}

check_tag_state() {
    local tag head tagged
    tag=$(release_tag)
    head=$(git rev-parse HEAD)
    if git show-ref --verify --quiet "refs/tags/$tag"; then
        tagged=$(git rev-list -n 1 "$tag")
        [[ "$tagged" == "$head" ]] ||
            die "local tag $tag points at $tagged, not $head"
    fi
}

check_release_link() {
    local tag
    tag=$(release_tag)
    grep -Fq "blob/$tag/crates/actor/examples/README.md" crates/actor/src/lib.rs ||
        die "crate docs must link the examples index through $tag"
}

check_release() {
    check_clean_checkout
    check_tag_state
    check_release_link
}

check_source() {
    check_release
    cargo fmt --all -- --check
    RUSTC_WRAPPER= cargo test -p loac --locked
    RUSTC_WRAPPER= cargo test -p loac-macros --locked
    cargo doc -p loac -p loac-macros --no-deps --locked
}

verify_package() (
    local package version archive temp package_dir
    package=${1:?package name is required}
    [[ "$package" == loac || "$package" == loac-macros ]] ||
        die "unknown package: $package"
    version=$(release_version)
    cargo package -p "$package" --locked
    archive="target/package/$package-$version.crate"
    [[ -f "$archive" ]] || die "cargo package did not create $archive"

    temp=$(mktemp -d "${TMPDIR:-/tmp}/loac-release.XXXXXX")
    trap 'rm -rf "$temp"' EXIT
    tar -xzf "$archive" -C "$temp"
    package_dir="$temp/$package-$version"
    [[ -f "$package_dir/Cargo.toml" ]] || die "invalid package archive: $archive"
    if [[ "$package" == "loac-macros" ]]; then
        # Macro UI tests already ran from the source checkout. The macro is
        # published before loac, so its archive must not require a new runtime
        # dev dependency that is not in the registry yet.
        CARGO_TARGET_DIR="$temp/target" RUSTC_WRAPPER= \
            cargo test --manifest-path "$package_dir/Cargo.toml" --lib --locked
    else
        CARGO_TARGET_DIR="$temp/target" RUSTC_WRAPPER= \
            cargo test --manifest-path "$package_dir/Cargo.toml" --locked
    fi
)

crate_status() (
    local package=${1:?crate name is required}
    local version archive local_checksum response_file http_code published_checksum
    [[ "$package" == loac || "$package" == loac-macros ]] ||
        die "unknown package: $package"
    version=$(release_version)
    archive="target/package/$package-$version.crate"
    [[ -f "$archive" ]] || die "package archive is missing: $archive"
    local_checksum=$(shasum -a 256 "$archive" | awk '{print $1}')

    response_file=$(mktemp "${TMPDIR:-/tmp}/loac-registry.XXXXXX")
    trap 'rm -f "$response_file"' EXIT
    http_code=$(curl --silent --show-error --location --retry 3 \
        --output "$response_file" --write-out '%{http_code}' \
        "https://crates.io/api/v1/crates/$package/$version") ||
        die "could not query crates.io for $package $version"
    case "$http_code" in
        404)
            printf 'absent\n'
            ;;
        200)
            published_checksum=$(python3 -c '
import json
import sys

data = json.load(sys.stdin)
try:
    print(data["version"]["checksum"])
except (KeyError, TypeError):
    raise SystemExit("crates.io response has no version checksum")
' <"$response_file")
            if [[ "$local_checksum" == "$published_checksum" ]]; then
                printf 'match\n'
            else
                printf 'mismatch\n'
            fi
            ;;
        *)
            die "crates.io returned HTTP $http_code for $package $version"
            ;;
    esac
)

remote_tag_commit() {
    local tag=${1:?tag name is required}
    local refs commit
    if ! refs=$(git ls-remote --tags origin "refs/tags/$tag" "refs/tags/$tag^{}"); then
        die "could not query remote tag: $tag"
    fi
    [[ -n "$refs" ]] || return 1
    commit=$(awk -v peeled="refs/tags/$tag^{}" '$2 == peeled { print $1; exit }' <<<"$refs")
    if [[ -z "$commit" ]]; then
        commit=$(awk -v ref="refs/tags/$tag" '$2 == ref { print $1; exit }' <<<"$refs")
    fi
    [[ -n "$commit" ]] || die "remote tag has no readable target: $tag"
    printf '%s\n' "$commit"
}

create_tag() {
    local tag head remote_commit
    tag=$(release_tag)
    head=$(git rev-parse HEAD)
    if remote_commit=$(remote_tag_commit "$tag"); then
        :
    else
        remote_commit=
    fi
    if git show-ref --verify --quiet "refs/tags/$tag"; then
        [[ "$(git rev-list -n 1 "$tag")" == "$head" ]] ||
            die "local tag $tag does not point at HEAD"
        if [[ -z "$remote_commit" ]]; then
            git push origin "refs/tags/$tag"
        elif [[ "$remote_commit" != "$head" ]]; then
            die "remote tag $tag does not point at HEAD"
        fi
        return
    fi
    if [[ -n "$remote_commit" ]]; then
        [[ "$remote_commit" == "$head" ]] ||
            die "remote tag $tag points at $remote_commit, not HEAD"
        git fetch origin "refs/tags/$tag:refs/tags/$tag"
        return
    fi
    git tag -a "$tag" -m "Release $tag" "$head"
    git push origin "refs/tags/$tag"
}

case "${1:-}" in
    check)
        check_source
        ;;
    verify-package)
        verify_package "${2:-}"
        ;;
    tag)
        check_release
        create_tag
        ;;
    version)
        release_version
        ;;
    tag-name)
        release_tag
        ;;
    crate-status)
        crate_status "${2:-}"
        ;;
    *)
        usage
        ;;
esac
