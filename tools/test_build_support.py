"""Native build selection, dependency failures and real wrapper execution with owned fake tools."""

import contextlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import build_support as build


class BuildChecks(unittest.TestCase):
    """Protect host selection and environment isolation without downloading or launching real audio."""

    def test_unsupported_hosts_stop_before_tools(self):
        """Windows, macOS, unknown systems and Linux ARM produce a clear early failure."""
        for system, machine in (("Windows", "AMD64"), ("Darwin", "arm64"), ("FreeBSD", "x86_64"), ("Linux", "aarch64")):
            with self.subTest(system=system, machine=machine):
                with patch.object(build.platform, "system", return_value=system), patch.object(build.platform, "machine", return_value=machine):
                    with patch.object(build.subprocess, "run") as run, contextlib.redirect_stderr(io.StringIO()) as errors:
                        self.assertEqual(build.main("debug", []), 1)
                        run.assert_not_called()
                        self.assertIn("Linux x86_64" if system != "Linux" else "requires x86_64", errors.getvalue())

    def test_native_environment_preserves_flags_and_paths(self):
        """Keep caller flags and paths; encode checkout names with spaces without shell interpretation."""
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "checkout with spaces; harmless"
            library = root / ".local/native/usr/lib64"
            (library / "pkgconfig").mkdir(parents=True)
            original = {"RUSTFLAGS": "-C debuginfo=2", "LD_LIBRARY_PATH": "/existing/lib", "PKG_CONFIG_PATH": "/existing/pc"}
            result = build.build_environment(root, original)
            self.assertEqual(original["RUSTFLAGS"], "-C debuginfo=2")
            self.assertEqual(result["CARGO_ENCODED_RUSTFLAGS"].split("\x1f"), ["-C", "debuginfo=2", "-L", f"native={library}"])
            self.assertEqual(result["LD_LIBRARY_PATH"], f"{library}:/existing/lib")
            self.assertEqual(result["PKG_CONFIG_PATH"], f"{library}/pkgconfig:/existing/pc")
            self.assertNotIn("RUSTFLAGS", result)
            original["CARGO_ENCODED_RUSTFLAGS"] = "--cfg\x1fexisting_flag"
            result = build.build_environment(root, original)
            self.assertEqual(result["CARGO_ENCODED_RUSTFLAGS"].split("\x1f")[:2], ["--cfg", "existing_flag"])
            self.assertNotIn("debuginfo=2", result["CARGO_ENCODED_RUSTFLAGS"])

    def test_system_libraries_leave_environment_unchanged(self):
        """A normal system installation needs no injected linker or loader settings."""
        with tempfile.TemporaryDirectory() as temporary:
            original = {"RUSTFLAGS": "-C debuginfo=2"}
            result = build.build_environment(Path(temporary), original)
            self.assertEqual(result, original)
            self.assertIsNot(result, original)

    def test_missing_dependencies_and_wrong_toolchain(self):
        """Missing tools, missing mpv and mismatched compiler hosts fail with actionable errors."""
        with patch.object(build.shutil, "which", return_value=None):
            with self.assertRaisesRegex(build.BuildError, "Required tool not found"):
                build.require_tool("cargo", {})
        with patch.object(build.shutil, "which", return_value="/owned/tool"):
            with patch.object(build, "ROOT", Path("/missing-owned-test-root")), patch.object(build.subprocess, "run", return_value=subprocess.CompletedProcess([], 1, "")):
                with self.assertRaisesRegex(build.BuildError, "libmpv development files are missing"):
                    build.require_mpv({})
            for output in ("host: x86_64-pc-windows-msvc", "malformed compiler output"):
                with patch.object(build.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, output)):
                    with self.assertRaises(build.BuildError):
                        build.native_target({})

    def test_invalid_jobs_are_rejected(self):
        """Invalid worker limits never reach Cargo."""
        for value in ("0", "-1", "many"):
            with contextlib.redirect_stderr(io.StringIO()), patch.object(build.subprocess, "run") as run:
                with self.assertRaises(SystemExit) as error:
                    build.main("debug", ["--jobs", value])
                self.assertEqual(error.exception.code, 2)
                run.assert_not_called()

    @unittest.skipUnless(sys.platform.startswith("linux"), "owned executable fixtures use Linux shebangs")
    def test_entry_points_run_from_other_directory_and_propagate_failures(self):
        """Execute both wrappers with fake tools, checking actual arguments, output selection and launch gating."""
        with tempfile.TemporaryDirectory(prefix="resolibre-build-") as temporary:
            root = Path(temporary) / "checkout with spaces"
            (root / "tools").mkdir(parents=True)
            source = Path(build.__file__).resolve().parents[1]
            for name in ("build_debug.py", "build_release.py", "tools/build_support.py"):
                shutil.copyfile(source / name, root / name)
            (root / ".local/native/usr/lib64/pkgconfig").mkdir(parents=True)
            (root / ".local/native/usr/lib64/libmpv.so").write_bytes(b"owned library fixture")
            binaries = Path(temporary) / "owned-tools"
            binaries.mkdir()
            stub = r'''"""Owned fake build tools record invocations and create a harmless executable."""
import json
import os
from pathlib import Path
import sys

name = Path(sys.argv[0]).name
if name == "rustc":
    print("host: x86_64-unknown-linux-gnu")
elif name == "pkg-config":
    # Local dynamic libmpv remains usable without static-only pkg-config dependencies.
    raise SystemExit(1)
elif sys.argv[1] == "metadata":
    print(json.dumps({"target_directory": os.environ["CARGO_TARGET_DIR"]}))
else:
    record = {"args": sys.argv[1:], "cwd": os.getcwd(), "flags": os.environ.get("CARGO_ENCODED_RUSTFLAGS", "")}
    Path(os.environ["BUILD_RECORD"]).write_text(json.dumps(record))
    failure = int(os.environ.get("FAKE_BUILD_EXIT", "0"))
    if failure:
        raise SystemExit(failure)
    profile = "release" if "--release" in sys.argv else "debug"
    output = Path(os.environ["CARGO_TARGET_DIR"]) / "x86_64-unknown-linux-gnu" / profile / "resolibre-desktop"
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text("#!" + sys.executable + "\nimport os\nfrom pathlib import Path\nPath(os.environ['RUN_RECORD']).write_text(os.environ.get('LD_LIBRARY_PATH', ''))\n")
    output.chmod(0o700)
'''
            for name in ("cargo", "rustc", "pkg-config"):
                executable = binaries / name
                executable.write_text("#!" + sys.executable + "\n" + stub)
                executable.chmod(0o700)
            record = Path(temporary) / "build.json"
            run_record = Path(temporary) / "run.txt"
            environment = dict(os.environ, PATH=str(binaries) + os.pathsep + os.defpath,
                               CARGO_TARGET_DIR=str(Path(temporary) / "output with spaces"),
                               CARGO_BUILD_TARGET="x86_64-pc-windows-msvc",
                               BUILD_RECORD=str(record), RUN_RECORD=str(run_record))
            environment.pop("RUSTC", None)
            for profile in ("debug", "release"):
                result = subprocess.run([sys.executable, str(root / f"build_{profile}.py"), "--offline", "-j", "2", "--run"],
                                        cwd=temporary, env=environment, capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                evidence = json.loads(record.read_text())
                self.assertEqual(evidence["cwd"], str(root))
                self.assertIn("--locked", evidence["args"])
                self.assertIn("--offline", evidence["args"])
                self.assertEqual(evidence["args"][evidence["args"].index("--target") + 1], "x86_64-unknown-linux-gnu")
                self.assertEqual("--release" in evidence["args"], profile == "release")
                self.assertIn(str(root / ".local/native/usr/lib64"), run_record.read_text())
                self.assertIn(f"/{profile}/resolibre-desktop", result.stdout)
            run_record.unlink()
            environment["FAKE_BUILD_EXIT"] = "23"
            result = subprocess.run([sys.executable, str(root / "build_debug.py"), "--run"],
                                    cwd=temporary, env=environment, capture_output=True, text=True)
            self.assertEqual(result.returncode, 23)
            self.assertFalse(run_record.exists())
            self.assertNotIn("Built:", result.stdout)


if __name__ == "__main__":
    unittest.main()
