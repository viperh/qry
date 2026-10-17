#!/usr/bin/env python3
"""Performance gates and regression checks over criterion's output (T006).

Stdlib only. Criterion writes, for every benchmark and saved baseline,

    target/criterion/<group>/<id>/<baseline>/estimates.json
    target/criterion/<group>/<id>/<baseline>/benchmark.json

(a benchmark without a group has one directory level fewer). The median is
`estimates["median"]["point_estimate"]`, in nanoseconds; a throughput, when
the bench declares one, is in `benchmark.json` as {"Bytes": n}.

    bench-gate.py gate --baseline ci [--gates scripts/bench-gates.toml]
        Checks every [[gate]] in the gates file against that baseline:
        `ci_max_ms` (the median must not exceed it) or `ci_min_mb_s` (the
        throughput at the median must reach it). Exits 1 on any breach,
        and for a gate whose bench has no estimates, so renaming a bench
        cannot silently switch its gate off.

    bench-gate.py compare --old main --new ci [--threshold 15]
        Compares every bench present in both baselines and exits 1 if any
        median grew by more than the threshold, in percent.

    bench-gate.py self-test
        Runs both modes against synthetic trees in a temporary directory
        and checks their exit codes.

`--criterion DIR` (default target/criterion, or $CARGO_TARGET_DIR/criterion)
points at another tree.
"""

from __future__ import annotations

import argparse
import contextlib
import io
import json
import os
import sys
import tempfile
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_GATES = ROOT / "scripts" / "bench-gates.toml"


def default_criterion_dir() -> Path:
    target = os.environ.get("CARGO_TARGET_DIR")
    return (Path(target) if target else ROOT / "target") / "criterion"


def bench_dir(criterion: Path, bench: str) -> Path:
    return criterion.joinpath(*bench.split("/"))


def median_ns(criterion: Path, bench: str, baseline: str) -> float | None:
    path = bench_dir(criterion, bench) / baseline / "estimates.json"
    try:
        estimates = json.loads(path.read_text())
    except (OSError, ValueError):
        return None
    try:
        return float(estimates["median"]["point_estimate"])
    except (KeyError, TypeError, ValueError):
        return None


def throughput_bytes(criterion: Path, bench: str, baseline: str) -> int | None:
    path = bench_dir(criterion, bench) / baseline / "benchmark.json"
    try:
        throughput = json.loads(path.read_text()).get("throughput") or {}
    except (OSError, ValueError, AttributeError):
        return None
    value = throughput.get("Bytes") or throughput.get("BytesDecimal")
    return int(value) if value else None


def find_benches(criterion: Path, baseline: str) -> set[str]:
    """Every bench (as `group/id` or `id`) that has `baseline` saved."""
    found = set()
    if not criterion.is_dir():
        return found
    for estimates in criterion.rglob(f"{baseline}/estimates.json"):
        bench = estimates.parent.parent.relative_to(criterion)
        if bench.parts and bench.parts[0] != "report":
            found.add("/".join(bench.parts))
    return found


def fmt_ms(ns: float) -> str:
    return f"{ns / 1e6:.3f} ms"


def cmd_gate(criterion: Path, gates_file: Path, baseline: str) -> int:
    try:
        gates = tomllib.loads(gates_file.read_text()).get("gate", [])
    except (OSError, tomllib.TOMLDecodeError) as e:
        print(f"cannot read {gates_file}: {e}")
        return 1
    if not gates:
        print(f"{gates_file} has no [[gate]] entries")
        return 1

    failed = False
    rows = []
    for gate in gates:
        bench = gate.get("bench", "?")
        median = median_ns(criterion, bench, baseline)
        if median is None:
            rows.append((bench, "-", "-", f"FAIL: no estimates for {bench}"))
            failed = True
            continue
        if "ci_max_ms" in gate:
            limit = float(gate["ci_max_ms"])
            ok = median / 1e6 <= limit
            rows.append((bench, fmt_ms(median), f"<= {limit} ms", "ok" if ok else "FAIL"))
        elif "ci_min_mb_s" in gate:
            limit = float(gate["ci_min_mb_s"])
            nbytes = throughput_bytes(criterion, bench, baseline)
            if not nbytes:
                rows.append((bench, fmt_ms(median), f">= {limit} MB/s",
                             f"FAIL: {bench} declares no byte throughput"))
                failed = True
                continue
            mb_s = nbytes / (median / 1e9) / 1e6
            ok = mb_s >= limit
            rows.append((bench, f"{mb_s:.1f} MB/s", f">= {limit} MB/s", "ok" if ok else "FAIL"))
        else:
            rows.append((bench, fmt_ms(median), "-", "FAIL: gate has no ci_max_ms or ci_min_mb_s"))
            ok = False
        failed |= not ok

    print_table(("bench", "median", "ci limit", "result"), rows)
    if failed:
        print(f"\nPerformance gate FAILED for baseline `{baseline}`.")
        return 1
    print(f"\nEvery gate passed for baseline `{baseline}`.")
    return 0


def cmd_compare(criterion: Path, old: str, new: str, threshold: float) -> int:
    common = sorted(find_benches(criterion, old) & find_benches(criterion, new))
    if not common:
        print(f"No bench has both `{old}` and `{new}` baselines; nothing to compare.")
        return 0
    failed = False
    rows = []
    for bench in common:
        before = median_ns(criterion, bench, old)
        after = median_ns(criterion, bench, new)
        if not before or after is None:
            rows.append((bench, "-", "-", "-", "skipped: unreadable estimates"))
            continue
        change = (after - before) / before * 100
        regressed = change > threshold
        failed |= regressed
        rows.append((bench, fmt_ms(before), fmt_ms(after), f"{change:+.1f}%",
                     "REGRESSED" if regressed else "ok"))
    print_table(("bench", old, new, "change", "result"), rows)
    if failed:
        print(f"\nA median regressed by more than {threshold}%.")
        return 1
    print(f"\nNo median regressed by more than {threshold}%.")
    return 0


def print_table(header: tuple[str, ...], rows: list[tuple[str, ...]]) -> None:
    widths = [max(len(str(r[i])) for r in [header, *rows]) for i in range(len(header))]
    line = lambda r: "  ".join(str(c).ljust(w) for c, w in zip(r, widths)).rstrip()
    print(line(header))
    print("  ".join("-" * w for w in widths))
    for row in rows:
        print(line(row))


# --------------------------------------------------------------------------
# self-test


def write_estimate(criterion: Path, bench: str, baseline: str, ms: float,
                   nbytes: int | None = None) -> None:
    d = bench_dir(criterion, bench) / baseline
    d.mkdir(parents=True, exist_ok=True)
    ns = ms * 1e6
    est = {k: {"point_estimate": ns, "standard_error": 0.0,
               "confidence_interval": {"confidence_level": 0.95,
                                       "lower_bound": ns, "upper_bound": ns}}
           for k in ("mean", "median", "median_abs_dev", "slope", "std_dev")}
    (d / "estimates.json").write_text(json.dumps(est))
    group, _, function = bench.rpartition("/")
    meta = {"group_id": group or bench, "function_id": function if group else None,
            "throughput": {"Bytes": nbytes} if nbytes else None}
    (d / "benchmark.json").write_text(json.dumps(meta))


def run_quiet(fn, *args) -> tuple[int, str]:
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        code = fn(*args)
    return code, out.getvalue()


def self_test() -> int:
    failures = []

    def check(name: str, got: int, want: int, output: str, needle: str | None = None):
        if got != want:
            failures.append(f"{name}: exit {got}, expected {want}\n{output}")
        elif needle is not None and needle not in output:
            failures.append(f"{name}: output lacks {needle!r}\n{output}")
        else:
            print(f"ok  {name}")

    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        crit = tmp / "criterion"
        gates = tmp / "gates.toml"

        def gate_file(text: str) -> Path:
            gates.write_text(text)
            return gates

        # gate pass: 1.0 ms against 4.
        write_estimate(crit, "pass_group/fast", "ci", 1.0)
        code, out = run_quiet(cmd_gate, crit, gate_file(
            '[[gate]]\nbench = "pass_group/fast"\nmax_ms = 2\nci_max_ms = 4\n'), "ci")
        check("gate pass", code, 0, out)

        # gate breach: 5.0 ms against 4, and the bench is named.
        write_estimate(crit, "breach_group/slow", "ci", 5.0)
        code, out = run_quiet(cmd_gate, crit, gate_file(
            '[[gate]]\nbench = "breach_group/slow"\nmax_ms = 2\nci_max_ms = 4\n'), "ci")
        check("gate breach", code, 1, out, "breach_group/slow")

        # throughput gate: 1 MB in 50 ms is 20 MB/s, against 25.
        write_estimate(crit, "lex/1MB", "ci", 50.0, nbytes=1_000_000)
        code, out = run_quiet(cmd_gate, crit, gate_file(
            '[[gate]]\nbench = "lex/1MB"\nmin_mb_s = 50\nci_min_mb_s = 25\n'), "ci")
        check("throughput gate breach", code, 1, out, "20.0 MB/s")
        code, out = run_quiet(cmd_gate, crit, gate_file(
            '[[gate]]\nbench = "lex/1MB"\nmin_mb_s = 30\nci_min_mb_s = 15\n'), "ci")
        check("throughput gate pass", code, 0, out)

        # missing bench.
        code, out = run_quiet(cmd_gate, crit, gate_file(
            '[[gate]]\nbench = "gone/renamed"\nmax_ms = 1\nci_max_ms = 2\n'), "ci")
        check("missing bench", code, 1, out, "no estimates for gone/renamed")

        # an ungrouped bench id has one level fewer.
        write_estimate(crit, "single_id", "ci", 1.0)
        code, out = run_quiet(cmd_gate, crit, gate_file(
            '[[gate]]\nbench = "single_id"\nmax_ms = 1\nci_max_ms = 2\n'), "ci")
        check("ungrouped bench", code, 0, out)

        # compare: old 10 ms, new 12 ms (+20%).
        cmp = tmp / "compare"
        write_estimate(cmp, "render/100k", "main", 10.0)
        write_estimate(cmp, "render/100k", "ci", 12.0)
        write_estimate(cmp, "only_in_ci/x", "ci", 1.0)
        code, out = run_quiet(cmd_compare, cmp, "main", "ci", 25.0)
        check("compare within threshold", code, 0, out)
        code, out = run_quiet(cmd_compare, cmp, "main", "ci", 10.0)
        check("compare regression", code, 1, out, "render/100k")
        # T006 lists "10 -> 12 ms, threshold 15 -> exit 0", but 12 against
        # 10 is +20%, which a 15% threshold must fail. The intent (a change
        # under the threshold passes) is checked with 11 ms, +10%.
        write_estimate(cmp, "render/100k", "ci", 11.0)
        code, out = run_quiet(cmd_compare, cmp, "main", "ci", 15.0)
        check("compare +10% under 15", code, 0, out)

        # compare with no common benches.
        none = tmp / "none"
        write_estimate(none, "a/x", "main", 1.0)
        write_estimate(none, "b/y", "ci", 1.0)
        code, out = run_quiet(cmd_compare, none, "main", "ci", 15.0)
        check("compare without common benches", code, 0, out, "nothing to compare")

        # the real gates file parses and every gate names a limit.
        try:
            real = tomllib.loads(DEFAULT_GATES.read_text()).get("gate", [])
            bad = [g.get("bench") for g in real
                   if not ("ci_max_ms" in g and "max_ms" in g)
                   and not ("ci_min_mb_s" in g and "min_mb_s" in g)]
            if not real or bad:
                failures.append(f"bench-gates.toml: gates without both limits: {bad}")
            else:
                print(f"ok  bench-gates.toml ({len(real)} gates, both limits each)")
        except (OSError, tomllib.TOMLDecodeError) as e:
            failures.append(f"bench-gates.toml: {e}")

    if failures:
        print("\nself-test FAILED:")
        for f in failures:
            print(f"- {f}")
        return 1
    print("\nself-test passed")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--criterion", type=Path, default=None,
                        help="criterion's output directory (default target/criterion)")
    sub = parser.add_subparsers(dest="command", required=True)
    g = sub.add_parser("gate", help="enforce scripts/bench-gates.toml")
    g.add_argument("--baseline", required=True)
    g.add_argument("--gates", type=Path, default=DEFAULT_GATES)
    c = sub.add_parser("compare", help="fail on a median regression")
    c.add_argument("--old", required=True)
    c.add_argument("--new", required=True)
    c.add_argument("--threshold", type=float, default=15.0, help="percent")
    sub.add_parser("self-test", help="check this script against synthetic data")
    args = parser.parse_args(argv)

    criterion = args.criterion or default_criterion_dir()
    if args.command == "gate":
        return cmd_gate(criterion, args.gates, args.baseline)
    if args.command == "compare":
        return cmd_compare(criterion, args.old, args.new, args.threshold)
    return self_test()


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
