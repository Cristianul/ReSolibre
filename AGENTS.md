# ReSolibre agent rules

Lightweight personal music player; currently a local desktop/audio prototype.

## Focused work

Read root and affected [core](crates/AGENTS.md)/[desktop](apps/desktop/AGENTS.md)
rules, then source/tests. Cross-area/architecture work uses the
[map](docs/architecture/project-map.md), area map and focused topics. Prefer `rg`;
no full-repo scans or doc preloads. Preserve user work; external repos are read-only
unless explicitly authorized.

## Readable, debuggable code

- Write simple, spacious code a Java developer new to Rust can follow. Use
  descriptive action names, named intermediate values, explicit branches,
  guard clauses and short functions with one purpose. Separate logical steps
  with blank lines; keep nesting shallow and responsibilities organized.
- Prefer straightforward control flow over dense chains, clever tricks,
  speculative abstractions or large functions. Keep idiomatic Rust and the
  formatter; explain necessary complexity rather than hiding it.
- Document every handwritten function/method above its definition (`///` in
  Rust; language equivalents such as Python docstrings). Explain purpose and,
  where relevant, results, effects, failure and ownership expectations.
  Inside functions, explain non-obvious decisions/invariants, not each line.
  Keep comments accurate, concise and separated from surrounding steps.
- For bugs, reproduce or gather evidence, trace the root cause, then make the
  smallest complete fix with a relevant regression check. Avoid symptom-only
  patches and unrelated refactors. New features may change all necessary areas.
- Design for security, reliability, performance and stability from the start:
  explicit errors, recovery, resource bounds and observable failure paths.

## Reusable features

- Give each feature one owning module, explicit API and authoritative state.
  Reuse behavior, validation and configuration; find existing code first.
  Views, menus, hotkeys and future remotes dispatch the same typed actions.
  Define shortcut mappings once; OS registration belongs in adapters.
- Keep modules cohesive; extract real reuse without speculative traits/crates/
  services. Test shared behavior at its owner and consumer wiring separately.

## Invariants

- Normal import/scan/playback/UI NEVER writes, retags, renames or deletes music.
  Store application data separately. Future writes require a distinct explicit
  workflow and confirmation before saving/deleting.
- Prioritize actual audio/control responsiveness, bounded memory and playback
  during scans. Separate UI, portable core and native/platform adapters.
- Adapt output to supported devices; do not reject playable music for purity.
- Validate untrusted media, tags, art, skins, network and stored data. Protect
  secrets/private content and source/profile/device isolation.

## Verify

EVERY change assesses direct tests, connected regressions and security: trust,
access, isolation, inputs, secrets/private data, caches, dependencies and abuse/
replay. Add relevant negative tests per the [test plan](docs/testing/test-plan.md).
Docs-only work uses consistency checks. Report executed checks and gaps;
never claim planned code/tests/benchmarks exist.

Log failures once where handled: error=failure, warn=recoverable issue,
info=meaningful lifecycle, debug=routine, trace=high-volume diagnostics. Exclude
secrets, private paths/URLs, raw payloads and unfiltered native diagnostics.

## Maintain documentation

- Update docs/maps with behavior/structure changes. Current/feature docs describe
  implementation; focused targets describe intent.
- Archive completed plans in `docs/plans/implemented/` in the same task with
  status/evidence and updated links/profiles. Incomplete/blocked plans stay active.
- Root maps route; area maps navigate; target summary: 3–5 KB.
- EVERY complete reading flow: <=32,768 UTF-8 bytes, deduplicated, including
  applicable AGENTS, routing and topics.
- Split/reroute oversized flows without dropping required context. Update
  profiles and run `python3 tools/check_docs.py` after docs/routing changes.
- Use English Markdown/Mermaid. Feature leaves: Read When, Do Not Read For,
  Current Behavior, Key Files, Tests, Edge Cases, Related Docs.
- For target changes or unresolved boundaries, explain options and obtain a
  decision unless already authorized; update the focused target. Routine
  approved work proceeds.

## Automatically track new files

In the same task, `git add` new source/tests/docs/config/scripts/lockfiles/assets
using explicit reviewed paths. Exclude temporary files, logs, secrets, runtime/IDE/
workstation data, build output and unrelated changes. No `git add .`; no commit/
push without authorization. Check staging and ignores before reporting.

## Commit messages

When authorized, use Conventional Commits: `<type>[optional scope][!]: <summary>`.
Describe staged outcomes in plain language; body: what/why, checks and limits.
Explain jargon, distinguish planned from working features, omit conversation and
abandoned plans. Relevant jokes are welcome, except around failures or security.
