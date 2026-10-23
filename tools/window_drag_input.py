#!/usr/bin/env python3
"""Deliver one pointer drag inside check_window_snap.py's private KWin compositor.

Protocol: KDE plasma-wayland-protocols/src/protocols/fake-input.xml.
This test-only helper uses the installed Wayland library, not production code.
"""

import ctypes as ffi
import os
from pathlib import Path
import sys
import time


class Message(ffi.Structure):
    """Wayland request descriptor ABI."""

    _fields_ = [("name", ffi.c_char_p), ("signature", ffi.c_char_p), ("types", ffi.c_void_p)]


class Interface(ffi.Structure):
    """Wayland interface descriptor ABI."""

    _fields_ = [
        ("name", ffi.c_char_p), ("version", ffi.c_int),
        ("count", ffi.c_int), ("methods", ffi.POINTER(Message)),
        ("events", ffi.c_int), ("event_types", ffi.c_void_p),
    ]


def load_client():
    """Declare pointer-bearing ABI arguments before calling the native library."""
    client = ffi.CDLL("libwayland-client.so.0")
    client.wl_display_connect.argtypes = [ffi.c_char_p]
    client.wl_display_connect.restype = ffi.c_void_p
    client.wl_display_roundtrip.argtypes = [ffi.c_void_p]
    client.wl_display_disconnect.argtypes = [ffi.c_void_p]
    client.wl_proxy_marshal_flags.restype = ffi.c_void_p
    client.wl_proxy_marshal_flags.argtypes = [
        ffi.c_void_p, ffi.c_uint, ffi.POINTER(Interface), ffi.c_uint, ffi.c_uint,
    ]
    client.wl_proxy_add_listener.argtypes = [ffi.c_void_p, ffi.c_void_p, ffi.c_void_p]
    return client


def run():
    """Bind a private test device, deliver a held diagonal drag, then disconnect."""
    runtime = Path(os.environ.get("XDG_RUNTIME_DIR", ""))
    if (os.environ.get("RESOLIBRE_SNAP_CHILD") != "1"
            or not runtime.parent.name.startswith("resolibre-window-check-")):
        raise SystemExit("Run only through the isolated window-check harness")

    client = load_client()
    display = client.wl_display_connect(None)
    if not display:
        raise RuntimeError("Private compositor connection failed")

    # Keep descriptors and callbacks alive until the connection is closed.
    signatures = [
        (b"authenticate", b"ss"), (b"pointer_motion", b"ff"), (b"button", b"uu"),
        (b"axis", b"uf"), (b"touch_down", b"2uff"), (b"touch_motion", b"2uff"),
        (b"touch_up", b"2u"), (b"touch_cancel", b"2"), (b"touch_frame", b"2"),
        (b"pointer_motion_absolute", b"3ff"),
    ]
    methods = (Message * len(signatures))(
        *[Message(name, signature, None) for name, signature in signatures]
    )
    interface = Interface(b"org_kde_kwin_fake_input", 3, len(methods), methods, 0, None)
    registry_interface = Interface.in_dll(client, "wl_registry_interface")
    registry = client.wl_proxy_marshal_flags(display, 1, ffi.byref(registry_interface), 1, 0, None)
    devices = []

    @ffi.CFUNCTYPE(None, ffi.c_void_p, ffi.c_void_p, ffi.c_uint, ffi.c_char_p, ffi.c_uint)
    def bind_device(data, registry, name, offered_interface, version):
        """Bind only fake input with absolute-motion support."""
        if offered_interface == interface.name and version >= 3:
            device = client.wl_proxy_marshal_flags(
                registry, 0, ffi.byref(interface), 3, 0,
                ffi.c_uint(name), offered_interface, ffi.c_uint(3), None,
            )
            devices.append(device)

    @ffi.CFUNCTYPE(None, ffi.c_void_p, ffi.c_void_p, ffi.c_uint)
    def ignore_removal(data, registry, name):
        """Ignore global removal during this short, harness-bounded connection."""

    listeners = (ffi.c_void_p * 2)(
        ffi.cast(bind_device, ffi.c_void_p), ffi.cast(ignore_removal, ffi.c_void_p),
    )
    client.wl_proxy_add_listener(registry, listeners, None)
    client.wl_display_roundtrip(display)
    if not devices:
        raise RuntimeError("Private KWin fake-input interface unavailable")
    device = devices[0]
    client.wl_proxy_marshal_flags(
        device, 0, None, 3, 0, b"ReSolibre isolated check", b"Pointer gesture regression",
    )

    def emit(opcode, *arguments):
        """Flush one request and allow UI processing before the next sample."""
        client.wl_proxy_marshal_flags(
            device, opcode, None, 3, 0, *[ffi.c_int(value) for value in arguments],
        )
        if client.wl_display_roundtrip(display) < 0:
            raise RuntimeError("Private compositor disconnected")
        time.sleep(0.04)

    x, y = map(int, sys.argv[1:])
    try:
        emit(9, x * 256, y * 256)  # Wayland fixed-point coordinates.
        emit(2, 272, 1)  # Linux BTN_LEFT.
        time.sleep(0.7)  # The native test checks activation before any movement.
        for offset in range(5, 51, 5):
            emit(9, (x + offset) * 256, (y + offset) * 256)
    finally:
        emit(2, 272, 0)
        client.wl_display_disconnect(display)


if __name__ == "__main__":
    run()
