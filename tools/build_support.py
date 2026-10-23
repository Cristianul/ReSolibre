"""Shared native build setup for the two repository-root build entry points."""

import argparse
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
PACKAGE = "resolibre-desktop"


class BuildError(Exception):
    """A missing prerequisite or unsupported native build configuration."""


def positive_jobs(value):
    """Parse a positive worker limit without silently accepting an invalid build option."""
    try:
        jobs = int(value)
    except ValueError:
        raise argparse.ArgumentTypeError("jobs must be a positive integer") from None
    if jobs < 1:
        raise argparse.ArgumentTypeError("jobs must be a positive integer")
    return jobs


def require_supported_platform(system, machine):
    """Reject unported desktop platforms before invoking tools or creating build output."""
    if system not in ("Linux", "Windows"):
        label = "macOS" if system == "Darwin" else system
        raise BuildError(
            f"{label} detected. The desktop player supports Linux/Windows x86_64 only."
        )
    if machine.lower() not in ("x86_64", "amd64"):
        raise BuildError(f"{system} {machine} detected; the audio adapter currently requires x86_64.")


def require_tool(name, environment):
    """Resolve a required executable using the caller's PATH, without invoking a shell."""
    executable = shutil.which(name, path=environment.get("PATH", os.defpath))
    if executable is None:
        raise BuildError(f"Required tool not found: {name}. See docs/development/local-builds.md.")
    return executable


def native_target(environment, system="Linux"):
    """Read the installed compiler's host target, rejecting an incompatible toolchain."""
    compiler = require_tool(environment.get("RUSTC", "rustc"), environment)
    result = subprocess.run(
        [compiler, "-vV"], cwd=ROOT, env=environment, check=True, capture_output=True, text=True
    )
    for line in result.stdout.splitlines():
        if line.startswith("host: "):
            target = line.removeprefix("host: ").strip()
            if system == "Linux" and target.startswith("x86_64-") and "-linux-" in target:
                return target
            if system == "Windows" and target == "x86_64-pc-windows-msvc":
                return target
            raise BuildError(f"The Rust host target {target} does not match the supported {system} host.")
    raise BuildError("rustc did not report its host target.")


def prepend_paths(environment, name, directories):
    """Prepend existing native directories while preserving the caller's search path."""
    if not directories:
        return
    values = [str(directory) for directory in directories]
    previous = environment.get(name)
    if previous:
        values.append(previous)
    environment[name] = os.pathsep.join(values)


def build_environment(root, inherited, system="Linux"):
    """Use optional checkout-local Linux libraries without changing the parent shell or host."""
    environment = inherited.copy()
    if system == "Windows":
        return environment
    prefix = root / ".local/native/usr"
    libraries = [directory for directory in (prefix / "lib64", prefix / "lib") if directory.is_dir()]
    if not libraries:
        return environment

    # Cargo's encoded form preserves paths with spaces as single compiler arguments.
    if "CARGO_ENCODED_RUSTFLAGS" in environment:
        encoded = environment["CARGO_ENCODED_RUSTFLAGS"]
        flags = encoded.split("\x1f") if encoded else []
    else:
        flags = environment.get("RUSTFLAGS", "").split()
    for directory in libraries:
        if os.pathsep in str(directory) or "\x1f" in str(directory):
            raise BuildError("The native library path contains a search-path delimiter; move the checkout.")
        flags.extend(["-L", f"native={directory}"])
    environment["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join(flags)
    environment.pop("RUSTFLAGS", None)

    prepend_paths(environment, "LD_LIBRARY_PATH", libraries)
    pkgconfig = [directory / "pkgconfig" for directory in libraries if (directory / "pkgconfig").is_dir()]
    prepend_paths(environment, "PKG_CONFIG_PATH", pkgconfig)
    return environment


def require_mpv(environment):
    """Check the libmpv development package before beginning the expensive Rust build."""
    pkg_config = require_tool("pkg-config", environment)
    # Static-only dependencies in mpv.pc need not be installed for dynamic FFI linking.
    local = ROOT / ".local/native/usr"
    candidates = [local / "lib64/libmpv.so", local / "lib/libmpv.so"]
    result = subprocess.run(
        [pkg_config, "--variable=libdir", "mpv"], cwd=ROOT, env=environment,
        check=False, capture_output=True, text=True,
    )
    if result.returncode == 0 and result.stdout.strip():
        candidates.append(Path(result.stdout.strip()) / "libmpv.so")
    if not any(candidate.is_file() for candidate in candidates):
        raise BuildError(
            "libmpv development files are missing. Install your distribution's libmpv development "
            "package or supply .local/native/usr; see docs/development/native-environment.md."
        )


def windows_runtime(root, environment):
    """Require an explicitly supplied local x64 runtime; never download or search music directories."""
    directory = Path(environment.get("RESOLIBRE_NATIVE_DIR", root / ".local/native/windows"))
    if not directory.is_absolute():
        directory = root / directory
    files = [directory / name for name in ("libmpv-2.dll", "ffmpeg.exe", "ffprobe.exe")]
    for path in files:
        if not path.is_file():
            raise BuildError(f"Missing Windows runtime: {path.name}. See docs/development/local-builds.md.")
        # Reject an incompatible architecture before the costly build, without executing native code.
        with path.open("rb") as binary:
            header = binary.read(64)
            if len(header) != 64 or header[:2] != b"MZ":
                raise BuildError(f"Invalid Windows runtime: {path.name}.")
            offset = int.from_bytes(header[60:64], "little")
            if offset > 1024 * 1024:
                raise BuildError(f"Invalid Windows runtime: {path.name}.")
            binary.seek(offset)
            if binary.read(6) != b"PE\0\0\x64\x86":
                raise BuildError(f"Windows runtime must be x86_64: {path.name}.")
    return files


def stage_windows_runtime(files, executable):
    """Place required runtime files beside the built executable, so launch does not depend on PATH."""
    for source in files:
        destination = executable.parent / source.name
        if source.resolve() != destination.resolve():
            shutil.copy2(source, destination)


def build_command(cargo, profile, target, offline, jobs):
    """Select the host explicitly so external Cargo target settings cannot cross-compile by accident."""
    command = [cargo, "build", "--locked", "--package", PACKAGE, "--bin", PACKAGE, "--target", target]
    if profile == "release":
        command.append("--release")
    if offline:
        command.append("--offline")
    if jobs is not None:
        command.extend(["--jobs", str(jobs)])
    return command


def executable_path(cargo, target, profile, environment):
    """Read Cargo's actual output directory, including custom CARGO_TARGET_DIR/config settings."""
    command = [cargo, "metadata", "--format-version", "1", "--no-deps", "--locked", "--offline"]
    result = subprocess.run(command, cwd=ROOT, env=environment, check=True, capture_output=True, text=True)
    try:
        directory = Path(json.loads(result.stdout)["target_directory"])
    except (ValueError, KeyError, TypeError):
        raise BuildError("Cargo did not report a valid target directory.") from None
    name = PACKAGE + (".exe" if target.endswith("-windows-msvc") else "")
    executable = directory / target / profile / name
    if not executable.is_file():
        raise BuildError("Cargo finished but the expected desktop executable is missing.")
    return executable


def main(profile, arguments=None):
    """Build or build-and-run the native player; return failing tool exit codes unchanged."""
    parser = argparse.ArgumentParser(description=f"Build ReSolibre for this computer ({profile}).")
    parser.add_argument("--offline", action="store_true", help="use cached Cargo dependencies only")
    parser.add_argument("-j", "--jobs", type=positive_jobs, help="limit parallel compiler jobs")
    parser.add_argument("--run", action="store_true", help="start the player after a successful build")
    options = parser.parse_args(arguments)

    try:
        system = platform.system()
        require_supported_platform(system, platform.machine())
        environment = build_environment(ROOT, os.environ, system)
        cargo = require_tool("cargo", environment)
        target = native_target(environment, system)
        runtime = []
        if system == "Windows":
            runtime = windows_runtime(ROOT, environment)
        else:
            require_mpv(environment)
        command = build_command(cargo, profile, target, options.offline, options.jobs)
        print(f"Building ReSolibre: {system} / {target} / {profile}", flush=True)
        subprocess.run(command, cwd=ROOT, env=environment, check=True)
        executable = executable_path(cargo, target, profile, environment)
        stage_windows_runtime(runtime, executable)
        print(f"Built: {executable}", flush=True)
        if options.run:
            return subprocess.run([str(executable)], cwd=ROOT, env=environment, check=False).returncode
        print("Use this script with --run to launch with the same native library setup.")
        return 0
    except BuildError as error:
        print(f"Build unavailable: {error}", file=sys.stderr)
        return 1
    except subprocess.CalledProcessError as error:
        if error.stderr:
            print(error.stderr.rstrip(), file=sys.stderr)
        print(f"Build failed (exit {error.returncode}).", file=sys.stderr)
        return error.returncode if error.returncode > 0 else 1
    except OSError as error:
        print(f"Build could not start: {error}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("Build interrupted.", file=sys.stderr)
        return 130
