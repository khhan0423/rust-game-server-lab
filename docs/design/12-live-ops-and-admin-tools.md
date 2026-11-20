# Design 12. Live Operations and Admin Tools

- Status: **approved** (2026-11-20)
- Related: [11-design-data.md](11-design-data.md), `08-operations-observability.md`, `01-threading.md` 5.7, `03-data-ownership.md`, `docs/research/admin-web-platform.ko.md` (security research)

**English** | [한국어](12-live-ops-and-admin-tools.ko.md)

This document defines how operators change a running service, and the web tools they use to do it. Decisions below were approved by the owner on 2026-11-20. The requirements come from earlier live services: changes whose "success" only meant "the signal was sent", grants that could pay twice, audit records written separately from the change, and admin tools writing directly into game databases. This design closes each of those gaps.

## 1. Where live-ops state lives

Live-ops state answers "what is turned on right now": notices, maintenance windows, content and shop switches, event schedules, sanctions.

- Game-wide items (notices, maintenance, switches, event schedules) live in the Common DB.
- Account sanctions live in the Account DB.

Each record carries:

- a generation number
- a target (all, one shard, one platform)
- an effective period
- the operator who made the change
- the reason

The Redis-compatible store (Valkey) holds none of this. It is a bulletin board, not the answer. The same rule as everywhere else in this project: authoritative data lives in the database.

## 2. Generation numbers and how a change reaches servers

```
ops tool ──► admin API of the owning service ──► DB commit (generation N+1, audit row in the same transaction)
                                              └─► PUBLISH on Valkey (hint only)
game servers: on the hint, on reconnect, and on a periodic check → read up to the latest generation → apply → report "applied N+1"
ops tool: shows requested / applied / failed per server
```

The publish command is a single Valkey command, used as a hint to make the change visible quickly. Correctness never depends on it. A server that misses the hint, was restarting, or was disconnected catches up from the generation number on its next periodic check or reconnect. The periodic check interval is short enough that a missed hint costs seconds of delay, not a failed operation.

Each server applies the new generation and reports back which generation it has applied. The commit that bumps the generation and the audit row are written in one database transaction, so a change can never exist without its record.

## 3. The control path

The operations tool never writes game databases. Every change goes through the admin API of the service that owns the state, on the private admin port defined in design 08 section 2. The owning service commits to its own database through the normal path.

This is a hard boundary. If the tool wrote databases directly, the owning service would not know its own state changed, the change would bypass fencing and idempotency rules, and the audit row would live in a different write than the change.

## 4. Grants

A grant gives items, currency, or mail to one player or a list of players.

- Every grant request has a request ID. Every recipient within it has an idempotency key.
- Execution goes through the existing grant path to the player's home shard, inside one transaction per recipient, with the idempotency insert in the same transaction. A retry sends only what failed and never pays twice.
- Every bulk grant requires a target preview, a reason, and a second approver. This is stricter than a money threshold and stays that way until the game economy defines amounts, at which point the threshold rule can replace it. Until then there is no bulk grant without a second approver.

The request, the grant ledger, and the audit event are recorded so that they cannot succeed separately.

## 5. Maintenance

Maintenance follows a staged flow. Each stage is a live-ops record and a command, and the tool shows which stage each server is in.

1. Block new entries (readiness off, so the load balancer stops sending new connections)
2. Announce the maintenance to connected players
3. Drain or disconnect
4. Confirm the server is empty
5. Patch
6. Verify readiness
7. Reopen in stages

## 6. The two web tools

Two separate tools, deployed and authenticated separately. They share no database and no session.

| Tool | Scope |
|---|---|
| Game operations tool | Game-side operations only. No access to login or payment data. |
| Publisher back-office | Accounts, login methods, platform sanctions, payments and refunds. No access to game databases. |

The game operations tool is built as a minimal version (server status, notices, maintenance, kick, grants) right after the pilot's cross-shard messaging stage, rather than waiting for the late operations stage of development. The full tool grows from it.

## 7. Stack

Decided on 2026-11-20: both tools use `axum` with Askama templates compiled at build time, plus htmx for partial updates.

The reason is in the security research ([docs/research/admin-web-platform.ko.md](../research/admin-web-platform.md)). The 2025 to 2026 incident record shows repeated compromises through Node server frameworks and the npm supply chain: authorization bypasses in middleware, server-side code execution through deserialization, and malicious packages pushed through stolen publisher accounts. A server-rendered Rust tool with no Node server and no npm build removes those attack paths. What remains is Cargo and the OS, which this project already operates.

htmx is a single vendored JavaScript file with a pinned version and recorded hash, served by the tool itself. It is not an npm dependency and no CDN is used. Askama templates are checked at compile time, so a broken template fails the build.

If a screen ever proves too complex for server rendering, the fallback is a static SPA with a same-origin Rust API, applied only to that tool, with the npm supply chain review that comes with it. That is a future decision, made only when a concrete screen needs it.

## 8. Security baseline

Regardless of stack, both tools follow the same baseline, from the security research:

- Not reachable from the public internet. Access through a VPN or a zero-trust access proxy on managed devices only.
- Operator sign-on with phishing-resistant MFA (passkeys or security keys). Sessions are short: 15 minutes idle, 8 hours maximum as starting values. Re-authentication immediately before grants and refunds.
- Role-based least privilege. Query, notice, maintenance, kick and sanction, grant, refund, and audit read are separate permissions. Hiding a button is not authorization; the admin API checks the operator's permission on every request.
- Every operator action is audited: who, when (UTC), what target, what change with before and after values, reason, result, request ID. Audit records are protected by separate permissions. A risky change that cannot record its audit event does not execute.
- CSRF protection, server-side origin checks, and a strict content security policy: same-origin scripts only, `frame-ancestors 'none'`, `object-src 'none'`, `form-action 'self'`.
- Secrets stay outside the repository, per design 08 section 8. The tools get no game database credentials at all.
- `Cargo.lock` is committed, and dependency advisories are checked with `cargo-audit` or `cargo-deny`. Neither is in the quality gate yet; adding one is part of building the first tool.

## 9. First screens

Game operations tool:

- server status and applied generations
- notices
- maintenance schedule
- content and shop switches
- kick
- sanctions
- player lookup (read-only role for support)
- grants with approval
- audit log

Publisher back-office:

- account lookup
- login methods and linking
- platform sanctions
- payments and refunds
- support history
- audit log

## 10. Monitoring additions

These metrics extend the design 08 metric set:

- Age of each server's status sample. A server whose report is stale is a server to investigate.
- Lag between a policy's commit and its application, per server. This is the number that answers "did the change actually reach every server".
- Partial-failure rate of grants. A grant that paid some recipients and failed others must be visible immediately.

Alerts: stale status sample beyond a threshold, apply lag beyond a threshold, and grant partial failure above zero.
