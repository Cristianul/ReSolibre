"""Smoke-test a built Windows GUI with an isolated profile and a bounded graceful close."""

import argparse
import ctypes
from ctypes import wintypes
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time


def main():
    """Start only the supplied executable, inspect its own windows, then close and reap it."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("executable", type=Path)
    options = parser.parse_args()
    if sys.platform != "win32":
        parser.error("This check requires Windows")
    executable = options.executable.resolve(strict=True)
    user32 = ctypes.WinDLL("user32", use_last_error=True)
    callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    user32.EnumWindows.argtypes = [callback_type, wintypes.LPARAM]
    user32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
    user32.GetWindowTextW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
    user32.PostMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
    user32.IsWindowVisible.argtypes = [wintypes.HWND]

    with tempfile.TemporaryDirectory(prefix="resolibre-windows-startup-") as temporary:
        environment = dict(os.environ, LOCALAPPDATA=temporary)
        startup = subprocess.STARTUPINFO()
        startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
        startup.wShowWindow = 0
        with tempfile.TemporaryFile() as diagnostics:
            child = subprocess.Popen([str(executable), "--native-display"], cwd=temporary,
                                     env=environment, stdin=subprocess.DEVNULL, stdout=diagnostics,
                                     stderr=diagnostics, startupinfo=startup,
                                     creationflags=subprocess.CREATE_NO_WINDOW)
            try:
                deadline = time.monotonic() + 20
                main_window = None
                while main_window is None and child.poll() is None and time.monotonic() < deadline:
                    windows = []

                    @callback_type
                    def inspect_window(handle, _):
                        """Read titles only for the owned child's handles; do not inspect other applications."""
                        process_id = wintypes.DWORD()
                        user32.GetWindowThreadProcessId(handle, ctypes.byref(process_id))
                        if process_id.value == child.pid:
                            title = ctypes.create_unicode_buffer(128)
                            user32.GetWindowTextW(handle, title, 128)
                            if title.value == "ReSolibre" and user32.IsWindowVisible(handle):
                                windows.append(handle)
                        return True

                    user32.EnumWindows(inspect_window, 0)
                    if windows:
                        main_window = windows[0]
                    else:
                        time.sleep(0.05)
                if main_window is None:
                    raise RuntimeError("The player did not create its main window before the deadline")
                # WM_CLOSE exercises production shutdown without global input injection.
                if not user32.PostMessageW(main_window, 0x0010, 0, 0):
                    raise RuntimeError("Cannot close the owned player window")
                if child.wait(timeout=15) != 0:
                    raise RuntimeError("The player failed during startup or shutdown")
                profile = Path(temporary) / "ReSolibre"
                if not all((profile / name).is_file() for name in ("session.lock", "profile.lock")):
                    raise RuntimeError("The player did not use its isolated profile")
                diagnostics.seek(0)
                output = diagnostics.read(65537)
                if output:
                    raise RuntimeError("The player emitted startup/shutdown diagnostics; inspect the native environment")
                print("PASS: native window startup, isolated profile, graceful shutdown")
            finally:
                if child.poll() is None:
                    child.kill()
                child.wait()


if __name__ == "__main__":
    main()
