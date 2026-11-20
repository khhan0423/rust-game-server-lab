# Execution Plan

**English** | [한국어](README.ko.md)

- Status: **Draft** (2026-11-19)
- Basis: `docs/design/` 01~10, `docs/protocol/session-contract.md`, `docs/RUST_GUIDELINES.md`, `AGENTS.md`

This document defines the order, scope, and verification method for turning the design documents into actual code.

## Documents

| Document | Contents |
|---|---|
| [01-pilot.md](01-pilot.md) | Pilot: what to prove, ten phases, acceptance criteria, risks |
| [02-development-phases.md](02-development-phases.md) | Development phases D1~D9 after the pilot and implementation order |
| [03-test-scenarios.md](03-test-scenarios.md) | Test scenarios per phase (number, method, expected result) |
| [04-test-stages.md](04-test-stages.md) | Draft: the stages from development to launch (friends test, alpha, beta) and what the server needs for each |

## At a glance

```
[Pilot]  Verify the skeleton of the design. Content is minimal
  P0 Foundation → P1 Protocol → P2 Launcher → P3 Connection → P4 Content and DB → P5 Zone and Lobby
  → P6 Cross-shard messaging and recovery → P6.5 Minimal operations tool → P8 Reconnect → P9 Load measurement and verdict
  (P7 GUI test client runs alongside from P3)

[Development]  Content and operations on top of the skeleton
  D1 Design data → D2 Combat server and dungeons → D3 Fields and channels → D4 Social → D5 Guilds
  → D6 Steam login and payments → D7 Trading → D8 Cluster and operations → D9 Launch preparation
```

## Principles

1. **Thin end-to-end first.** Do not finish one layer completely before moving to the next. Open the thinnest path from connection to DB first, then widen it.
2. **Session structure first.** Zone and content go on top only after session creation, lookup, and termination are settled (P3 comes before P4 and P5).
3. **Build the tools first.** Tests, the launcher, and the test client must exist so later phases can proceed with confidence (P0, P2, P7).
4. **Kill it.** Tests that forcibly kill servers, not just happy paths, are a condition for completing a phase.
5. **Measure again.** Performance numbers are decided by measurement, not guesswork (P9, D3, D9).
6. **The design is the standard.** If something must be built differently from the design, fix the design document first and get approval.

## Definition of a completed phase

A phase is complete only when all of the following are done.

- All automated test scenarios for that phase (U, A, D, I, F) pass
- Automated scenarios of earlier phases keep passing
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` pass
- Manual scenarios (M) were checked in the GUI
- Changes are reflected in the design documents and in the progress table below

## Progress table

| Phase | Name | Status | Notes |
|---|---|---|---|
| P0 | Foundation | **Done** (2026-11-19) | Crates `infra`, `db`, `server`, `gateway`. 14 tests. Gate passed on Windows (MSVC, Rust 1.100.0) and on Linux (the `linux-gate` container on Rancher Desktop, 2026-11-20) |
| P1 | Protocol and frames | Not started | |
| P2 | Server launcher | Not started | |
| P3 | Connection and session | In progress | Publisher login server done (guest login, entry tickets, verification API; 2026-11-20). Entry server, gateway sessions, and bot remain |
| P4 | Content actors and DB | Not started | |
| P5 | Zone and lobby | Not started | |
| P6 | Cross-shard messaging and recovery | Not started | |
| P6.5 | Minimal operations tool | Not started | Server status, notices, maintenance, kick, grants (design 12) |
| P7 | GUI test client | In progress | Window, login panel, and log panel done; guest login against the login server works (2026-11-20). World view and game panels remain |
| P8 | Reconnect | Not started | |
| P9 | Load measurement and verdict | Not started | |
| D1~D9 | Development phases | After pilot verdict | |

## Decisions made before start (all confirmed as recommended on 2026-11-19)

The owner confirmed the items below as recommended. They are reflected in guide chapters 2 and 13 and in AGENTS.md.

| # | Decision | Why | Confirmed |
|---|---|---|---|
| C1 | Crate for shared operations code | Config loading, log initialization, metrics, and the admin port are shared by Gateway and the servers. The current crate layout has nowhere to put them (`net` is network code, `gateway` does not know `game` or `db`) | Add `crates/infra`. `gateway → infra`, `server → infra`. `infra` does not depend on game crates |
| C2 | Token signing method | Entry signs and Gateway verifies | Ed25519 (`ed25519-dalek`). Gateway only needs the public key, so even if Gateway is compromised, tokens cannot be forged |
| C3 | Protocol code generation tool | Generates Rust code from `.proto` | `prost-build` (build-time dependency only) |
| C4 | Valkey client | The guide says decide between `redis` or `fred` at implementation time | `redis` crate (most widely used, sufficient for our use of single commands only) |
| C5 | Random numbers | Session key generation | `rand` (uses the OS random source) |
| C6 | Command-line argument handling | Launcher headless mode, bot scenario selection | `clap` |
| C7 | Development authentication | Login until Steam integration | **Changed (2026-11-19):** do not build fake authentication. Build the publisher login server in pilot P3 and use guest login |
| C8 | Crate for the publisher login server | A place is needed to implement the publisher area of design 07 (accounts, login methods, entry tickets) | `crates/publisher` binary. The Login DB SQL goes in a separate module of the `db` crate. Game server crates do not touch the Login DB. Approved 2026-11-20 |

## Design documents still to write

| Document | When needed |
|---|---|
| Protocol message list (messages per kind and numbering rules) | P1 |
| Game DB schema (player, wallet, inventory, mail, idempotency keys, outbox) | P4 |
| Save failure policy (when saving state already applied in memory ultimately fails) | P5 |
| Design data | D1. Design doc 11 already exists (confirmed 2026-11-20) |
| Social (friends, chat, ranking) | D4 |
| 06 Cluster | D8 |

## Decisions still open

| Item | When decided |
|---|---|
| Capacity per field, thread count, pool sizes, allocator | After P9, D3 measurements |
| Items that will use the consumable lease model | After D2 measurements |
| Data retention after leaving (provisional: archive after 6 months) | D9, legal review |
| Process supervision (systemd), operations log retention (90 days) default confirmation | D8 |
