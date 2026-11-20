# Pilot Plan

**English** | [한국어](01-pilot.ko.md)

- Status: **Draft** (2026-11-19)
- Parent document: [Execution Plan](README.md). Test scenarios: [03-test-scenarios.md](03-test-scenarios.md)

## 1. What the Pilot Is

It is **a small, complete product that verifies that the main parts of the design fit together in actual code**. It has almost no content, but it goes through every layer once, from connection to saving to the DB.

```
GUI test client / bot ── 1. Login ──► Publisher login server ── Login DB
        │                                         ▲ Ticket check
        ├── 2. Enter ───────────────────► Entry server ── Account DB
        │
        └── 3. Play ───► Gateway ──────────────── Valkey (connection state, server responsible for the space)
                            │
                ┌───────────┼───────────┐
           Game Server 1 Game Server 2 Game Server 3
                │           │           │
            Game DB 1   Game DB 2   Game DB 3

The server launcher (GUI) starts the servers above in order, shuts them down, and forcibly kills one of them.
```

There are only five things a user can do in the pilot.

1. Log in as a guest (the publisher login server creates an account and gives the user an entry ticket). First-time users are assigned a shard.
2. Enter their own lobby and move around. They can see other people.
3. Buy an item from the shop.
4. Send an item by mail to a user on another shard.
5. Reconnect after the connection drops.

### 1.1 What the Pilot Must Prove

| # | What to prove | Design reference |
|---|---|---|
| V1 | The zone thread (synchronous, 10Hz) and the player actor (async) run together without locks | 01 Chapters 4 and 5 |
| V2 | The client connects to only one Gateway, and messages go to the right server based on their type | 02 3.2 |
| V3 | Game Server N connects only to Game DB N, and cross-shard operations are applied only once via messages | 04 Chapter 2, 03 Chapter 3 |
| V4 | Currency changes only after the DB commit, and resending the same request does not cause it to be processed twice | 01 5.5 |
| V5 | Committed data and messages being sent are not lost even if the server process is forcibly killed and restarted | 01 Chapter 10 |
| V6 | Only one server is responsible for a space (InGameSpace), and when it becomes empty, it is saved and unloaded after a grace period | 01 Chapter 6, 05 |
| V7 | A stalled request can be found in the logs by user number and request number | 01 5.8 |
| V8 | A dropped connection resumes within the grace period, and no important responses are missed | Session contract |
| V9 | 2,000 bots run on a development PC within the latency target | 01 Chapter 9 (validate assumptions) |

### 1.2 What the Pilot Does Not Include

| What is not included | Reason | When to add it |
|---|---|---|
| Steam login and payments | Validate the architecture first without external integration. The publisher login server supports only guest login | Development stage D6 |
| Combat server, dungeons, PvP | Add them using the same framework after the zone architecture is validated | D2 |
| MMO fields and channels, NPC AI | Validate the zone architecture first with a single lobby | D3 |
| Guilds, friends, chat, rankings | Add them after the cross-shard messaging foundation is validated | D4, D5 |
| Design data pipeline | A few small TOML tables are enough for the pilot | D1 |
| Running one shard across multiple servers, Valkey redundancy | The 06 cluster document is needed first | D8 |

## 2. Stages

The pilot has eleven stages. Each stage is complete only when **the code, tests, documentation updates, and passing the quality gate** are all done.

```
P0 Foundation ─► P1 Protocol ─► P3 Connection ─► P4 Content ─► P5 Zone ─► P6 Cross-shard ─► P6.5 Minimal operations tool ─► P8 Reconnect ─► P9 Measurement
   └───────────► P2 Launcher ─────────┘          P7 GUI client (runs in parallel from P3)
```

| Stage | Name | What to build (summary) | Effort (estimate) |
|---|---|---|---|
| P0 | Foundation | Workspace, quality gate, development DB/Valkey, configuration, logs, metrics, management port | 1 week |
| P1 | Protocol and frames | Message definitions, 16-byte frame codec, server-to-server envelope codec | 1 week |
| P2 | Server launcher | GUI launcher and windowless mode, topology file, startup in order, forced shutdown | 1 week |
| P3 | Connection and session | Publisher login server (guest login, entry tickets), Entry server, Gateway sessions, shard assignment, duplicate login, bot basics | 3 weeks |
| P4 | Content actor and DB | Player actor, Game DB schema, purchases, idempotency key, startup barrier, trace logs | 2 weeks |
| P5 | Zone and lobby | Zone thread, tick, AOI, lobby entry and movement, claiming ownership of a space, saving and unloading | 3 weeks |
| P6 | Cross-shard messaging and recovery | Server-to-server links, outbox, cross-shard mail, graceful shutdown, recovery from forced shutdown | 2 weeks |
| P6.5 | Minimal operations tool | Minimal web tool for viewing server status, posting announcements, performing maintenance, kicking players, and issuing grants (Design 12) | 2 weeks |
| P7 | GUI test client | Bevy screens: login, lobby view, movement, shop, mail, packet log | 2 weeks (in parallel) |
| P8 | Reconnect | Reliable sequence numbers and acknowledgments, retransmission buffer, Resume, resuming through a different Gateway | 2 weeks |
| P9 | Load measurement and assessment | Bot load scenarios, dashboards, deciding on configuration values, pilot report | 1~2 weeks |

The scale is estimated for one developer + AI assistance. Actual progress is recorded in the progress table in [README](README.md).

### P0. Foundation

- **Workspace:** Centralize versions and lints in the root `Cargo.toml` (Guide Appendix A). Create crates **at the stage where they are first needed** (do not create empty crates in advance). Create `infra`, `db`, `server`, and `gateway` in P0, `protocol` and `net` in P1, `launcher` in P2, `bot` in P3, `game` in P4, and `client` in P7.
- **Shared operations code:** Put configuration loading, log initialization, metrics, and the admin port in the `infra` crate. When ready, the server writes a single line, `ready admin=<admin port address>`, to standard output. The launcher and integration tests wait for this line (they do not poll periodically).
- **Development environment:** Use `dev/compose.yaml` to start one PostgreSQL instance (databases `account`, `common`, `game_1`, `game_2`, `game_3`) and one Valkey instance. In production the DBs are separate, but on the development PC they are grouped in one instance. Put passwords and test connection strings in the root `.env` (not committed to the repository; see `.env.example` for an example).
- **Migrations:** Start with `migrations/game` (apply the same migrations to all shards). Add `account` and `common` at the stage where those DBs are first used (P3). Apply migrations with the `server migrate` command.
- **Configuration and secrets:** Use TOML for general configuration and environment variables for secrets. Redact secret types in `Debug` output (Document 08, Chapter 8).
- **Observability:** `tracing` JSON logs (asynchronous file writes), `/health/live`, `/health/ready`, `/metrics`.
- **Quality gate scripts:** `scripts/gate.ps1` and `scripts/gate.sh` run fmt, clippy, and test in order.
- **Completion criteria:** The gate passes. The empty `server` and `gateway` binaries start and respond to health checks. One `sqlx::test` passes against a real PostgreSQL instance.

### P1. Protocol and Frames

- Define System messages (Auth, AuthOk, Resume, Ping, Close, EnterGame, etc.) and Content and Zone messages for the pilot in `.proto`. Generate message IDs within the range for each type (Document 02, Section 3.2).
- Implement the client frame (16-byte header) and interserver envelope codecs in `net` (Document 02, Sections 8.1 and 4.2).
- **Completion criteria:** Encoding round-trip, boundary value, and malformed input tests pass for all messages.

### P2. Server Launcher

- Read `topology.toml` and start Entry, then the game servers, then Gateway. Move on only after each server's `/health/ready` is up.
- UI: status, PID, port, and log window for each server, with buttons to start, stop, restart, and kill the process. Determine whether a server is ready from the `ready admin=<address>` line on standard output.
- There must be a **headless mode** (`launcher --headless up`, etc.). Automated integration tests use it to start and kill the server group.
- **Completion criteria:** Five empty servers start in order, killing one process is reflected on the status dashboard, and no processes remain after a full shutdown.

### P3. Connections and Sessions

- **Publisher login server:** A web server separate from the game (Design 07, Chapter 2). Implement only guest login for the pilot. Guest account creation (store only the hash of the secret kept on the device), login, game entry ticket issuance, a ticket verification API called by the game, and the Login DB. Add other login methods such as Steam to the same structure later (D6).
- **Entry server:** Verify the ticket with the publisher login server (do not trust account information sent by the client), look up AccountDB and handle first entry, assign a shard (prioritize the friend code; if none, use the shard with the lowest cumulative player count), and issue a signed `login_token`. At first entry it takes the nickname (unique across the game), issues the player tag, and never sends internal keys to the client (design 07 section 5.1).
- **Gateway:** Accept connections, handle session state transitions, verify tokens, handle duplicate login using Valkey `online:{account}`, route by message ID range, handle heartbeats, and enforce size and rate limits.
- **Game server:** Accept Gateway links and notify of session binding (`SessionBound`).
- **Bot:** A reference implementation that connects according to the session contract, and a scenario runner.
- **Completion criteria:** A bot logs in as a guest, gets a ticket, and connects through Entry to its home shard's game server. Forged, expired, and already used tickets are rejected. The shard does not change on login again. On duplicate login, the first connection is cleaned up before the new one connects.

### P4. Content Actors and DB

- Content runtime, `ContentManager`, and player actors (with bounded inboxes).
- Game DB: players, wallets, inventories, mailboxes, idempotency keys, outbox, transaction logs.
- `db` crate functions: load, purchase (one transaction), and return results on idempotent re-execution.
- **Startup barrier:** Configuration, then DB connection and migration version check, then shared data loading, then runtime startup, then `ready`. If any step fails, startup fails. Discard blocking DB tools after startup.
- **Tracing:** A span with `player_id` and `request_id` for each command. Warnings for slow commands and inbox backlogs.
- **Completion criteria:** When a bot buys an item, the purchase is reflected in the DB, and resending the same request reduces the balance only once. In the logs, a single request ID can be used to trace the request from Gateway through to the DB.

### P5. Zones and Lobbies

- Zone thread and `ZoneHost` tick loop (10 Hz, command limit per tick, skip ticks if behind).
- `InGameSpace` (shared core + lobby mode), 2D grid AOI, entry, exit, and movement.
- `InGameSpaceManager`: Claim server ownership and renew leases through Valkey, queue entry requests during loading, and perform a final save and unload after a 10-minute grace period.
- Lobby state persistence: The zone sends requests to a DB task instead of calling the DB directly. Attach a fencing number and a version number to each save.
- Between zone and content: Pass `CombatProfile` and track the pending state of equip requests.
- **Completion criteria:** Multiple bots in the same lobby see each other move. Movement updates are not sent to bots outside the area of interest. Once everyone leaves, the lobby is saved and unloaded after the grace period, and it starts in the saved state when someone enters again.

### P6. Cross-Shard Messages and Recovery

- Interserver links (including authentication), outbox sender, and idempotent receiving.
- Cross-shard mail: On the sending side, delete the item and write the outbox record in one transaction; on the receiving side, add the item to the mailbox and record the idempotency key in one transaction.
- For messages addressed to offline users, briefly wake the actor to process them.
- Graceful shutdown sequence (Document 01, Section 10.1) and recovery on restart after a process is killed.
- **Completion criteria:** The "3 users, 3 shards" scenario passes automatically. Even if any server process is killed and restarted midway through, mail arrives exactly once.

### P6.5. Minimal Operations Tool

- First build a minimal version of the game operations tool finalized in Design 12. It has screens for server status and applied generations, announcements, maintenance schedules, player kicks, and grants requiring approval. Follow the axum + Askama + htmx stack (Design 12, Chapter 7) and security standards (Chapter 8) as specified.
- **Completion criteria:** When announcements and maintenance are scheduled in the tool, each game server catches up by generation number and its application report appears in the tool. Resending the same grant request results in only one grant. There is no path for the tool to write directly to the game DB.

### P7. GUI test client (in parallel starting from P3)

- Render the lobby in a single Bevy window. You can see yourself, other users, and the AOI grid.
- egui panel: login (account, friend code), inventory, shop, mail, packet log (send/receive, filter), disconnect and reconnect buttons, latency display.
- The network layer uses the same code as the bots.
- **Completion criteria:** A person can try logging in, moving, making purchases, using mail, and reconnecting in the window, and Korean text displays correctly.

### P8. Reconnect/resume

- Reliable frame sequence numbers and acknowledgments, a retransmission buffer (with an upper limit), disconnection detection and a grace period, a `Resume` handshake, and a space snapshot on resume.
- Keep a session mirror in Valkey so the session can resume through a different Gateway as well.
- `Suspend` (a longer grace period when the app is backgrounded).
- **Completion criteria:** Scenarios A~H in the session contract document pass automatically with bots.

### P9. Load measurement and evaluation

- Bot load scenarios and latency distribution collection, Prometheus and Grafana dashboards (development compose).
- Values determined by measurement: content worker count, zone thread count, initial lobby capacity, pool size, and whether to adopt the allocator (mimalloc).
- **Completion criteria:** Meet the evaluation criteria in Section 3 and record the results in `docs/plan/pilot-report.md`.

## 3. Pilot evaluation criteria

| Category | Criteria |
|---|---|
| Functionality | All automated scenarios P0~P8 in the test scenario document pass |
| Consistency | Total amounts of currency and items match in the forced termination scenarios (P6-F, P8). 0 duplicate grants, 0 losses |
| Performance (development PC, 2,000 bots, 3 shards) | Zone tick p99 at most 50ms. Content request latency p99 at most 150ms (including DB). DB pool wait p99 at most 1 second. 0 errors |
| Density | Measure the maximum number of users in one space while keeping tick p99 at or below 50ms. This number becomes the initial capacity |
| Isolation | When DB calls for some users are deliberately slowed down, request latency for other users does not increase |
| Stability | Under 4 hours of continuous load, memory does not keep growing, and there are 0 instances of a full channel |
| Operability | Find a single deliberately stalled request within 5 minutes using only the logs |
| Quality | fmt, clippy(-D warnings), and test pass. 0 occurrences of `unsafe` or `unwrap` (outside tests) |

The performance numbers are a starting point. Measure the final target (10,000 concurrent users per process) again on a production-spec server during launch preparation.

### 3.1 If an evaluation criterion is not met

| Failed criterion | Response |
|---|---|
| Zone tick | Apply shared broadcast serialization, batched sending per recipient, and less frequent updates for distant entities, in that order, then measure again. If that still does not work, lower the capacity |
| Content latency | Adjust the content worker count and DB pool size. Move heavy computations to the blocking pool |
| Isolation failure | Find and fix code that ties up a worker inside an actor (synchronous calls, long computations) |
| Consistency | This is a launch blocker. Fix the cause and make the scenario a regression test |

## 4. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| Rust proficiency | Slow progress at first | Start with small crates at each stage and explain new concepts in the report by comparing them with C++ (AGENTS.md) |
| Bevy versions change quickly | Client maintenance cost | Use the client only as a tool and keep it separate from the server code. Pin the version |
| Integration tests become slow or unstable | Development flow is interrupted | Wait for events instead of relying on timing. Set short timer durations through configuration. Start the servers in the launcher's windowless mode |
| The servers, DB, and bots all run on one development PC | Performance numbers differ from real-world results | Use the pilot numbers only for relative comparisons and as starting values |
| Reconnect/resume (P8) is more work than expected | Schedule delay | Put P8 after P6 so that earlier stages have already been verified even if P8 is delayed |
