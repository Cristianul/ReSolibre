#!/usr/bin/env python3
"""Validate ReSolibre's local Markdown and complete documented reading flows."""

from pathlib import Path
import os
import re
import sys
from urllib.parse import unquote, urlsplit


ROOT = Path(__file__).resolve().parents[1]
LIMIT_BYTES = 32 * 1024
ARCH = "docs/architecture/"
TARGET = ARCH + "target/"
TEST_PLAN = "docs/testing/test-plan.md"
EXPANDED_TEST_PLAN = "docs/testing/expanded-features-test-plan.md"
LOCAL_PLAYBACK = ARCH + "features/local-playback.md"
LOCAL_DESKTOP = ARCH + "features/local-desktop-player.md"
ROOT_CONTEXT = (
    "AGENTS.md",
    "ARCHITECTURE.md",
    ARCH + "project-map.md",
    ARCH + "current-architecture.md",
)
CORE_CONTEXT = ROOT_CONTEXT + (
    "crates/AGENTS.md",
    ARCH + "core-project-map.md",
)
DESKTOP_CONTEXT = ROOT_CONTEXT + (
    "apps/desktop/AGENTS.md",
    ARCH + "desktop-project-map.md",
)
VERIFICATION = (TARGET + "security-and-privacy.md", TEST_PLAN)

# These are complete task-specific contexts, not arbitrary chunks sized to pass.
# Add required feature leaves/guidance here whenever routing or behavior changes.
READING_FLOWS = {
    "profile-reset": DESKTOP_CONTEXT + VERIFICATION + (
        ARCH + "features/profile-reset.md", ARCH + "features/session-persistence.md",
    ),
    "library-search": CORE_CONTEXT + DESKTOP_CONTEXT + VERIFICATION + (
        ARCH + "features/library-search.md", ARCH + "features/track-metadata.md",
    ),
    "track-metadata": CORE_CONTEXT + DESKTOP_CONTEXT + VERIFICATION + (
        ARCH + "features/track-metadata.md",
    ),
    "product-orientation": ROOT_CONTEXT + (
        "README.md",
        ARCH + "target-architecture-summary.md",
        TARGET + "open-decisions.md",
    ),
    "product-roadmap": ROOT_CONTEXT + (TEST_PLAN,) + (
        "docs/plans/product-roadmap.md",
        TARGET + "delivery-and-licensing.md",
        TARGET + "open-decisions.md",
    ),
    "core-and-platform-contracts": CORE_CONTEXT + VERIFICATION + (
        TARGET + "core-and-platforms.md",
        LOCAL_PLAYBACK,
    ),
    "native-environment": CORE_CONTEXT + VERIFICATION + (
        "docs/development/native-environment.md",
        TARGET + "delivery-and-licensing.md",
    ),
    "local-playback-implementation": CORE_CONTEXT + VERIFICATION + (
        TARGET + "core-and-platforms.md",
        TARGET + "audio-and-equalizer.md",
        LOCAL_PLAYBACK,
    ),
    "local-audio-validation": CORE_CONTEXT + VERIFICATION + (
        LOCAL_PLAYBACK,
        "docs/development/native-environment.md",
        "docs/testing/local-audio-validation.md",
    ),
    "output-adaptation-validation": CORE_CONTEXT + VERIFICATION + (
        LOCAL_PLAYBACK,
        TARGET + "audio-and-equalizer.md",
        "docs/testing/output-adaptation-validation.md",
    ),
    "pcm-conversion-validation": CORE_CONTEXT + VERIFICATION + (
        LOCAL_PLAYBACK,
        TARGET + "audio-and-equalizer.md",
        "docs/plans/implemented/pcm-conversion-plan.md",
        "docs/testing/pcm-conversion-validation.md",
    ),
    "output-failure-validation": CORE_CONTEXT + VERIFICATION + (
        LOCAL_PLAYBACK,
        TARGET + "audio-and-equalizer.md",
        "docs/plans/implemented/output-failure-plan.md",
        "docs/testing/output-failure-validation.md",
    ),
    "playback-responsiveness-history": CORE_CONTEXT + VERIFICATION + (
        LOCAL_PLAYBACK,
        "docs/plans/implemented/playback-responsiveness-plan.md",
    ),
    "playback-responsiveness-validation": CORE_CONTEXT + VERIFICATION + (
        LOCAL_PLAYBACK,
        "docs/testing/playback-responsiveness-validation.md",
    ),
    "named-eq-presets": CORE_CONTEXT + DESKTOP_CONTEXT + VERIFICATION + (
        ARCH + "features/named-eq-presets.md",

    ),
    "equalizer": CORE_CONTEXT + DESKTOP_CONTEXT + VERIFICATION + (
        "docs/plans/implemented/equalizer-plan.md", ARCH + "features/local-equalizer.md",
    ),
    "player-appearance": DESKTOP_CONTEXT + VERIFICATION + (
        ARCH + "features/player-appearance.md", "docs/design/compact-player.md",
        "docs/testing/player-appearance-validation.md",
    ),
    # Historical execution decisions are optional when changing the current appearance.
    "player-appearance-history": DESKTOP_CONTEXT + VERIFICATION + (
        ARCH + "features/player-appearance.md", "docs/design/compact-player.md",
        "docs/plans/implemented/player-visual-polish-plan.md",
    ),
    "detachable-playlist": DESKTOP_CONTEXT + VERIFICATION + (
        ARCH + "features/detachable-playlist.md", TARGET + "interface-and-themes.md",
        "docs/testing/detachable-playlist-validation.md",
    ),
    "playlist-interactions": CORE_CONTEXT + DESKTOP_CONTEXT + VERIFICATION + (
        ARCH + "features/playlist-interactions.md",
    ),
    "playback-edges": CORE_CONTEXT + DESKTOP_CONTEXT + VERIFICATION + (
        ARCH + "features/playback-edges.md",
    ),
    "transport-scan": CORE_CONTEXT + DESKTOP_CONTEXT + VERIFICATION + (
        ARCH + "features/transport-scan.md", TARGET + "audio-and-equalizer.md",
    ),
    "session-persistence": CORE_CONTEXT + DESKTOP_CONTEXT + VERIFICATION + (
        ARCH + "features/session-persistence.md",
    ),
    "shuffle-repeat": CORE_CONTEXT + DESKTOP_CONTEXT + VERIFICATION + (
        "docs/plans/implemented/shuffle-repeat-plan.md",
        ARCH + "features/queue-playback-order.md",
    ),
    "local-file-import": CORE_CONTEXT + DESKTOP_CONTEXT + VERIFICATION + (
        ARCH + "features/local-file-import.md",
        "docs/testing/local-file-import-validation.md",
        TARGET + "local-file-import.md",
    ),
    "local-desktop-ui": CORE_CONTEXT + DESKTOP_CONTEXT + VERIFICATION + (
        LOCAL_DESKTOP, "docs/design/compact-player.md",
    ),
    "local-desktop-validation": CORE_CONTEXT + DESKTOP_CONTEXT + VERIFICATION + (
        LOCAL_DESKTOP, "docs/testing/local-desktop-validation.md",
    ),
    # Historical measurement protocol is self-contained; current contracts use the flow above.
    "local-desktop-baseline": CORE_CONTEXT + DESKTOP_CONTEXT + VERIFICATION + (
        "docs/testing/local-desktop-baseline.md",
    ),
    "desktop-environment": CORE_CONTEXT + DESKTOP_CONTEXT + VERIFICATION + (
        "docs/development/native-environment.md",
        "docs/development/desktop-environment.md",
    ),
    "local-desktop-player-plan": CORE_CONTEXT + DESKTOP_CONTEXT + VERIFICATION + (
        "docs/plans/local-desktop-player-plan.md",
        "docs/design/compact-player.md",
    ),
    "smb-playback-plan": CORE_CONTEXT + VERIFICATION + (
        TARGET + "library-and-sources.md",
        "docs/plans/smb-playback-plan.md",
    ),
    "playback-local-and-smb": CORE_CONTEXT + VERIFICATION + (
        LOCAL_PLAYBACK,
        TARGET + "audio-and-equalizer.md",
        TARGET + "library-and-sources.md",
    ),
    "catalog-search-and-scanning": CORE_CONTEXT + VERIFICATION + (
        TARGET + "library-and-sources.md",
        TARGET + "library-and-playlists.md",
    ),
    "native-and-service-playlists": CORE_CONTEXT + VERIFICATION + (
        TARGET + "library-and-playlists.md",
        EXPANDED_TEST_PLAN,
    ),
    "audio-processing": CORE_CONTEXT + VERIFICATION + (
        TARGET + "audio-and-equalizer.md",
        TARGET + "audio-processing.md",
        EXPANDED_TEST_PLAN,
    ),
    "track-analysis-plan": CORE_CONTEXT + DESKTOP_CONTEXT + VERIFICATION + (
        "docs/plans/track-analysis-plan.md",
    ),
    "track-information": CORE_CONTEXT + DESKTOP_CONTEXT + VERIFICATION + (
        ARCH + "features/track-information.md",
    ),
    "dj-mix": CORE_CONTEXT + DESKTOP_CONTEXT + VERIFICATION + (
        ARCH + "features/dj-mix.md",
    ),
    "dj-mix-validation": CORE_CONTEXT + VERIFICATION + (
        ARCH + "features/dj-mix.md", "docs/testing/mix-validation.md",
    ),
    "library-analysis": CORE_CONTEXT + DESKTOP_CONTEXT + VERIFICATION + (
        ARCH + "features/library-analysis.md",
    ),
    "analysis-cache": CORE_CONTEXT + DESKTOP_CONTEXT + VERIFICATION + (
        ARCH + "features/analysis-cache.md",
    ),
    "track-analysis": CORE_CONTEXT + DESKTOP_CONTEXT + VERIFICATION + (
        ARCH + "features/track-analysis.md",
    ),
    "spectral-analysis": CORE_CONTEXT + VERIFICATION + (
        ARCH + "features/spectral-analysis.md",
    ),
    "live-spectrum": CORE_CONTEXT + DESKTOP_CONTEXT + VERIFICATION + (
        ARCH + "features/live-spectrum.md",
        "crates/resolibre-audio/native/ladspa/README.md",
    ),
    "live-tap-validation": CORE_CONTEXT + VERIFICATION + (
        "docs/testing/live-tap-validation.md",
        "docs/development/native-environment.md",
    ),
    "analysis-and-radio": CORE_CONTEXT + VERIFICATION + (
        TARGET + "analysis-and-radio.md",
        EXPANDED_TEST_PLAN,
    ),
    "metadata-editing": CORE_CONTEXT + VERIFICATION + (
        TARGET + "library-and-sources.md",
        TARGET + "metadata-editing.md",
        EXPANDED_TEST_PLAN,
    ),
    "desktop-layouts-and-skins": DESKTOP_CONTEXT + VERIFICATION + (
        TARGET + "interface-and-themes.md",
        TARGET + "audio-and-equalizer.md",
    ),
    "cross-area-session-and-eq": CORE_CONTEXT + DESKTOP_CONTEXT + VERIFICATION + (
        TARGET + "audio-and-equalizer.md",
        TARGET + "interface-and-themes.md",
        ARCH + "features/README.md",
    ),
    # Future desktop/source target planning; current local code uses the profile above.
    "planned-desktop-and-smb-playback": CORE_CONTEXT + DESKTOP_CONTEXT + VERIFICATION + (
        TARGET + "audio-and-equalizer.md",
        TARGET + "library-and-sources.md",
    ),
    "future-device-control": CORE_CONTEXT + VERIFICATION + (
        TARGET + "core-and-platforms.md",
        TARGET + "remote-control.md",
    ),
    "connected-client-validation": CORE_CONTEXT + VERIFICATION + (
        TARGET + "remote-control.md",
        EXPANDED_TEST_PLAN,
    ),
    "delivery-and-decisions": ROOT_CONTEXT + (TEST_PLAN,) + (
        TARGET + "delivery-and-licensing.md",
        TARGET + "open-decisions.md",
    ),
    "feature-documentation-maintenance": ROOT_CONTEXT + (TEST_PLAN,) + (
        ARCH + "features/README.md",
        ARCH + "target-architecture-summary.md",
    ),
}


def collect_documents():
    """Collect authored Markdown from the documented areas, excluding outputs."""
    documents = set(ROOT.glob("*.md"))
    excluded = {"target", "build", "dist", "node_modules", ".git", ".local", "tmp"}
    for area in ("docs", "apps", "crates"):
        directory = ROOT / area
        if not directory.exists():
            continue
        for current, directories, filenames in os.walk(directory):
            # Architecture target/ is authored documentation, not Cargo output.
            if area != "docs":
                directories[:] = [name for name in directories if name not in excluded]
            for name in filenames:
                if name.endswith(".md"):
                    documents.add(Path(current) / name)
    return sorted(path.resolve() for path in documents)


def check_documents(documents):
    """Check readable UTF-8, local inline links and unexpanded template fields."""
    errors = []
    for path in documents:
        relative = path.relative_to(ROOT)
        try:
            content = path.read_bytes().decode("utf-8")
        except UnicodeError:
            errors.append(f"{relative}: invalid UTF-8")
            continue
        if re.search(r"\{\{[A-Z][A-Z0-9_]*\}\}", content):
            errors.append(f"{relative}: unexpanded template placeholder")
        # Authored docs use inline links; ignore illustrative fenced code.
        prose = re.sub(r"```.*?```", "", content, flags=re.DOTALL)
        for destination in re.findall(r"\[[^\]]*\]\(([^)]+)\)", prose):
            url = urlsplit(destination.strip().strip("<>"))
            if url.scheme or url.netloc or not url.path:
                continue
            target = (path.parent / unquote(url.path)).resolve()
            if not target.is_relative_to(ROOT):
                errors.append(f"{relative}: local link leaves repository: {destination}")
            elif not target.exists():
                errors.append(f"{relative}: missing local link: {destination}")
    return errors


def check_reading_flows(documents):
    """Count deduplicated UTF-8 bytes including guidance, routing and topics."""
    errors = []
    covered = set()
    for name, entries in READING_FLOWS.items():
        paths = {(ROOT / entry).resolve() for entry in entries}
        missing = sorted(path for path in paths if not path.is_file())
        if missing:
            errors.append(f"{name}: missing flow document(s)")
            continue
        covered.update(paths)
        size = sum(path.stat().st_size for path in paths)
        print(f"{name}: {size:,} / {LIMIT_BYTES:,} bytes ({len(paths)} files)")
        if size > LIMIT_BYTES:
            errors.append(f"{name}: exceeds reading-flow budget by {size - LIMIT_BYTES} bytes")
    for path in documents:
        if path not in covered:
            errors.append(f"{path.relative_to(ROOT)}: no complete reading-flow profile")
    return errors


def main():
    """Run documentation checks and fail visibly when the contract is violated."""
    documents = collect_documents()
    errors = check_documents(documents)
    errors.extend(check_reading_flows(documents))
    summary = ROOT / ARCH / "target-architecture-summary.md"
    if summary.is_file():
        size = summary.stat().st_size
        print(f"target summary: {size:,} bytes (target 3,000–5,120)")
        if not 3000 <= size <= 5120:
            errors.append("target summary: move detail to focused docs; target is 3–5 KB")
    else:
        errors.append("missing target architecture summary")
    if errors:
        print("\nDocumentation validation failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    print(f"PASS: {len(documents)} Markdown files; all reading flows fit within 32 KiB.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
