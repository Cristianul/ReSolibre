#!/usr/bin/env python3
"""Verify window snapping and rendering inside an owned, isolated KWin/XWayland desktop."""

import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / ".local/window-snap"


def run_child():
    """Warm the private X server, then run native moves at both scales and capture the real UI."""
    environment = os.environ.copy()
    # Match production allocator policy before libtest/toolkit threads start.
    environment["MALLOC_MMAP_THRESHOLD_"] = "131072"
    environment["GLIBC_TUNABLES"] = "glibc.malloc.mmap_threshold=131072"
    environment["LD_LIBRARY_PATH"] = environment.get("RESOUL_SNAP_LIBS", "")
    binary = environment["RESOUL_SNAP_BINARY"]
    # KWin starts XWayland lazily. Establish its WM bridge before mapping test windows.
    subprocess.run(["xprop", "-root", "_NET_SUPPORTING_WM_CHECK"], check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1)
    cases = [
        ("playlist_window::placement_tests::native_prepared_startup", "1", "startup-1x.log", False),
        ("playlist_window::placement_tests::native_prepared_startup", "2", "startup-2x.log", False),
        ("themes::tests::native_app_menu_preview", "1", "app-menu-1x.log", False),
        ("themes::tests::native_app_menu_preview", "2", "app-menu-2x.log", False),
        ("analysis_window::tests::native_analysis_journey", "1", "analysis-journey-1x.log", False),
        ("analysis_window::tests::native_analysis_journey", "2", "analysis-journey-2x.log", False),
        ("analysis_window::tests::native_analysis_preview", "1", "analysis-1x.log", False),
        ("analysis_window::tests::native_analysis_preview", "2", "analysis-2x.log", False),
        ("window_snap::drag::tests::native_direct_group_drag", "1", "direct-drag-1x.log", False),
        ("window_snap::drag::tests::native_direct_group_drag", "2", "direct-drag-2x.log", False),
        ("window_snap::native_tests::native_frameless_snap_journey", "1", "frameless-moves-1x.log", False),
        ("window_snap::native_tests::native_frameless_snap_journey", "2", "frameless-moves-2x.log", False),
        ("playlist_window::placement_tests::native_playlist_remap_keeps_position", "1", "playlist-remap.log", False),
        ("playlist_window::placement_tests::native_playlist_remap_keeps_position", "2", "playlist-remap-2x.log", False),
        ("window_group::native_tests::native_task_switcher_journey", "1", "task-switcher.log", False),
        ("playback_edges::tests::native_processing_preview", "1", "processing-1x.log", False),
        ("playback_edges::tests::native_processing_preview", "2", "processing-2x.log", False),
        ("drawer_tests::native_drawer_animation", "1", "drawer-1x.log", False),
        ("drawer_tests::native_drawer_animation", "2", "drawer-2x.log", False),
        ("playlist_window::interaction_tests::native_note_animation", "1", "note-animation.log", False),
        ("window_group::native_tests::native_click_raise_journey", "1", "click-raise.log", False),
        ("window_group::native_tests::native_activation_journey", "1", "activation.log", False),
        ("window_snap::native_tests::native_snap_journey", "1", "native-1x.log", False),
        ("window_snap::native_tests::native_snap_journey", "2", "native-2x.log", False),
        ("playlist_window::tests::native_playlist_preview", "1", "preview.log", False),
        ("playlist_window::tests::native_playlist_preview", "2", "preview-2x.log", False),
        ("playlist_window::tests::native_playlist_preview", "1", "wayland-preview.log", True),
        ("themes::tests::native_theme_preview", "1", "themes.log", False),
        ("themes::tests::native_theme_preview", "1", "themes-wayland.log", True),
        ("file_import::native_tests::native_drop_journey", "1", "drop.log", False),
        ("ui_tests::native_preview", "1", "visual-1x.log", False),
        ("ui_tests::native_preview", "2", "visual-2x.log", False),
        ("live_spectrum::tests::native_motion", "1", "live-spectrum-1x.log", False),
        ("live_spectrum::tests::native_motion", "2", "live-spectrum-2x.log", False),
        ("ui_tests::native_session_journey", "1", "visual-playback.log", False),
        ("ui_tests::native_queue_edit_journey", "1", "visual-queue.log", False),
        ("ui_tests::native_modes_journey", "1", "visual-modes.log", False),
        ("ui_tests::native_equalizer_journey", "1", "visual-eq.log", False),
    ]
    selected = environment.get("RESOUL_WINDOW_CASE")
    if selected:
        names = set(selected.split(","))
        known = {case[0] for case in cases}
        if names - known:
            raise RuntimeError("Unknown isolated window test")
        cases = [case for case in cases if case[0] in names]
    success = True
    for name, scale, log_name, wayland in cases:
        if wayland:
            environment["RESOUL_PREVIEW_WAYLAND"] = "1"
        else:
            environment.pop("RESOUL_PREVIEW_WAYLAND", None)
        environment["WINIT_X11_SCALE_FACTOR"] = scale
        environment["RESOUL_PREVIEW_DIR"] = str(OUTPUT)
        with (OUTPUT / log_name).open("w") as log:
            result = subprocess.run([binary, name, "--exact", "--ignored", "--nocapture"],
                                    env=environment, stdout=log, stderr=subprocess.STDOUT,
                                    timeout=15)
        output = (OUTPUT / log_name).read_text()
        success = success and result.returncode == 0 and "test result: ok. 1 passed; 0 failed;" in output
    if environment.get("RESOUL_VISUAL_APP"):
        measure_idle_app(environment)
    (OUTPUT / "result.json").write_text(json.dumps({"passed": success, "cases": len(cases)}) + "\n")
    return 0 if success else 1


def wait_for_x11_window(process, environment, started):
    """Measure first/all mapped clients of the default two-window app on this owned X server."""
    first_ms = None
    while time.monotonic() - started < 5:
        if process.poll() is not None:
            raise RuntimeError("Player exited before mapping its window")
        clients = subprocess.run(["xprop", "-root", "_NET_CLIENT_LIST"],
                                 env=environment, capture_output=True, text=True,
                                 timeout=1, check=True)
        owned = []
        for window in re.findall(r"0x[0-9a-fA-F]+", clients.stdout):
            owner = subprocess.run(["xprop", "-id", window, "_NET_WM_PID"],
                                   env=environment, capture_output=True, text=True,
                                   timeout=1, check=False)
            if owner.stdout.strip().endswith(f"= {process.pid}"):
                owned.append(window)
        if owned and first_ms is None:
            first_ms = (time.monotonic() - started) * 1000
        if len(owned) == 2:
            return first_ms, (time.monotonic() - started) * 1000
        time.sleep(0.01)
    raise RuntimeError("Player did not map a window within five seconds")


def measure_idle_app(environment):
    """Sample the explicit production binary in this temporary profile, then reap only that child."""
    measurements = {}
    for backend, arguments, scale in (("x11-1x", ["--x11"], "1"),
                                      ("x11-2x", ["--x11"], "2"),
                                      ("wayland", ["--native-display"], "1")):
        environment["WINIT_X11_SCALE_FACTOR"] = scale
        samples = []
        with (OUTPUT / f"idle-{backend}.log").open("w") as log:
            started = time.monotonic()
            process = subprocess.Popen([environment["RESOUL_VISUAL_APP"], *arguments],
                                       env=environment, stdout=log, stderr=subprocess.STDOUT)
            try:
                mapped_ms = None
                all_mapped_ms = None
                if backend.startswith("x11"):
                    mapped_ms, all_mapped_ms = wait_for_x11_window(process, environment, started)
                for _ in range(12):
                    time.sleep(0.25)
                    if process.poll() is not None:
                        raise RuntimeError("Production player exited before idle sampling")
                    status = Path(f"/proc/{process.pid}/status").read_text()
                    for line in status.splitlines():
                        if line.startswith("VmRSS:"):
                            samples.append(int(line.split()[1]) / 1024)
                measurements[backend] = {"peak_rss_mib": max(samples), "settled_rss_mib": samples[-1]}
                if mapped_ms is not None:
                    measurements[backend]["first_mapped_window_ms"] = mapped_ms
                    measurements[backend]["all_mapped_windows_ms"] = all_mapped_ms
            finally:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
    (OUTPUT / "idle-memory.json").write_text(json.dumps(measurements, indent=2) + "\n")
    # This virtual desktop forces software rendering; report it separately from the
    # hardware-backed reference-machine budget rather than treating the workloads as equal.
    for backend, result in measurements.items():
        print(f"MEASURE: isolated software {backend} idle peak RSS {result['peak_rss_mib']:.2f} MiB")


def build_test_binary():
    """Select the executable from Cargo's structured output without depending on its hash suffix."""
    result = subprocess.run(["cargo", "test", "-p", "resolibre-desktop", "--locked", "--no-run",
                             "--message-format=json"], cwd=ROOT, capture_output=True, text=True)
    (OUTPUT / "build.log").write_text(result.stderr)
    if result.returncode != 0:
        raise RuntimeError("Native window test build failed; see the build log")
    for line in result.stdout.splitlines():
        message = json.loads(line)
        if message.get("reason") == "compiler-artifact" and message.get("executable"):
            if message.get("profile", {}).get("test"):
                return message["executable"]
    raise RuntimeError("Cargo did not report the native test executable")


def run_isolated():
    """Own the private display and its process group; never alter the user's display or profile."""
    for program in ("kwin_wayland", "Xwayland", "dbus-run-session", "xprop", "xmessage", "fusermount3"):
        if shutil.which(program) is None:
            raise RuntimeError(f"Native window checks require {program}")
    OUTPUT.mkdir(parents=True, exist_ok=True)
    (OUTPUT / "result.json").unlink(missing_ok=True)
    binary = build_test_binary()
    with tempfile.TemporaryDirectory(prefix="resolibre-window-check-") as temporary:
        private = Path(temporary)
        environment = os.environ.copy()
        for variable, child in (("XDG_CONFIG_HOME", "config"), ("XDG_CACHE_HOME", "cache"),
                                ("XDG_DATA_HOME", "data"), ("XDG_RUNTIME_DIR", "runtime")):
            path = private / child
            path.mkdir(mode=0o700)
            environment[variable] = str(path)
        environment.update(RESOUL_SNAP_CHILD="1", RESOUL_SNAP_BINARY=binary,
                           RESOUL_SNAP_LIBS=environment.get("LD_LIBRARY_PATH", ""),
                           LIBGL_ALWAYS_SOFTWARE="1")
        command = ["dbus-run-session", "--", "kwin_wayland", "--virtual", "--width", "2560",
                   "--height", "1440", "--xwayland", "--no-lockscreen", "--inputmethod",
                   "/usr/bin/true", "--no-global-shortcuts", "--no-kactivities",
                   "--exit-with-session", str(Path(__file__).resolve())]
        with (OUTPUT / "compositor.log").open("w") as log:
            process = subprocess.Popen(command, env=environment, stdout=log,
                                       stderr=subprocess.STDOUT, start_new_session=True)
            try:
                process.wait(timeout=120)
            finally:
                try:
                    os.killpg(process.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                process.wait(timeout=5)
                subprocess.run(["fusermount3", "-u", str(private / "runtime/doc")],
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
    result = OUTPUT / "result.json"
    if not result.exists() or not json.loads(result.read_text()).get("passed"):
        raise RuntimeError("Native window checks failed; see .local/window-snap logs")
    count = json.loads(result.read_text())["cases"]
    print(f"PASS: {count} isolated native window cases")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(run_child() if os.environ.get("RESOUL_SNAP_CHILD") == "1" else run_isolated())
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        sys.exit(1)
