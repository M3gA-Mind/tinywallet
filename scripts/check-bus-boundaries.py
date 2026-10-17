#!/usr/bin/env python3
"""Audit the bus's normal/build closure using resolved Cargo package identities."""
import json
import subprocess
import sys

ALLOWED = {
    "serde", "serde_core", "serde_derive", "serde_json", "thiserror",
    "thiserror-impl", "proc-macro2", "quote", "syn", "unicode-ident",
    "itoa", "memchr", "zmij",
}
REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"

# The `storage` feature (OFF by default) is the one sanctioned way anything
# beyond ALLOWED enters the closure. It brings `tinystoragedrivers-core`, the
# storage ports crate, and with it that crate's own registry dependencies
# (tokio, zstd, ...). The exemption is deliberately narrow:
#   * only when `storage` is enabled; the default closure is audited strictly
#     and must not contain any of it,
#   * only that git crate, pinned to the tinystoragedrivers repository, plus
#     registry crates; a path or other git crate (any implementation) is
#     still a violation.
STORAGE_FEATURE = "storage"
STORAGE_EXEMPT_GIT = {"tinystoragedrivers-core": "https://github.com/tinyhumansai/tinystoragedrivers"}

# Chain, key and crypto implementations never enter the bus closure, not even
# through the `storage` feature's registry-crate exemption.
FORBIDDEN_NAMES = {"bitcoin", "k256"}
FORBIDDEN_PREFIXES = ("coins-",)


def forbidden(name):
    """Whether `name` is a chain/key implementation the bus must never link."""
    return name in FORBIDDEN_NAMES or name.startswith(FORBIDDEN_PREFIXES)


def audit(metadata):
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    roots = [package["id"] for package in metadata["packages"] if package["name"] == "tinywallet-bus"]
    if len(roots) != 1:
        raise ValueError("expected exactly one tinywallet-bus package")
    queue = [(roots[0], ["tinywallet-bus"])]
    seen = set()
    violations = []
    while queue:
        identity, path = queue.pop()
        if identity in seen:
            continue
        seen.add(identity)
        package = packages[identity]
        if len(path) > 1 and (package["name"] not in ALLOWED or package.get("source") != REGISTRY):
            violations.append(" -> ".join(path))
        for dependency in nodes[identity]["deps"]:
            if any(kind["kind"] in (None, "build") for kind in dependency["dep_kinds"]):
                child = dependency["pkg"]
                queue.append((child, path + [packages[child]["name"]]))
    return violations


def tree_packages(feature=None):
    """(name, source) of every normal/build dependency of the bus alone.

    `cargo tree -p` resolves features for the selected package only, so a
    workspace sibling that enables `storage` cannot leak into the default run
    the way it does in `cargo metadata`.
    """
    command = ["cargo", "tree", "-p", "tinywallet-bus", "-e", "normal,build", "--prefix", "none", "-f", "{p}"]
    if feature:
        command += ["--features", feature]
    out = subprocess.run(command, check=True, capture_output=True, text=True).stdout
    packages = []
    for line in out.splitlines():
        parts = line.split()
        if len(parts) < 2 or parts[0] == "tinywallet-bus":
            continue
        # Markers such as `(proc-macro)` and `(*)` are not sources; a path or
        # git source is the parenthesised URL or absolute path.
        source = next((part.strip("()") for part in parts[2:] if part.startswith("(") and part not in ("(proc-macro)", "(*)", "(build)")), "")
        packages.append((parts[0], source))
    return packages


def audit_tree(packages, storage=False):
    """Violations for a `cargo tree` listing; `storage` applies the exemption."""
    violations = []
    for name, source in packages:
        if forbidden(name):
            violations.append(f"tinywallet-bus -> {name} (forbidden)")
            continue
        if name in ALLOWED and not source:
            continue
        if storage and not source:
            continue  # a registry crate reached through the storage feature
        if storage and name in STORAGE_EXEMPT_GIT and source.startswith(STORAGE_EXEMPT_GIT[name]):
            continue
        violations.append(f"tinywallet-bus -> {name} ({source or 'registry'})")
    return violations


if __name__ == "__main__":
    failures = audit_tree(tree_packages())
    if failures:
        print("Forbidden contract dependencies (default features):\n" + "\n".join(failures), file=sys.stderr)
        sys.exit(1)
    failures = audit_tree(tree_packages(STORAGE_FEATURE), storage=True)
    if failures:
        print("Forbidden contract dependencies (storage feature):\n" + "\n".join(failures), file=sys.stderr)
        sys.exit(1)
    print("tinywallet-bus: default closure is pure; the storage feature adds only the storage ports crate and registry crates")
