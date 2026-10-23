# Audio output failure plan

Status: completed on 2026-10-20; archived after verification.

Objective: distinguish EOF from unavailable/lost output, then verify a fresh
player after restoring a private virtual audio service. Production defaults,
public API and automatic recovery policy remain unchanged.

| Step | Completion |
| --- | --- |
| 1. Private service | Owned PipeWire/Pulse graph, explicit links, non-silent monitor; no desktop service changes |
| 2. Baseline/missing output | Three EOF runs and three safe errors with endpoint absent |
| 3. Connection loss | Three errors after monitor-confirmed audio, no EOF or stale formats |
| 4. Fresh recovery | Three new players reached EOF after failed-worker destruction and service restart |
| 5. Fix/check/close | No production defect found; negative/regression checks passed; docs updated |

All acceptance bounds, commands, safeguards, measured results and limitations
are preserved in the [validation record](../../testing/output-failure-validation.md).
Physical loss, automatic reconnect and latency remain outside this plan.
