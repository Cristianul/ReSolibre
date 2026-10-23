#!/usr/bin/env python3
"""Install this checkout's built debug player and brand in the current user's app menu."""

import os
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def desktop_string(value):
    """Escape a literal desktop-entry string without introducing extra keys or lines."""
    return str(value).replace("\\", "\\\\").replace("\n", "\\n").replace("\r", "\\r")


def exec_argument(value):
    """Quote one Exec argument across desktop-string and command parsing; never use a shell."""
    quoted = str(value).replace("%", "%%")
    for character in ("\\", '"', "`", "$"):
        quoted = quoted.replace(character, "\\" + character)
    return desktop_string('"' + quoted + '"')


def main():
    """Register only ReSolibre-owned files; require an existing binary and absolute XDG root."""
    binary = ROOT / "target/debug/resolibre-desktop"
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise SystemExit("Build resolibre-desktop before installing its launcher")

    data = Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local/share")
    if not data.is_absolute():
        raise SystemExit("XDG_DATA_HOME must be absolute")

    icon = data / "icons/hicolor/256x256/apps/resolibre.png"
    entry = data / "applications/resolibre.desktop"
    for destination in (icon, entry):
        if destination.is_symlink():
            raise SystemExit("Refusing to replace a symlink at a ReSolibre install destination")
        destination.parent.mkdir(parents=True, exist_ok=True)

    arguments = [str(binary)]
    libraries = ROOT / ".local/native/usr/lib64"
    if libraries.is_dir():
        arguments = ["/usr/bin/env", f"LD_LIBRARY_PATH={libraries}", *arguments]
    command = " ".join(exec_argument(argument) for argument in arguments)
    contents = (
        "[Desktop Entry]\nType=Application\nName=ReSolibre\n"
        "GenericName=Music Player\nComment=Play your local music\n"
        f"Exec={command}\nIcon={desktop_string(icon)}\n"
        "Terminal=false\nCategories=AudioVideo;Audio;Player;\n"
        "StartupWMClass=resolibre\nStartupNotify=true\n"
    )
    shutil.copyfile(ROOT / "apps/desktop/ui/assets/resolibre.png", icon)
    entry.write_text(contents, encoding="utf-8")
    if shutil.which("update-desktop-database"):
        subprocess.run(["update-desktop-database", str(entry.parent)], check=True)
    print("Installed ReSolibre in the current user's application menu")


if __name__ == "__main__":
    main()
