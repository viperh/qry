#!/usr/bin/env python3
"""Crate layering check (T003).

Reads `cargo metadata` and fails when a workspace crate breaks the dependency
direction in the README: `qry-core` must never depend on `ratatui`,
`crossterm` or `clap` (nor on the other UI-side crates in RULES), and nothing
may depend on the `qry` binary.

Only *normal* dependencies count (dev and build dependencies are ignored), on
every target platform. The graph is checked twice, with `--all-features` and
with `--no-default-features`. This is a second, independent implementation of
`crates/qry/tests/workspace_metadata.rs`; `deny.toml` bans the same crates.

Only the Python standard library is used.

Usage:
    python3 scripts/check-layering.py [--manifest-path Cargo.toml]
    python3 scripts/check-layering.py --self-test

Exit code 0 = clean, 1 = violations (or a failed self-test), 2 = could not
run cargo metadata.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from collections import deque
from typing import NamedTuple

# Workspace crate -> crates it must never reach through normal dependencies.
# Every workspace crate needs an entry, so a new crate gets a decision.
RULES: dict[str, set[str]] = {
    "qry-core": {"ratatui", "crossterm", "clap", "ratatui-textarea", "keyring"},
    "qry": set(),
}

# The binary: no other workspace crate may depend on it, in any kind.
BINARY = "qry"


class Violation(NamedTuple):
    krate: str
    forbidden: str
    path: str  # "qry-core -> foo -> crossterm"

    def __str__(self) -> str:
        hops = self.path.split(" -> ")[1:-1]
        via = f"via {' -> '.join(hops)}" if hops else "direct"
        return f"{self.krate} -> {self.forbidden} ({via})"


def metadata(manifest: str | None, features: str) -> dict:
    cmd = ["cargo", "metadata", "--format-version", "1", "--locked", features]
    if manifest:
        cmd += ["--manifest-path", manifest]
    try:
        out = subprocess.run(cmd, check=True, capture_output=True, text=True)
    except (OSError, subprocess.CalledProcessError) as err:
        stderr = getattr(err, "stderr", "") or ""
        print(f"error: `{' '.join(cmd)}` failed: {err}\n{stderr}", file=sys.stderr)
        sys.exit(2)
    return json.loads(out.stdout)


def normal_graph(meta: dict) -> tuple[dict[str, str], dict[str, list[str]]]:
    """Return (package id -> name, package id -> ids of normal deps)."""
    names = {p["id"]: p["name"] for p in meta["packages"]}
    edges: dict[str, list[str]] = {}
    for node in meta["resolve"]["nodes"]:
        deps = []
        for dep in node.get("deps", []):
            kinds = dep.get("dep_kinds") or [{"kind": None}]
            if any(k.get("kind") is None for k in kinds):
                deps.append(dep["pkg"])
        edges[node["id"]] = sorted(deps, key=lambda i: (names[i], i))
    return names, edges


def shortest_paths(root: str, edges: dict[str, list[str]]) -> dict[str, list[str]]:
    """Every package reachable from `root` -> the shortest id path to it."""
    paths = {root: [root]}
    queue = deque([root])
    while queue:
        cur = queue.popleft()
        for nxt in edges.get(cur, ()):
            if nxt not in paths:
                paths[nxt] = paths[cur] + [nxt]
                queue.append(nxt)
    del paths[root]
    return paths


def check(meta: dict) -> tuple[list[Violation], list[str]]:
    """Return (forbidden-dependency violations, other errors)."""
    violations: list[Violation] = []
    errors: list[str] = []
    names, edges = normal_graph(meta)
    members = {pid: names[pid] for pid in meta["workspace_members"]}

    for pid, name in sorted(members.items(), key=lambda kv: kv[1]):
        if name not in RULES:
            errors.append(
                f"{name}: workspace crate has no layering rule; "
                "add it to RULES in scripts/check-layering.py"
            )
            continue
        for dep_id, path in shortest_paths(pid, edges).items():
            if names[dep_id] in RULES[name]:
                chain = " -> ".join(names[i] for i in path)
                violations.append(Violation(name, names[dep_id], chain))

    # Nothing depends on the binary (its own self dev-dependency is fine).
    for pkg in meta["packages"]:
        if pkg["id"] not in members or pkg["name"] == BINARY:
            continue
        for dep in pkg.get("dependencies", []):
            if dep["name"] == BINARY:
                kind = dep.get("kind") or "normal"
                errors.append(f"{pkg['name']} depends on the {BINARY} binary ({kind})")

    return sorted(set(violations)), errors


# --- self-test ---------------------------------------------------------------


def synthetic(edges: dict[str, list[str]]) -> dict:
    """A minimal `cargo metadata` document with the given normal edges."""
    names = set(edges) | {d for deps in edges.values() for d in deps}
    ident = {n: f"path+file:///synthetic/{n}#0.1.0" for n in names}
    return {
        "packages": [
            {"id": ident[n], "name": n, "dependencies": []} for n in sorted(names)
        ],
        "workspace_members": [ident["qry"], ident["qry-core"]],
        "resolve": {
            "nodes": [
                {
                    "id": ident[n],
                    "deps": [
                        {"pkg": ident[d], "dep_kinds": [{"kind": None, "target": None}]}
                        for d in edges.get(n, [])
                    ],
                }
                for n in sorted(names)
            ]
        },
    }


def self_test() -> int:
    failures: list[str] = []

    planted = synthetic(
        {
            "qry": ["qry-core", "ratatui"],
            "qry-core": ["foo", "serde"],
            "foo": ["crossterm"],
            "ratatui": ["crossterm"],
        }
    )
    # Dev and build edges never count: this one must not be reported.
    for node in planted["resolve"]["nodes"]:
        if node["id"].endswith("/qry-core#0.1.0"):
            node["deps"].append(
                {
                    "pkg": "path+file:///synthetic/ratatui#0.1.0",
                    "dep_kinds": [{"kind": "dev", "target": None}],
                }
            )
    violations, errors = check(planted)
    if errors:
        failures.append(f"planted graph: unexpected errors {errors}")
    if [v.path for v in violations] != ["qry-core -> foo -> crossterm"]:
        failures.append(f"planted graph: expected one violation, got {violations}")
    elif str(violations[0]) != "qry-core -> crossterm (via foo)":
        failures.append(f"planted graph: bad message {str(violations[0])!r}")

    clean = synthetic(
        {"qry": ["qry-core", "ratatui", "clap"], "qry-core": ["serde", "tokio"]}
    )
    violations, errors = check(clean)
    if violations or errors:
        failures.append(f"clean graph: expected nothing, got {violations} {errors}")

    on_binary = synthetic({"qry": ["qry-core"], "qry-core": []})
    on_binary["packages"][[p["name"] for p in on_binary["packages"]].index("qry-core")][
        "dependencies"
    ].append({"name": "qry", "kind": "dev"})
    _, errors = check(on_binary)
    if errors != ["qry-core depends on the qry binary (dev)"]:
        failures.append(f"dependency on the binary: got {errors}")

    unknown = synthetic({"qry": ["qry-core"], "qry-core": []})
    unknown["workspace_members"].append("path+file:///synthetic/qry-new#0.1.0")
    unknown["packages"].append(
        {"id": "path+file:///synthetic/qry-new#0.1.0", "name": "qry-new", "dependencies": []}
    )
    _, errors = check(unknown)
    if len(errors) != 1 or "qry-new" not in errors[0]:
        failures.append(f"crate without a rule: got {errors}")

    if failures:
        print("check-layering self-test FAILED:", file=sys.stderr)
        for failure in failures:
            print(f"  - {failure}", file=sys.stderr)
        return 1
    print("check-layering self-test OK")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--manifest-path", default=None)
    parser.add_argument(
        "--self-test", action="store_true", help="check the checker on synthetic graphs"
    )
    args = parser.parse_args()
    if args.self_test:
        return self_test()

    lines: list[str] = []
    for features in ("--all-features", "--no-default-features"):
        violations, errors = check(metadata(args.manifest_path, features))
        lines += [f"[{features[2:]}] {v}" for v in violations]
        lines += [f"[{features[2:]}] {e}" for e in errors]

    if lines:
        print("crate layering violations:", file=sys.stderr)
        for line in lines:
            print(f"  {line}", file=sys.stderr)
        return 1
    print("crate layering OK (all-features and no-default-features graphs)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
