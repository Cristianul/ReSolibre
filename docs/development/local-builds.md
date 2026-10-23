# Local desktop builds

## Read When

Building the current player locally for a debugger or optimized use.

## Do Not Read For

OS porting, installers or distribution packaging. Use the
[delivery target](../architecture/target/delivery-and-licensing.md) for those decisions.

## Current Behavior

On Linux, from the checkout root, with Python 3.9+, Rust/Cargo, a C11 compiler, pkg-config,
libmpv development files and Slint's Linux native dependencies installed:

```sh
python3 build_debug.py
python3 build_release.py
```

The scripts locate the checkout themselves, so invoking their absolute paths
from another directory also works. They detect the OS and architecture before
building. The desktop/audio adapter supports Linux x86_64 and Windows x86_64
MSVC. macOS and other architectures get an unsupported-platform error.
Each build explicitly selects the installed
Rust compiler's host target and preserves Cargo.lock with `--locked`.

On Windows use `./build_windows.ps1 -Profile Debug` or `-Profile Release`.
Optional switches are `-Offline`, `-Jobs N`, `-Run`, and `-Python <executable>`.
The Python entries also work; Windows outputs have the `.exe` suffix.
See [Windows prerequisites, runtime staging and validation](windows-native.md).

Debug uses Cargo's development profile with debugger symbols and assertions;
the existing core/RustFFT optimization keeps interactive DSP usable. Release
uses optimization level 3, thin link-time optimization, one code-generation unit
and removed debug information. Normal Rust panic unwinding is retained.
These are local optimized builds, not audited distribution packages.
See [Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html).

Both entry points accept:

- `--offline`: use cached Cargo dependencies without network access.
- `--jobs N` or `-j N`: bound parallel compilation.
- `--run`: launch only after a successful build, with the same native library environment.

```sh
python3 build_debug.py --offline --jobs 4 --run
python3 build_release.py --offline --jobs 4
```

With default Cargo settings, executables are at
`target/<rust-host>/debug/resolibre-desktop` and
`target/<rust-host>/release/resolibre-desktop`. The scripts print the actual
executable path, honoring custom Cargo output directories. They do not replace
the separate app-menu installer's `target/debug` executable.

On Linux, existing `.local/native/usr/lib64` or `lib` directories are added to compiler,
loader and pkg-config search paths for the child processes only. User flags and
paths are retained; paths containing spaces are supported. No libraries are
downloaded or installed by the scripts, and no profile or music data is touched
unless the caller explicitly requests `--run` and then uses the player.

## Key Files

- [Debug](../../build_debug.py) and [release](../../build_release.py) entry points.
- [Shared build setup](../../tools/build_support.py); [profiles](../../Cargo.toml).

## Tests

`python3 -m unittest discover -s tools -p 'test_build_support.py' -v` covers host
rejection, missing dependencies, compiler mismatch, argument/path preservation,
custom output directories, both real entry points with fake tools and failure
propagation. On the Fedora x86_64 reference, both real builds passed on
2026-10-23; ELF inspection confirmed debug information only in the debug build,
and both binaries resolved their runtime libraries. Windows debug/release builds
and the actual PowerShell wrapper were checked on 2026-10-24; see the
[Windows evidence and limitations](windows-native.md). No macOS build is claimed.

## Edge Cases

Missing tools/development libraries fail before the Rust build. Failed builds
return a nonzero status and never launch an older executable. `--offline` fails
when required crates are not cached. Cargo's host target is selected explicitly,
even if `CARGO_BUILD_TARGET` requests a different platform. Build success does
not establish playback/device quality or support for another Linux distribution.

## Related Docs

[Native setup](native-environment.md), [desktop setup](desktop-environment.md),
[test plan](../testing/test-plan.md).
