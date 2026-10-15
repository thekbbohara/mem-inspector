#!/usr/bin/python3
"""Installer preservation and destination boundaries, using disposable assets."""

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent


class InstallTests(unittest.TestCase):
    def test_refuses_existing_and_external_destinations(self):
        with tempfile.TemporaryDirectory(prefix="mem-runtime-installer-") as temp:
            prefix = Path(temp) / "prefix"
            for relative in [
                "libwebkit2gtk-4.1.so.0",
                "libjavascriptcoregtk-4.1.so.0",
                "webkit2gtk-4.1/WebKitWebProcess",
                "webkit2gtk-4.1/WebKitNetworkProcess",
            ]:
                p = prefix / "usr/lib" / relative
                p.parent.mkdir(parents=True, exist_ok=True)
                p.write_text("disposable asset")
            binary = Path(temp) / "binary"
            binary.write_text("#!/bin/sh\nexit 0\n")
            binary.chmod(0o700)
            base = Path.home() / ".local/share/mem-inspector"
            # The only cleanup target is the directory created by this test.
            with tempfile.TemporaryDirectory(
                prefix="installer-test-", dir=base
            ) as testdir:
                destination = Path(testdir) / "installation"
                args = [
                    "sh",
                    str(HERE / "install-home-runtime.sh"),
                    str(prefix),
                    str(destination),
                    str(binary),
                ]
                selected = base / "current"
                original_selection = (
                    os.readlink(selected) if selected.is_symlink() else None
                )
                subprocess.run(args, check=True, capture_output=True)
                sentinel = destination / "owner-sentinel"
                sentinel.write_text("preserve existing work")
                result = subprocess.run(
                    args, check=False, capture_output=True, text=True
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("preserving", result.stderr)
                self.assertEqual(sentinel.read_text(), "preserve existing work")
                self.assertEqual(os.readlink(selected), original_selection)
                self.assertTrue((destination / "mem-inspector").stat().st_mode & 0o100)
                self.assertIn(
                    "bin/mem-inspector-desktop",
                    (destination / "assets.sha256").read_text(),
                )
                outside = Path(temp) / "outside"
                args[3] = str(outside)
                self.assertNotEqual(
                    subprocess.run(args, check=False, capture_output=True).returncode, 0
                )
                self.assertFalse(outside.exists())
                symlink = Path(testdir) / "symlinked-parent"
                symlink.symlink_to(Path(temp), target_is_directory=True)
                args[3] = str(symlink / "redirected")
                self.assertNotEqual(
                    subprocess.run(args, check=False, capture_output=True).returncode, 0
                )
                self.assertFalse((Path(temp) / "redirected").exists())
                external_link = prefix / "usr/lib/external-link"
                external_link.symlink_to(binary)
                args[3] = str(Path(testdir) / "external-runtime")
                self.assertNotEqual(
                    subprocess.run(args, check=False, capture_output=True).returncode, 0
                )
                self.assertFalse(Path(args[3]).exists())


if __name__ == "__main__":
    unittest.main()
