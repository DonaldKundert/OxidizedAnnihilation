"""Collect legal text only, with exact reviewed bindings for incomplete crates.

This does not replace cargo-about's independent fail-closed license check.
It never downloads files or treats an unreviewed missing notice as acceptable.
"""

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import tomllib


NOTICE_NAMES = (
    "LICENSE", "LICENCE", "COPYING", "COPYRIGHT", "NOTICE", "UNLICENSE",
    "OFL", "AUTHORS", "PATENTS",
)
# Only reviewed text-notice formats are copied, never implementation extensions.
# New formats need review rather than an ever-growing source-code denylist.
LEGAL_SUFFIXES = {
    "", ".txt", ".md", ".markdown", ".rst", ".html", ".htm", ".mit",
    ".apache", ".bsd", ".isc", ".zlib", ".unicode", ".bsl", ".unlicense",
    ".chromium", ".lib", ".0_with_llvm-exception",
}
CRATES_IO_SOURCE = "registry+https://github.com/rust-lang/crates.io-index"
MAX_NOTICE_BYTES = 2 * 1024 * 1024


class NoticeError(ValueError):
    """A legal input needs review before distribution."""


def contained_file(root, relative):
    """Reject absolute paths, traversal and symlinks, including parent links."""
    relative = Path(relative)
    if relative.is_absolute() or not relative.parts or ".." in relative.parts:
        raise NoticeError(f"Unsafe notice path: {relative}")
    root = root.resolve()
    path = root / relative
    for parent in (path, *path.parents):
        if parent == root:
            break
        if parent.is_symlink():
            raise NoticeError(f"Symlink in notice path: {relative}")
    path.resolve().relative_to(root)
    if not path.is_file():
        raise NoticeError(f"Missing legal input: {relative}")
    return path


def legal_bytes(path):
    if path.stat().st_size > MAX_NOTICE_BYTES:
        raise NoticeError(f"Unexpectedly large legal input: {path.name}")
    data = path.read_bytes()
    data.decode("utf-8")
    if not data or b"\0" in data:
        raise NoticeError(f"Empty or binary legal input: {path.name}")
    return data


def checked_inputs(directory):
    manifest = json.loads((directory / "manifest.json").read_text())
    if manifest.get("schema_version") != 1:
        raise NoticeError("Unsupported pinned-notice manifest")
    files = {}
    for record in manifest["files"]:
        name = record["path"]
        if name in files:
            raise NoticeError(f"Duplicate pinned legal input: {name}")
        if Path(name).suffix.lower() not in LEGAL_SUFFIXES:
            raise NoticeError(f"Unreviewed legal input format: {name}")
        path = contained_file(directory, name)
        data = legal_bytes(path)
        if len(data) != record["bytes"] or hashlib.sha256(data).hexdigest() != record["sha256"]:
            raise NoticeError(f"Pinned legal input changed: {name}")
        if "git_blob_sha1" in record:
            blob = b"blob " + str(len(data)).encode() + b"\0" + data
            if hashlib.sha1(blob).hexdigest() != record["git_blob_sha1"]:
                raise NoticeError(f"Upstream legal blob changed: {name}")
        files[name] = (path, record)
    packages = {}
    for record in manifest["packages"]:
        key = (record["name"], record["version"])
        if key in packages or not record["notice_files"]:
            raise NoticeError(f"Duplicate or empty package binding: {key}")
        if not set(record["notice_files"]).issubset(files):
            raise NoticeError(f"Unpinned package legal input: {key}")
        packages[key] = record
    return packages, files


def check_package_binding(dependency, root, binding, locked):
    for field in ("name", "version", "source", "license", "repository"):
        if dependency.get(field) != binding[field]:
            raise NoticeError(f"Reviewed package {field} mismatch: {dependency['name']}")
    entries = [item for item in locked if all(
        item.get(key) == binding[key] for key in ("name", "version", "source")
    )]
    if len(entries) != 1 or entries[0].get("checksum") != binding["package_checksum"]:
        raise NoticeError(f"Reviewed package checksum mismatch: {dependency['name']}")
    vcs = json.loads(contained_file(root, ".cargo_vcs_info.json").read_text())
    if (vcs["git"]["sha1"] != binding["vcs_revision"]
            or vcs.get("path_in_vcs", "") != binding["path_in_vcs"]
            or vcs["git"].get("dirty", False)):
        raise NoticeError(f"Reviewed package source revision mismatch: {dependency['name']}")
    for check in binding.get("source_checks", []):
        source = contained_file(root, check["path"])
        if hashlib.sha256(source.read_bytes()).hexdigest() != check["sha256"]:
            raise NoticeError(f"Reviewed source changed: {dependency['name']}/{check['path']}")


def check_public_lock_entry(dependency, locked):
    """Never redistribute private, Git, or unlocked package content."""
    if dependency["source"] != CRATES_IO_SOURCE:
        raise NoticeError(f"Only public crates.io dependencies may be packaged: {dependency['name']}")
    entries = [item for item in locked if all(
        item.get(key) == dependency[key] for key in ("name", "version", "source")
    )]
    if len(entries) != 1 or not re.fullmatch(r"[0-9a-f]{64}", entries[0].get("checksum", "")):
        raise NoticeError(f"Missing or ambiguous locked public dependency: {dependency['name']}")
    return entries[0]["checksum"]


def collect(metadata, lockfile, pinned_directory, output):
    bindings, pinned = checked_inputs(pinned_directory)
    locked = tomllib.loads(lockfile.read_text())["package"]
    active = {node["id"] for node in metadata["resolve"]["nodes"]}
    inventory = []
    for dep in sorted(metadata["packages"], key=lambda item: (item["name"], item["version"])):
        if dep["id"] not in active or dep["source"] is None:
            continue
        checksum = check_public_lock_entry(dep, locked)
        name, version = dep["name"], dep["version"]
        binding = bindings.get((name, version))
        if binding is None and any(key[0] == name for key in bindings):
            raise NoticeError(f"New version of reviewed dependency needs notice review: {name} {version}")
        if any(char in name + version for char in ("/", "\\")) or name in (".", ".."):
            raise NoticeError("Unsafe package name/version")
        root = Path(dep["manifest_path"]).parent.resolve()
        if binding:
            check_package_binding(dep, root, binding, locked)
        dest = output / "dependency-notices" / f"{name}-{version}"
        originals = {
            path.relative_to(root).as_posix()
            for path in root.rglob("*")
            if path.is_file() and path.suffix.lower() in LEGAL_SUFFIXES
            and any(token in path.name.upper() for token in NOTICE_NAMES)
        }
        if dep.get("license_file"):
            if Path(dep["license_file"]).suffix.lower() not in LEGAL_SUFFIXES:
                raise NoticeError(f"Source files require reviewed legal excerpts: {name}")
            originals.add(dep["license_file"])
        notices = []
        for relative in sorted(originals):
            source = contained_file(root, relative)
            data = legal_bytes(source)
            target = dest / "packaged" / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
            notices.append({"path": target.relative_to(output).as_posix(),
                            "origin": "published-crate", "sha256": hashlib.sha256(data).hexdigest()})
        if binding:
            for relative in binding["notice_files"]:
                source, record = pinned[relative]
                target = dest / "reviewed" / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(source, target)
                notices.append({"path": target.relative_to(output).as_posix(),
                                "origin": "reviewed-pinned-input", "provenance": record})
        if not notices:
            raise NoticeError(f"Missing upstream notices for {name} {version}; review before distribution")
        item = {key: dep[key] for key in ("name", "version", "license", "repository")}
        item.update(source=dep["source"], package_checksum=checksum)
        item["notices"] = notices
        if binding:
            item["reviewed_source"] = {key: binding[key] for key in (
                "source", "package_checksum", "vcs_revision", "path_in_vcs"
            )}
            if binding.get("license_choice"):
                item["distribution_license_choice"] = binding["license_choice"]
        inventory.append(item)
    (output / "DEPENDENCIES.json").write_text(json.dumps(inventory, indent=2) + "\n")
    print(f"Dependency notice inventory: {len(inventory)} packages")
    return inventory


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--metadata", type=Path, required=True)
    parser.add_argument("--lockfile", type=Path, required=True)
    parser.add_argument("--pinned-notices", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    collect(json.loads(args.metadata.read_text()), args.lockfile, args.pinned_notices, args.output)


if __name__ == "__main__":
    main()
