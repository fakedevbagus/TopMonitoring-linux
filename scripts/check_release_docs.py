#!/usr/bin/env python3
from pathlib import Path
import re
import subprocess
import sys
import tomllib
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
problems: list[str] = []

with (ROOT / "Cargo.toml").open("rb") as handle:
    cargo = tomllib.load(handle)
with (ROOT / "Cargo.lock").open("rb") as handle:
    lock = tomllib.load(handle)
version = cargo["package"]["version"]
candidate = (ROOT / "SOURCE_CANDIDATE").read_text().strip()
if not re.fullmatch(rf"{re.escape(version)}-r\d+", candidate):
    problems.append(
        f"SOURCE_CANDIDATE {candidate!r} does not identify a revision of {version}"
    )
deb_metadata = cargo["package"].get("metadata", {}).get("deb", {})
for relationship in ("conflicts", "replaces"):
    value = deb_metadata.get(relationship, "")
    if "topmonitoring-no-wayland" not in value:
        problems.append(
            f"Cargo Debian {relationship} does not migrate topmonitoring-no-wayland"
        )
lock_versions = [
    package["version"]
    for package in lock["package"]
    if package.get("name") == cargo["package"]["name"]
]
if lock_versions != [version]:
    problems.append(f"Cargo.lock version {lock_versions!r} does not match {version}")

changelog = (ROOT / "CHANGELOG.md").read_text()
if not re.search(rf"^## \[{re.escape(version)}\](?:\s+-\s+\d\d\d\d-\d\d-\d\d)?$", changelog, re.M):
    problems.append(f"CHANGELOG.md has no release heading for {version}")
readme = (ROOT / "README.md").read_text()
if version not in readme:
    problems.append(f"README.md does not mention package version {version}")

metadata = ET.parse(ROOT / "data/io.github.fakedevbagus.TopMonitoring.metainfo.xml")
release_versions = [node.attrib.get("version") for node in metadata.findall(".//release")]
if version not in release_versions:
    problems.append(f"AppStream releases do not include {version}")

required_docs = [
    "README.md",
    "CHANGELOG.md",
    "docs/TECHNICAL.md",
    "docs/TESTING.md",
    "RELEASE_CHECKLIST.md",
]
for relative in required_docs:
    path = ROOT / relative
    if not path.exists() or path.stat().st_size == 0:
        problems.append(f"missing or empty release document: {relative}")

tag = ""
if (ROOT / ".git").exists():
    try:
        tag = subprocess.run(
            ["git", "describe", "--tags", "--abbrev=0"],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=True,
        ).stdout.strip()
    except (subprocess.CalledProcessError, FileNotFoundError):
        pass
if tag:
    changed = set(
        subprocess.run(
            ["git", "diff", "--name-only", f"{tag}..HEAD"],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=True,
        ).stdout.splitlines()
    )
    if "CHANGELOG.md" not in changed:
        problems.append(f"CHANGELOG.md was not updated since {tag}")
    code_changed = any(path.startswith("src/") for path in changed)
    if code_changed and not ({"README.md", "docs/TECHNICAL.md"} & changed):
        problems.append(f"code changed since {tag}, but README/TECHNICAL did not")
    if "src/config.rs" in changed and "MIGRATION_V2.md" not in changed:
        problems.append(f"config schema changed since {tag}, but MIGRATION_V2.md did not")

if problems:
    print("Release documentation gate failed:", file=sys.stderr)
    print("\n".join(f"- {problem}" for problem in problems), file=sys.stderr)
    raise SystemExit(1)
print(f"release documentation: ok (version {version})")
