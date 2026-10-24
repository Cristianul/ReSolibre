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

    def test_detector_notices_accompany_native_builds(self):
        """The embedded model's MIT notice must not depend on a network download or the source tree at runtime."""
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "licenses").mkdir()
            (root / "output").mkdir()
            for name in ("lossprint-MIT.txt", "tract-MIT.txt"):
                (root / "licenses" / name).write_text("owned notice fixture", encoding="utf-8")
            build.stage_detector_licenses(root, root / "output" / "player.exe")
            for name in ("lossprint-MIT.txt", "tract-MIT.txt"):
                self.assertEqual((root / "output" / "licenses" / name).read_text(), "owned notice fixture")

    def test_unsupported_hosts_stop_before_tools(self):
        """macOS, unknown systems and unsupported architectures fail before any tools run."""
        for system, machine in (("Windows", "ARM64"), ("Darwin", "arm64"), ("FreeBSD", "x86_64"), ("Linux", "aarch64")):
            with self.subTest(system=system, machine=machine):
                with patch.object(build.platform, "system", return_value=system), patch.object(build.platform, "machine", return_value=machine):
                    with patch.object(build.subprocess, "run") as run, contextlib.redirect_stderr(io.StringIO()) as errors:
                        self.assertEqual(build.main("debug", []), 1)
                        run.assert_not_called()
                        self.assertIn("x86_64", errors.getvalue())

    def test_native_environment_preserves_flags_and_paths(self):
        """Keep caller flags and paths; encode checkout names with spaces without shell interpretation."""
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "checkout with spaces"
            library = root / ".local/native/usr/lib64"
            (library / "pkgconfig").mkdir(parents=True)
            original = {"RUSTFLAGS": "-C debuginfo=2", "LD_LIBRARY_PATH": "/existing/lib", "PKG_CONFIG_PATH": "/existing/pc"}
            result = build.build_environment(root, original)
            self.assertEqual(original["RUSTFLAGS"], "-C debuginfo=2")
            self.assertEqual(result["CARGO_ENCODED_RUSTFLAGS"].split("\x1f"), ["-C", "debuginfo=2", "-L", f"native={library}"])
            self.assertEqual(result["LD_LIBRARY_PATH"], f"{library}{os.pathsep}/existing/lib")
            self.assertEqual(result["PKG_CONFIG_PATH"], f"{library / 'pkgconfig'}{os.pathsep}/existing/pc")
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

    def test_windows_host_and_environment_are_isolated(self):
        """Accept x64 MSVC only and keep Linux linker paths out of Windows child processes."""
        build.require_supported_platform("Windows", "AMD64")
        with patch.object(build.shutil, "which", return_value="rustc.exe"):
            for host in ("x86_64-pc-windows-msvc", "x86_64-pc-windows-gnu", "aarch64-pc-windows-msvc"):
                with patch.object(build.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, f"host: {host}")):
                    if host.endswith("msvc") and host.startswith("x86_64"):
                        self.assertEqual(build.native_target({}, "Windows"), host)
                    else:
                        with self.assertRaises(build.BuildError):
                            build.native_target({}, "Windows")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / ".local/native/usr/lib64").mkdir(parents=True)
            inherited = {"PATH": "owned tools", "RUSTFLAGS": "-C debuginfo=2"}
            self.assertEqual(build.build_environment(root, inherited, "Windows"), inherited)

    def test_windows_runtime_missing_wrong_architecture_and_staging(self):
        """Validate all native files before Cargo; stage only beside the chosen executable."""
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            runtime = root / "runtime with spaces"
            runtime.mkdir()
            environment = {"RESOLIBRE_NATIVE_DIR": str(runtime)}
            with self.assertRaisesRegex(build.BuildError, "Missing Windows runtime"):
                build.windows_runtime(root, environment)
            header = bytearray(64)
            header[:2] = b"MZ"
            header[60:64] = (64).to_bytes(4, "little")
            payload = bytes(header) + b"PE\0\0\x64\x86"
            for name in ("libmpv-2.dll", "ffmpeg.exe", "ffprobe.exe"):
                (runtime / name).write_bytes(payload)
            files = build.windows_runtime(root, environment)
            executable = root / "custom output" / "resolibre-desktop.exe"
            executable.parent.mkdir()
            build.stage_windows_runtime(files, executable)
            for source in files:
                self.assertEqual((executable.parent / source.name).read_bytes(), payload)
            (runtime / "ffprobe.exe").write_bytes(bytes(header) + b"PE\0\0\x4c\x01")
            with self.assertRaisesRegex(build.BuildError, "must be x86_64"):
                build.windows_runtime(root, environment)

    def test_windows_build_failure_never_stages_or_runs(self):
        """A failed compiler propagates its status and cannot launch a stale executable."""
        with patch.object(build.platform, "system", return_value="Windows"), patch.object(build.platform, "machine", return_value="AMD64"):
            with patch.object(build, "require_tool", return_value="cargo.exe"), patch.object(build, "native_target", return_value="x86_64-pc-windows-msvc"):
                with patch.object(build, "windows_runtime", return_value=[]), patch.object(build, "stage_windows_runtime") as stage:
                    with patch.object(build.subprocess, "run", side_effect=subprocess.CalledProcessError(23, ["cargo.exe"])) as run:
                        with contextlib.redirect_stderr(io.StringIO()), contextlib.redirect_stdout(io.StringIO()):
                            self.assertEqual(build.main("release", ["--offline", "--run"]), 23)
                        self.assertEqual(run.call_count, 1)
                        stage.assert_not_called()

    @unittest.skipUnless(sys.platform == "win32", "requires Windows PowerShell")
    def test_powershell_entry_from_another_directory_and_failure_exit(self):
        """Run the real PowerShell wrapper with owned Python entry points and paths containing spaces."""
        with tempfile.TemporaryDirectory(prefix="resolibre wrapper ") as temporary:
            root = Path(temporary) / "checkout with spaces; literal"
            root.mkdir()
            source = Path(build.__file__).resolve().parents[1]
            shutil.copyfile(source / "build_windows.ps1", root / "build_windows.ps1")
            stub = "import json,sys\nfrom pathlib import Path\nprint(json.dumps([Path(__file__).name, sys.argv[1:]]))\nraise SystemExit(23 if '--run' in sys.argv else 0)\n"
            for profile in ("debug", "release"):
                (root / f"build_{profile}.py").write_text(stub)
                command = ["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(root / "build_windows.ps1"),
                           "-Python", sys.executable, "-Profile", profile, "-Offline", "-Jobs", "2"]
                result = subprocess.run(command, cwd=temporary, capture_output=True, text=True, timeout=20)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(json.loads(result.stdout), [f"build_{profile}.py", ["--offline", "--jobs", "2"]])
                failure = subprocess.run(command + ["-Run"], cwd=temporary, capture_output=True, text=True, timeout=20)
                self.assertEqual(failure.returncode, 23, failure.stderr)

    @unittest.skipUnless(sys.platform.startswith("linux"), "owned executable fixtures use Linux shebangs")
    def test_entry_points_run_from_other_directory_and_propagate_failures(self):
        """Execute both wrappers with fake tools, checking actual arguments, output selection and launch gating."""
        with tempfile.TemporaryDirectory(prefix="resolibre-build-") as temporary:
            root = Path(temporary) / "checkout with spaces"
            (root / "tools").mkdir(parents=True)
            source = Path(build.__file__).resolve().parents[1]
            for name in ("build_debug.py", "build_release.py", "tools/build_support.py"):
                shutil.copyfile(source / name, root / name)
            shutil.copytree(source / "licenses", root / "licenses")
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
