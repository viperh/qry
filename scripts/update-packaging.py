#!/usr/bin/env python3
"""Fills the packaging manifests in packaging/ in for one release.

    scripts/update-packaging.py <version> <SHA256SUMS> --source-sha256 <hash> --out <dir>
    scripts/update-packaging.py --self-test
    scripts/update-packaging.py --check-srcinfo

The committed manifests carry the placeholder version 0.0.0 and all-zero
checksums. This script writes copies with the release's version, URLs and
checksums under <dir>, keeping the layout of packaging/:

    aur/qry/PKGBUILD        aur/qry/.SRCINFO        (from the source tarball)
    aur/qry-bin/PKGBUILD    aur/qry-bin/.SRCINFO    (prebuilt musl archives)
    homebrew/qry.rb
    scoop/qry.json

<SHA256SUMS> is the release's combined checksum file (`sha256sum` format).
--source-sha256 is the checksum of GitHub's tarball of the tag
(https://github.com/viperh/qry/archive/refs/tags/v<version>.tar.gz), which
the AUR `qry` package builds from. .SRCINFO is regenerated from each
PKGBUILD, the way `makepkg --printsrcinfo` would for these simple files.

--self-test runs the rewrite against a synthetic SHA256SUMS and asserts
that every placeholder was replaced and that each .SRCINFO matches its
PKGBUILD. --check-srcinfo checks that the committed .SRCINFO files match
the committed PKGBUILDs.

Standard library only. Exit codes: 0 success, 1 a failed check, 2 bad usage
or input.
"""

from __future__ import annotations

import argparse
import json
import re
import shlex
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PACKAGING = ROOT / "packaging"

PLACEHOLDER_VERSION = "0.0.0"
ZERO_HASH = "0" * 64

# The archive labels scripts/release-package.sh produces, and their suffix.
ARCHIVES = {
    "linux-x86_64": ".tar.gz",
    "linux-aarch64": ".tar.gz",
    "macos-universal": ".tar.gz",
    "windows-x86_64": ".zip",
}

VERSION_RE = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.+-]+)?$")
HASH_RE = re.compile(r"^[0-9a-f]{64}$")

# Where each AUR field goes in .SRCINFO, in makepkg's order.
SRCINFO_FIELDS = [
    "pkgdesc", "pkgver", "pkgrel", "epoch", "url", "install", "changelog",
    "arch", "groups", "license", "checkdepends", "makedepends", "depends",
    "optdepends", "provides", "conflicts", "replaces", "noextract", "options",
    "backup", "source", "validpgpkeys", "md5sums", "sha1sums", "sha224sums",
    "sha256sums", "sha384sums", "sha512sums", "b2sums",
]
ARCH_FIELDS = [
    "source", "checkdepends", "makedepends", "depends", "optdepends",
    "provides", "conflicts", "replaces", "md5sums", "sha1sums", "sha224sums",
    "sha256sums", "sha384sums", "sha512sums", "b2sums",
]


class InputError(Exception):
    """Bad arguments or input files (exit code 2)."""


def archive_name(version: str, label: str) -> str:
    return f"qry-{version}-{label}{ARCHIVES[label]}"


def read_sums(text: str) -> dict[str, str]:
    """Parses `sha256sum` output into {file name: hash}."""
    sums = {}
    for number, line in enumerate(text.splitlines(), 1):
        if not line.strip():
            continue
        parts = line.split(None, 1)
        if len(parts) != 2 or not HASH_RE.match(parts[0].lower()):
            raise InputError(f"SHA256SUMS line {number} is not '<sha256>  <file>': {line!r}")
        sums[parts[1].strip().lstrip("*")] = parts[0].lower()
    return sums


def archive_hashes(version: str, sums: dict[str, str]) -> dict[str, str]:
    """{label: hash} for every archive the manifests need."""
    hashes = {}
    for label in ARCHIVES:
        name = archive_name(version, label)
        if name not in sums:
            raise InputError(f"SHA256SUMS has no entry for {name}")
        hashes[label] = sums[name]
    return hashes


# --- PKGBUILD and .SRCINFO ---------------------------------------------------

ASSIGN_RE = re.compile(r"^([A-Za-z_][A-Za-z0-9_]*)=(.*)$")


def parse_pkgbuild(text: str) -> dict[str, list[str]]:
    """The top-level variable assignments of a simple PKGBUILD.

    Handles `name=value` and `name=(...)` (also over several lines), with
    the values word-split and unquoted the way bash would, and expands
    `$var` / `${var}` from earlier assignments. Functions and comments are
    skipped. That is all the manifests in packaging/ use.
    """
    values: dict[str, list[str]] = {}
    lines = text.splitlines()
    i = 0
    depth = 0
    while i < len(lines):
        line = lines[i]
        i += 1
        stripped = line.strip()
        if depth:
            depth += stripped.count("{") - stripped.count("}")
            continue
        if not stripped or stripped.startswith("#"):
            continue
        if re.match(r"^[A-Za-z_][A-Za-z0-9_]*\(\)\s*\{", stripped):
            depth = stripped.count("{") - stripped.count("}")
            continue
        match = ASSIGN_RE.match(stripped)
        if not match:
            raise InputError(f"cannot parse PKGBUILD line: {line!r}")
        name, rest = match.groups()
        if rest.startswith("("):
            body = rest[1:]
            while not body.rstrip().endswith(")"):
                if i >= len(lines):
                    raise InputError(f"unterminated array {name}")
                body += "\n" + lines[i]
                i += 1
            words = shlex.split(body.rstrip()[:-1], comments=True)
        else:
            words = shlex.split(rest, comments=True)[:1]
        values[name] = [expand(word, values) for word in words]
    return values


def expand(word: str, values: dict[str, list[str]]) -> str:
    def lookup(match: re.Match[str]) -> str:
        name = match.group(1) or match.group(2)
        if name not in values:
            raise InputError(f"PKGBUILD uses ${name} before setting it")
        return " ".join(values[name])

    return re.sub(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}|\$([A-Za-z_][A-Za-z0-9_]*)", lookup, word)


def srcinfo(pkgbuild: str) -> str:
    """The .SRCINFO for a single-package PKGBUILD."""
    values = parse_pkgbuild(pkgbuild)
    for required in ("pkgname", "pkgver", "pkgrel", "arch"):
        if required not in values:
            raise InputError(f"PKGBUILD has no {required}")
    name = values["pkgname"][0]
    out = [f"pkgbase = {name}"]
    for field in SRCINFO_FIELDS:
        for value in values.get(field, []):
            out.append(f"\t{field} = {value}")
    for arch in values["arch"]:
        for field in ARCH_FIELDS:
            for value in values.get(f"{field}_{arch}", []):
                out.append(f"\t{field}_{arch} = {value}")
    out += ["", f"pkgname = {name}", ""]
    return "\n".join(out)


def set_pkgbuild(text: str, version: str, sums: dict[str, str]) -> str:
    """Sets pkgver, resets pkgrel and fills each sha256sums* array.

    `sums` maps the array name (sha256sums, sha256sums_x86_64, …) to its
    hash. Every array the PKGBUILD has must be in it.
    """
    text, n = re.subn(r"(?m)^pkgver=.*$", f"pkgver={version}", text)
    if n != 1:
        raise InputError("PKGBUILD has no single pkgver= line")
    text = re.sub(r"(?m)^pkgrel=.*$", "pkgrel=1", text)

    def fill(match: re.Match[str]) -> str:
        name = match.group(1)
        if name not in sums:
            raise InputError(f"no checksum for {name}")
        return f"{name}=('{sums[name]}')"

    return re.sub(r"(?m)^(sha256sums(?:_[a-z0-9_]+)?)=\(.*\)$", fill, text)


# --- the rewrite --------------------------------------------------------------


def render(version: str, hashes: dict[str, str], source_sha256: str) -> dict[str, str]:
    """{path relative to packaging/: content} for the release."""
    files: dict[str, str] = {}

    for package, sums in (
        ("qry", {"sha256sums": source_sha256}),
        ("qry-bin", {
            "sha256sums_x86_64": hashes["linux-x86_64"],
            "sha256sums_aarch64": hashes["linux-aarch64"],
        }),
    ):
        template = (PACKAGING / "aur" / package / "PKGBUILD").read_text()
        pkgbuild = set_pkgbuild(template, version, sums)
        files[f"aur/{package}/PKGBUILD"] = pkgbuild
        files[f"aur/{package}/.SRCINFO"] = srcinfo(pkgbuild)

    # The formula: each zero `sha256` belongs to the `url` just above it.
    formula = (PACKAGING / "homebrew" / "qry.rb").read_text()
    formula = formula.replace(f'version "{PLACEHOLDER_VERSION}"', f'version "{version}"')
    formula = formula.replace(f"/v{PLACEHOLDER_VERSION}/qry-{PLACEHOLDER_VERSION}-",
                              f"/v{version}/qry-{version}-")
    lines = []
    current = None
    for line in formula.splitlines(keepends=True):
        url = re.search(r'url "[^"]*/(qry-[^"/]+)"', line)
        if url:
            current = url.group(1)
        if ZERO_HASH in line:
            label = next((l for l in ARCHIVES if current == archive_name(version, l)), None)
            if label is None:
                raise InputError(f"homebrew/qry.rb: sha256 with no known url above it ({current})")
            line = line.replace(ZERO_HASH, hashes[label])
            current = None
        lines.append(line)
    files["homebrew/qry.rb"] = "".join(lines)

    manifest = json.loads((PACKAGING / "scoop" / "qry.json").read_text())
    manifest["version"] = version
    win = manifest["architecture"]["64bit"]
    stem = f"qry-{version}-windows-x86_64"
    win["url"] = f"https://github.com/viperh/qry/releases/download/v{version}/{stem}.zip"
    win["hash"] = hashes["windows-x86_64"]
    win["extract_dir"] = stem
    files["scoop/qry.json"] = json.dumps(manifest, indent=4) + "\n"

    return files


def write(files: dict[str, str], out: Path) -> None:
    for relative, content in files.items():
        path = out / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)


def update(version: str, sums_path: Path, source_sha256: str, out: Path) -> None:
    if not VERSION_RE.match(version):
        raise InputError(f"{version!r} is not a version like 1.2.3")
    if not HASH_RE.match(source_sha256.lower()):
        raise InputError("--source-sha256 is not a sha256 hex digest")
    try:
        sums = read_sums(sums_path.read_text())
    except OSError as error:
        raise InputError(f"cannot read {sums_path}: {error}") from error
    hashes = archive_hashes(version, sums)
    files = render(version, hashes, source_sha256.lower())
    write(files, out)
    for relative in sorted(files):
        print(out / relative)


# --- checks -------------------------------------------------------------------


def check_srcinfo() -> list[str]:
    """Problems with the committed .SRCINFO files."""
    problems = []
    for package in ("qry", "qry-bin"):
        directory = PACKAGING / "aur" / package
        expected = srcinfo((directory / "PKGBUILD").read_text())
        actual = (directory / ".SRCINFO").read_text()
        if actual != expected:
            problems.append(f"packaging/aur/{package}/.SRCINFO does not match its PKGBUILD; "
                            "regenerate it with --write-srcinfo")
    return problems


def self_test() -> list[str]:
    problems = check_srcinfo()
    version = "1.2.3"
    # Distinct, recognisable hashes: 1111…, 2222…, …
    hashes = {label: str(n) * 64 for n, label in enumerate(ARCHIVES, 1)}
    source = "a" * 64
    sums_text = "".join(f"{hashes[l]}  {archive_name(version, l)}\n" for l in sorted(ARCHIVES))

    with tempfile.TemporaryDirectory() as tmp:
        tmp_path = Path(tmp)
        (tmp_path / "SHA256SUMS").write_text(sums_text)
        out = tmp_path / "out"
        update(version, tmp_path / "SHA256SUMS", source, out)
        written = {p.relative_to(out).as_posix(): p.read_text()
                   for p in out.rglob("*") if p.is_file()}

        expected = {"aur/qry/PKGBUILD", "aur/qry/.SRCINFO", "aur/qry-bin/PKGBUILD",
                    "aur/qry-bin/.SRCINFO", "homebrew/qry.rb", "scoop/qry.json"}
        if set(written) != expected:
            problems.append(f"wrote {sorted(written)}, expected {sorted(expected)}")

        for name, content in written.items():
            if PLACEHOLDER_VERSION in content:
                problems.append(f"{name} still contains {PLACEHOLDER_VERSION}")
            if ZERO_HASH in content:
                problems.append(f"{name} still contains a zero hash")
            if version not in content:
                problems.append(f"{name} does not mention {version}")

        def expect(name: str, *needles: str) -> None:
            for needle in needles:
                if needle not in written.get(name, ""):
                    problems.append(f"{name} lacks {needle!r}")

        expect("aur/qry/PKGBUILD", f"sha256sums=('{source}')")
        expect("aur/qry-bin/PKGBUILD",
               f"sha256sums_x86_64=('{hashes['linux-x86_64']}')",
               f"sha256sums_aarch64=('{hashes['linux-aarch64']}')")
        expect("aur/qry-bin/.SRCINFO",
               "\tpkgver = 1.2.3\n",
               "\tsource_x86_64 = https://github.com/viperh/qry/releases/download/"
               "v1.2.3/qry-1.2.3-linux-x86_64.tar.gz\n",
               f"\tsha256sums_aarch64 = {hashes['linux-aarch64']}\n")
        expect("aur/qry/.SRCINFO",
               "\tsource = qry-1.2.3.tar.gz::https://github.com/viperh/qry/archive/refs/tags/"
               "v1.2.3.tar.gz\n")
        expect("homebrew/qry.rb", *(hashes[l] for l in ARCHIVES if l != "windows-x86_64"),
               'version "1.2.3"')
        # Each formula hash sits right under the url of its own archive.
        formula = written.get("homebrew/qry.rb", "")
        for label in ("macos-universal", "linux-x86_64", "linux-aarch64"):
            pattern = (re.escape(archive_name(version, label)) + r'"\s*\n\s*sha256 "'
                       + hashes[label])
            if not re.search(pattern, formula):
                problems.append(f"homebrew/qry.rb: {label} hash is not under its url")

        for package in ("qry", "qry-bin"):
            pkgbuild = written.get(f"aur/{package}/PKGBUILD", "")
            if written.get(f"aur/{package}/.SRCINFO") != srcinfo(pkgbuild):
                problems.append(f"aur/{package}/.SRCINFO does not match its PKGBUILD")

        try:
            manifest = json.loads(written.get("scoop/qry.json", ""))
            win = manifest["architecture"]["64bit"]
            if win["hash"] != hashes["windows-x86_64"]:
                problems.append("scoop/qry.json has the wrong hash")
            if not win["url"].endswith("/v1.2.3/qry-1.2.3-windows-x86_64.zip"):
                problems.append(f"scoop/qry.json has the wrong url {win['url']}")
            if "$version" not in manifest["autoupdate"]["architecture"]["64bit"]["url"]:
                problems.append("scoop/qry.json lost its autoupdate $version")
        except (ValueError, KeyError) as error:
            problems.append(f"scoop/qry.json is not the expected JSON: {error}")

        # A SHA256SUMS that lacks an archive is an input error.
        (tmp_path / "short").write_text(sums_text.splitlines(keepends=True)[0])
        try:
            update(version, tmp_path / "short", source, tmp_path / "short-out")
            problems.append("a SHA256SUMS missing archives was accepted")
        except InputError:
            pass
    return problems


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("version", nargs="?")
    parser.add_argument("sha256sums", nargs="?", type=Path)
    parser.add_argument("--source-sha256")
    parser.add_argument("--out", type=Path)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--check-srcinfo", action="store_true")
    parser.add_argument("--write-srcinfo", action="store_true",
                        help="regenerate the committed .SRCINFO files from their PKGBUILDs")
    args = parser.parse_args(argv)

    try:
        if args.self_test or args.check_srcinfo:
            problems = self_test() if args.self_test else check_srcinfo()
            for problem in problems:
                print(f"FAIL: {problem}", file=sys.stderr)
            if not problems:
                print("ok" if args.check_srcinfo else "self-test passed")
            return 1 if problems else 0
        if args.write_srcinfo:
            for package in ("qry", "qry-bin"):
                directory = PACKAGING / "aur" / package
                (directory / ".SRCINFO").write_text(srcinfo((directory / "PKGBUILD").read_text()))
            return 0
        if not (args.version and args.sha256sums and args.source_sha256 and args.out):
            parser.error("need <version> <SHA256SUMS> --source-sha256 <hash> --out <dir>")
        update(args.version, args.sha256sums, args.source_sha256, args.out)
        return 0
    except InputError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
