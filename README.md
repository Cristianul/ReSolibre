# ReSolibre

A personal music player for people who want their own music, a responsive
interface and control over how it sounds.

**Status: Linux and Windows local desktop prototype with transport, seek, app volume, separate queue, Shuffle/Repeat and ten-band EQ with presets and session restoration.**

## Product direction

- Linux first: Fedora, Debian and Ubuntu; native Windows x64 builds are available as a prototype.
- Local player features/library first; then Jellyfin, SMB and Navidrome.
- Compact Winamp-inspired and modern library layouts sharing one playback
  session, queue and equalizer.
- ReSolibre skin packages and classic Winamp `.wsz` compatibility near the end.
- An equalizer in the first release of each eventual client: desktop, mobile
  and web. Device-compatible audio with the best practical fidelity.
- Hundreds of thousands of tracks, with one million catalog entries as a
  stress scenario. Fast controls and bounded resource use are design inputs.
- Later: account-free LAN pairing to control a PC from mobile/web; internet
  connectivity is a separate subsequent phase.

Offline-first, usable without a ReSolibre account or companion server. Commercial
success is not a requirement. Reuse shared behavior across clients and isolate
only necessary platform differences. Planned additions include one normalization
toggle for all tracks, album-independent crossfade, native
playlists/M3U reevaluated after Jellyfin, and useful audio diagnostics. Beat sync,
analysis/radio and connected features have later stages, not first-release promises.

Ordinary use never modifies original music files. Future tag editing and
deletion will be explicit, confirmed workflows.

## Start here

- [Architecture index](ARCHITECTURE.md)
- [Current state](docs/architecture/current-architecture.md)
- [Target summary](docs/architecture/target-architecture-summary.md)
- [Product roadmap and requirement status](docs/plans/product-roadmap.md)
- [Local desktop setup and controls](docs/development/desktop-environment.md)
- [Local player plan and acceptance](docs/plans/local-desktop-player-plan.md)
- [Project map](docs/architecture/project-map.md)
- [Test and performance plan](docs/testing/test-plan.md)
- [Agent/contributor rules](AGENTS.md)

The local player uses Rust, libmpv and Slint; libsmb2 and SQLite are later adapters.
Full UI/skin feasibility remains open. Core builds without GUI/native dependencies;
the desktop dependencies are pinned in Cargo.lock. Ordinary use leaves music intact.

## Local builds

```sh
python3 build_debug.py
python3 build_release.py
```

Add `--run` to launch after building. See [local build options and OS support](docs/development/local-builds.md).

Windows: `./build_windows.ps1 -Profile Release -Run`.
See [Windows prerequisites and known limits](docs/development/windows-native.md).

## Contributing

Read the affected area's rules and focused documents before implementation.
Keep changes reviewable; add relevant tests and update navigation/behavior
docs in the same change. VSCodium and IntelliJ are supported contributor
preferences; editor choice is not part of the runtime architecture.

Validate documentation with Python 3.9 or newer:

```sh
python3 tools/check_docs.py
git diff --check
git diff --cached --check
```

For prerequisites, build/run commands and measured limitations, read the
[local playback prototype](docs/architecture/features/local-playback.md).
Core-only tests need no libmpv: `cargo test -p resolibre-core --offline`.

## License

ReSolibre is licensed under **GPL-3.0-only**; see [LICENSE](LICENSE).
Official ReSolibre releases are intended to be free of charge. Third-party
dependencies and imported user skins retain their respective licenses.
Compatibility with an external skin format does not grant redistribution
rights to the skin artwork.
