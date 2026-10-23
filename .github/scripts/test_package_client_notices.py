"""Focused fail-closed tests; no Cargo build or private game data required."""

import copy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from package_client_notices import collect, NoticeError, MAX_NOTICE_BYTES


CHECKSUM = "a" * 64


class CollectorTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.root = self.base / "crate"
        self.root.mkdir()
        (self.root / "Cargo.toml").write_text('[package]\nname = "example"\n')
        self.pinned = self.base / "pinned"
        self.pinned.mkdir()
        self.output = self.base / "output"
        self.output.mkdir()
        self.dep = {
            "id": "example-id", "name": "example", "version": "1.0.0",
            "source": "registry+https://github.com/rust-lang/crates.io-index",
            "license": "MIT OR Apache-2.0", "repository": "https://example.invalid/upstream",
            "manifest_path": str(self.root / "Cargo.toml"), "license_file": None,
        }
        self.metadata = {"packages": [self.dep], "resolve": {"nodes": [{"id": "example-id"}]}}
        self.lock = self.base / "Cargo.lock"
        self.lock.write_text('version = 4\n[[package]]\nname = "example"\nversion = "1.0.0"\n'
                             f'source = "{self.dep["source"]}"\nchecksum = "{CHECKSUM}"\n')
        self.manifest = {"schema_version": 1, "packages": [], "files": []}
        self.save_manifest()

    def save_manifest(self):
        (self.pinned / "manifest.json").write_text(json.dumps(self.manifest))

    def add_reviewed(self):
        data = b"An unchanged upstream legal notice.\n"
        (self.pinned / "LICENSE.txt").write_bytes(data)
        blob = b"blob " + str(len(data)).encode() + b"\0" + data
        self.manifest["files"] = [{"path": "LICENSE.txt", "bytes": len(data),
                                   "sha256": hashlib.sha256(data).hexdigest(),
                                   "git_blob_sha1": hashlib.sha1(blob).hexdigest()}]
        self.binding = {key: self.dep[key] for key in (
            "name", "version", "source", "license", "repository"
        )}
        self.binding.update({"package_checksum": CHECKSUM, "vcs_revision": "exact-revision",
                             "path_in_vcs": "crate-path", "notice_files": ["LICENSE.txt"],
                             "source_checks": [{"path": "Cargo.toml", "sha256": hashlib.sha256(
                                 (self.root / "Cargo.toml").read_bytes()).hexdigest()}]})
        self.manifest["packages"] = [self.binding]
        self.vcs = {"git": {"sha1": "exact-revision"}, "path_in_vcs": "crate-path"}
        self.save_vcs()
        self.save_manifest()

    def save_vcs(self):
        (self.root / ".cargo_vcs_info.json").write_text(json.dumps(self.vcs))

    def run_collector(self):
        return collect(self.metadata, self.lock, self.pinned, self.output)

    def test_packaged_nested_notices_preserve_exact_bytes(self):
        nested = self.root / "fonts" / "OFL.txt"
        nested.parent.mkdir()
        nested.write_bytes(b"Font permission and attribution.\n")
        inventory = self.run_collector()
        target = self.output / "dependency-notices/example-1.0.0/packaged/fonts/OFL.txt"
        self.assertEqual(target.read_bytes(), nested.read_bytes())
        self.assertEqual(inventory[0]["license"], "MIT OR Apache-2.0")
        self.assertNotIn(str(self.base), (self.output / "DEPENDENCIES.json").read_text())

    def test_unknown_missing_notice_fails(self):
        with self.assertRaisesRegex(NoticeError, "Missing upstream notices"):
            self.run_collector()

    def test_exact_reviewed_input_is_allowed(self):
        self.add_reviewed()
        self.binding["license_choice"] = "Apache-2.0"
        self.save_manifest()
        inventory = self.run_collector()
        self.assertEqual(inventory[0]["distribution_license_choice"], "Apache-2.0")
        self.assertEqual(inventory[0]["license"], "MIT OR Apache-2.0")

    def test_metadata_mismatches_fail(self):
        self.add_reviewed()
        for field in ("source", "license", "repository"):
            with self.subTest(field=field):
                before = self.dep[field]
                self.dep[field] = "unreviewed"
                try:
                    with self.assertRaisesRegex(NoticeError, "mismatch|Only public crates.io"):
                        self.run_collector()
                finally:
                    self.dep[field] = before

    def test_new_version_has_no_fallback(self):
        self.add_reviewed()
        self.dep["version"] = "1.0.1"
        self.lock.write_text(self.lock.read_text().replace("1.0.0", "1.0.1"))
        with self.assertRaisesRegex(NoticeError, "needs notice review"):
            self.run_collector()

    def test_new_reviewed_version_fails_even_with_packaged_license(self):
        self.add_reviewed()
        (self.root / "LICENSE").write_text("a valid-looking root license")
        self.dep["version"] = "1.0.1"
        self.lock.write_text(self.lock.read_text().replace("1.0.0", "1.0.1"))
        with self.assertRaisesRegex(NoticeError, "needs notice review"):
            self.run_collector()

    def test_lock_checksum_mismatch_fails(self):
        self.add_reviewed()
        self.lock.write_text(self.lock.read_text().replace(CHECKSUM, "b" * 64))
        with self.assertRaisesRegex(NoticeError, "checksum mismatch"):
            self.run_collector()

    def test_vcs_revision_path_and_dirty_mismatches_fail(self):
        self.add_reviewed()
        original = copy.deepcopy(self.vcs)
        changes = [lambda v: v["git"].update(sha1="other"),
                   lambda v: v.update(path_in_vcs="other"),
                   lambda v: v["git"].update(dirty=True)]
        for change in changes:
            self.vcs = copy.deepcopy(original)
            change(self.vcs)
            self.save_vcs()
            with self.assertRaisesRegex(NoticeError, "revision mismatch"):
                self.run_collector()

    def test_source_fingerprint_mismatch_fails(self):
        self.add_reviewed()
        (self.root / "Cargo.toml").write_text("changed declaration")
        with self.assertRaisesRegex(NoticeError, "Reviewed source changed"):
            self.run_collector()

    def test_changed_pinned_notice_fails(self):
        self.add_reviewed()
        (self.pinned / "LICENSE.txt").write_text("modified")
        with self.assertRaisesRegex(NoticeError, "Pinned legal input changed"):
            self.run_collector()

    def test_git_blob_mismatch_fails(self):
        self.add_reviewed()
        self.manifest["files"][0]["git_blob_sha1"] = "wrong"
        self.save_manifest()
        with self.assertRaisesRegex(NoticeError, "Upstream legal blob changed"):
            self.run_collector()

    def test_source_file_is_not_copied_as_a_notice(self):
        for suffix in (".rs", ".py", ".js", ".go", ".sh", ".cpp", ".wgsl"):
            with self.subTest(suffix=suffix):
                source = self.root / f"LICENSE{suffix}"
                source.write_text("implementation source, not a legal notice")
                with self.assertRaisesRegex(NoticeError, "Missing upstream notices"):
                    self.run_collector()
                source.unlink()

    def test_private_git_and_other_registry_sources_fail_without_binding(self):
        (self.root / "LICENSE").write_text("a valid-looking license")
        original = self.dep["source"]
        for source in ("registry+https://private.invalid/index", "git+https://example.invalid/private#abc"):
            with self.subTest(source=source):
                self.dep["source"] = source
                self.lock.write_text(self.lock.read_text().replace(original, source))
                with self.assertRaisesRegex(NoticeError, "Only public crates.io"):
                    self.run_collector()
                self.assertEqual(list(self.output.rglob("*")), [])

    def test_unlocked_ordinary_dependency_fails(self):
        (self.root / "LICENSE").write_text("a valid-looking license")
        self.lock.write_text('version = 4\npackage = []\n')
        with self.assertRaisesRegex(NoticeError, "locked public dependency"):
            self.run_collector()

    def test_duplicate_or_invalid_checksum_ordinary_lock_entry_fails(self):
        (self.root / "LICENSE").write_text("a valid-looking license")
        original = self.lock.read_text()
        self.lock.write_text(original + original[original.index("[[package]]"):])
        with self.assertRaisesRegex(NoticeError, "locked public dependency"):
            self.run_collector()
        self.lock.write_text(original.replace(CHECKSUM, "not-a-sha256"))
        with self.assertRaisesRegex(NoticeError, "locked public dependency"):
            self.run_collector()

    def test_explicit_source_license_file_requires_reviewed_excerpt(self):
        for filename in ("lib.rs", "LICENSE.py"):
            (self.root / filename).write_text("implementation source, not legal text")
            self.dep["license_file"] = filename
            with self.assertRaisesRegex(NoticeError, "reviewed legal excerpts"):
                self.run_collector()

    def test_source_format_pinned_input_is_rejected(self):
        self.add_reviewed()
        (self.pinned / "LICENSE.py").write_bytes((self.pinned / "LICENSE.txt").read_bytes())
        self.manifest["files"][0]["path"] = "LICENSE.py"
        self.binding["notice_files"] = ["LICENSE.py"]
        self.save_manifest()
        with self.assertRaisesRegex(NoticeError, "Unreviewed legal input format"):
            self.run_collector()

    def test_binary_empty_and_oversized_notice_fail(self):
        for data in (b"\0binary", b"", b"x" * (MAX_NOTICE_BYTES + 1)):
            (self.root / "LICENSE").write_bytes(data)
            with self.assertRaises(NoticeError):
                self.run_collector()

    def test_notice_symlink_fails_even_inside_crate(self):
        (self.root / "plain.txt").write_text("legal text")
        (self.root / "LICENSE").symlink_to("plain.txt")
        with self.assertRaisesRegex(NoticeError, "Symlink"):
            self.run_collector()

    def test_license_file_traversal_fails(self):
        (self.base / "outside.txt").write_text("not a crate notice")
        self.dep["license_file"] = "../outside.txt"
        with self.assertRaisesRegex(NoticeError, "Unsafe notice path"):
            self.run_collector()

    def test_bound_package_is_checked_even_with_packaged_notices(self):
        self.add_reviewed()
        (self.root / "LICENSE").write_text("a new legal file")
        self.dep["license"] = "unreviewed"
        with self.assertRaisesRegex(NoticeError, "mismatch"):
            self.run_collector()
        self.assertEqual(list(self.output.rglob("*")), [])

    def test_private_and_inactive_packages_are_not_copied(self):
        private = copy.deepcopy(self.dep)
        private.update(id="private", name="private", source=None)
        inactive = copy.deepcopy(self.dep)
        inactive.update(id="inactive", name="inactive")
        self.metadata["packages"] = [private, inactive]
        self.metadata["resolve"]["nodes"] = [{"id": "private"}]
        self.assertEqual(self.run_collector(), [])

    def test_duplicate_and_unknown_manifest_entries_fail(self):
        self.add_reviewed()
        self.manifest["packages"].append(copy.deepcopy(self.binding))
        self.save_manifest()
        with self.assertRaisesRegex(NoticeError, "Duplicate"):
            self.run_collector()
        self.manifest["packages"].pop()
        self.binding["notice_files"] = ["not-pinned.txt"]
        self.save_manifest()
        with self.assertRaisesRegex(NoticeError, "Unpinned"):
            self.run_collector()


if __name__ == "__main__":
    unittest.main()
