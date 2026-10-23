# Open decisions

Only unresolved choices belong here. Resolve them with focused evidence,
record the decision in its owning target document and update this list.
Do not reopen accepted direction merely because an implementation is hard.

| Decision | Current position | Evidence / decision trigger |
| --- | --- | --- |
| Native playback feasibility | libmpv first; source-owned SMB access | Local/SMB seek, cancellation, EQ, gapless and resource measurements |
| Crossfade engine integration | Desired on any album; one session with bounded overlap | Separate prototype for mixing, format conversion, controls and next-source failure |
| Normalization numbers/headroom | Global ON/OFF, independent track loudness, no album mode | Target level, peak margin, EQ/mix safety and silence fixtures |
| Beat sync | Desired later; ordinary crossfade fallback | Detector confidence, slow/variable rhythm, tempo limits and stretching quality |
| Desktop toolkit/renderer | Slint first candidate; Qt alternative | Actual classic skin, modern list, Unicode, accessibility, scaling, idle/startup tests |
| Dependency/toolchain versions | Local reference recorded; support matrix not pinned | Reproducible Linux build, support/advisory/license review |
| Device/codec matrix | Broad common music support; no universal claim | Lossless/lossy fixtures, uncommon formats, rate/layout and output fallback tests |
| EQ numerical definition | Ten bands, preamp, presets and bypass | Frequencies/bandwidth/gain limits, headroom policy, response and click tests |
| Performance release budgets | Responsiveness is mandatory | Reference hardware, p50/p95/p99 latency, CPU/RAM and cold/warm results |
| Cache/retention budgets | Bounded and private | Working-set measurements, eviction and forget-source behavior |
| Catalog details | SQLite, indexed/FTS search, stable IDs | Scan concurrency, query plans, migrations and million-entry fixture |
| Advanced identity/relinking | Deferred; minimal path-independent IDs now | Conservative recording/version matching before cross-source substitution |
| Native playlist interchange | Native persistence first; practical M3U/M3U8 desired | Encoding/path scope and export limitations during playlist implementation |
| External playlist synchronization | Defer to service adapters; read/import first | API revisions, repeated occurrences, permissions and concurrent-edit behavior |
| Analysis and offline radio | Bounded jobs, simple cache; models unselected | Quality, resources, invalidation and library/model/data licenses |
| Older algorithm choices | Possible later, not promised | Maintenance/quality/security cost; distinguish results if coexistence is chosen |
| Per-device app volume | App-only gain settled | Playback-host versus output-endpoint persistence keys |
| Distribution matrix | Fedora/Debian/Ubuntu, then Windows | Minimum versions, architectures, audio/keyring/desktop integration |
| Linux packaging | Flatpak candidate | Startup, runtime size, source/device/secret-store access |
| Initial UI languages | English documentation selected; UI undecided | Product decision before copy/localization freeze |
| Desktop OS integration | Native controls are anticipated | Tray/media keys/background/close behavior decision |
| Theme package details | Declarative ReSolibre + classic Winamp `.wsz` | Versioned format and representative compatibility corpus |
| Future tag writes/deletion | Explicit confirmed writes only | Atomicity, backups/conflicts and source permissions before implementation |
| Future mobile/web UI and bindings | Portable core reuse required | Platform lifecycle/storage, framework and binding feasibility |
| Future LAN pairing transport | Account-free, explicit device pairing | Threat model, native/browser secure transport and revocation tests |
| Future internet connectivity | Deferred | Reachability, privacy, operating model; no mandatory account assumed |
| Companion/web/handoff/transfers | Optional later scopes accepted | State authority, browser access, destination availability and transfer safety |
| Mobile release channels | Deferred | License/distribution compatibility before committing to store delivery |
| Commercial music services | Exploratory | Authorized APIs, playback constraints and product decision |

## Settled direction

Local controls, EQ, layouts and catalog precede Jellyfin, then SMB; see the
[roadmap](../../plans/product-roadmap.md). Reevaluate playlists after Jellyfin; defer unless needed. Skins move near the end.
Rust modular local monolith; reusable core with platform adapters; local
first, then Jellyfin, SMB and Navidrome; adaptive audio instead of purity-based refusal;
EQ in each client's first release; compact and modern library layouts; themes
and classic skins; large-library performance; originals never changed by normal
use; GPLv3 (`GPL-3.0-only`); Lucide for ReSolibre controls; LAN remote control later,
internet after LAN. English Markdown/Mermaid documentation is selected.

Reuse common implementations before introducing platform variants. App volume
never changes OS/global volume; normalization treats every track independently.
Album membership never disables enabled crossfade. Advanced identity is deferred
without making paths the identity of native playlist items. The
[roadmap](../../plans/product-roadmap.md) separates release gates from future wishes.

The [local prototype](../features/local-playback.md) provides initial evidence;
it does not settle the broader feasibility or release-support rows.
The [test plan](../../testing/test-plan.md) defines how to collect evidence.
