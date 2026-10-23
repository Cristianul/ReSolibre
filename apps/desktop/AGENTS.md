# Desktop rules

Follow [root rules](../../AGENTS.md) and the
[desktop map](../../docs/architecture/desktop-project-map.md). A local Slint UI exists;
full skin, accessibility and performance feasibility remains to be validated.

## Shared presentation

- Desktop owns composition, focus, selections and drafts; core owns playback,
  queue, sources and EQ. Both layouts use one session and preserve it on switch.
- Views, menus and hotkeys call shared feature actions/configuration; keep only
  presentation and platform input handling here.
- Build shared button, slider, input, list, overlay/dialog and Icon primitives
  before feature-local widgets. Wrap third-party controls; use explicit values/
  events. Domain state does not live in presentation bindings.
- Reuse semantic tokens for color, spacing, sizes, borders, typography and
  motion. Keep errors/warnings distinct; buttons have no focus ring.
- Use Lucide control icons through Icon, with license/size/stroke tokens;
  user-approved branding uses BrandLogo. Imported skin sprites remain
  original compatibility artwork, including their bitmap-font appearance.
- Allow reference-matched material shading in the built-in hardware skin.
  Keep state indicators solid; no edge glows.

## Usability and resource bounds

- Virtualize long lists; bound artwork/cache memory. Keep UI feedback separate
  from confirmed audio state. Show buffering/errors honestly.
- Never block UI on scanning, network or decoding. Cancel/ignore stale work;
  release subscriptions, timers and resources when views are replaced.
- Preserve keyboard order/actions, names, focus restoration, slider increments
  and IME. Check resizing, DPI and Unicode.
- Respect reduced motion; avoid idle animation. Classic sprite controls need
  supported scaling and accessible semantics independent of bitmap text.
- Validate imported packages before applying them. Follow the theme/security
  targets for archive bounds and script prohibition; retain a usable fallback.
- Render metadata as text. Info is initially read-only; future edits remain
  drafts until explicit confirmed save.

## Tests and maintenance

Use the [test plan](../../docs/testing/test-plan.md): real desktop journeys,
component behavior, layout changes during playback, accessibility, theme
failure/stale-work cases, connected regressions and security risks.
Update docs/maps in the same task; explicitly track new relevant files and
run `python3 tools/check_docs.py`, counting both guidance files within 32 KiB.
Future web/mobile get scoped rules when created; a React/TypeScript client must
apply the complete white-label frontend template, including API/storage
boundaries, controlled inputs, responsive checks and real-browser journeys.
