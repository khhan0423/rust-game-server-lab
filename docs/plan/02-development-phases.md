# Development Phases and Implementation Order

**English** | [한국어](02-development-phases.ko.md)

- Status: **Draft** (2026-11-19)
- Parent document: [Execution Plan](README.md). Previous phase: [Pilot](01-pilot.md). Tests: [03-test-scenarios.md](03-test-scenarios.md)

This is the plan for after the pilot passes the quality gate. Content and operations features are added on top of the foundation built in the pilot.

## 1. Overall Order

```
Pilot (P0~P9)
   │
   ▼
D1 Design Data ─► D2 Combat Servers and Dungeons ─► D3 Fields and Channels ─► D4 Social ─► D5 Guilds
                                                                                              │
            D6 Steam Login and Payments (can run in parallel anytime after D2) ◄──────────────┤
                                                                                              ▼
                                             D7 Trading ─► D8 Cluster and Operations ─► D9 Launch Preparation
```

There are three reasons for this order.

1. **Later phases use what earlier phases build.** Dungeons need design data (monsters, reward tables), and guild dungeons need both dungeons and guilds.
2. **Do the risky work first.** Dungeons for parties with players from different shards (D2) make the most use of cross-server paths, so validate them early.
3. **Do the work involving money once the foundation is solid.** Payments (D6) and trading (D7) are added after idempotent processing and recovery have been validated in several phases.

## 2. Scope by Phase

| Phase | Name | Design Documents to Write First | Estimated Duration |
|---|---|---|---|
| D1 | Design Data | [Document 11](../design/11-design-data.md) (finalized 2026-11-20) | 2 weeks |
| D2 | Combat Servers and Dungeons | Document 03, section 3.4, chapter 4, and section 6.1 (existing) | 4 weeks |
| D3 | Fields, Channels, and NPCs | Document 05, chapter 3 (existing) | 4 weeks |
| D4 | Social | Social document (new) | 3 weeks |
| D5 | Guilds | Expand Document 04, section 5.1; Document 05, chapter 4 | 4 weeks |
| D6 | Steam Login and Payments | Document 09 (existing), revise Document 07 | 4 weeks |
| D7 | Trading | Document 03, section 6.3 (existing) | 3 weeks |
| D8 | Cluster and Operations | Document 06 on clusters (new), Document 08, [Document 12](../design/12-live-ops-and-admin-tools.md) | 4 weeks |
| D9 | Launch Preparation | Load and failure test results report | 4 weeks or more |

### D1. Design Data

- Implement Design Document 11 (finalized on 2026-11-20) as written.
- **What to build:** A TOML schema for each table and typed IDs and row types generated from the schemas, a spreadsheet export tool (one JSON file per table and a manifest), three layers of validation at export/CI/startup, a content hash check at login, and whole-snapshot replacement for reloads during operation (Document 01, 5.7).
- **Implementation order:** Schemas and code generation → export tool and validator → connect to the startup barrier → login hash check → `/admin` reload → display the content version in the test client.
- **Completion criteria:** The server does not start with invalid tables. Clients with a different hash are denied login. Reloading during operation does not interrupt in-flight requests.

### D2. Combat Server and Dungeons

- **What to build:** A combat server role (no content runtime or DB), instanced dungeon mode, a party actor (location sharing through Valkey), passing `CombatProfile` on entry and switching the zone destination in the Gateway, looting (`GrantItem`, roll back if the inventory is full), clear rewards (send only the result and let the home shard calculate the rewards), and delivery by mail if important rewards do not fit in the inventory.
- **Implementation order:** Combat server startup and registration → dungeon creation and entry (same-shard party) → zone destination switching → monsters and basic combat (1 to 2 skills) → looting → clear rewards → mixed-shard parties → handling forced combat server shutdown.
- **Completion criteria:** Three users from different shards run a dungeon, and rewards are recorded exactly once in each user's game DB. Rewards that have already been finalized remain even if the combat server process is killed.

### D3. Fields, Channels, and NPCs

- **What to build:** MMO field mode, channels (soft capacity, hard capacity, automatic assignment, manual transfers and cooldowns, closing empty channels), publishing channel populations to Valkey, NPC AI (state machine), activation by AOI cell, and movement between locations on the same server and forwarding packets during the move.
- **Implementation order:** Field mode and one persistent channel → NPC spawning and AI → cell-level activation → adding channels and automatic assignment → manual transfers → movement between the lobby and the field.
- **Completion criteria:** A new channel opens when the number of users entering exceeds capacity, and party members enter the same channel. NPCs in cells with no players are not updated.

### D4. Social

- **What to build:** Friends (request, accept, remove; Common DB), linking friend codes to shard assignment, displaying online status, visits to friends' lobbies (including on other shards) and visit rewards, chat (whisper, party, location), and a leaderboard.
- **Implementation order:** Friend relationships → online status → friend codes → lobby visits → chat → rankings.
- **Completion criteria:** A player can enter a friend's lobby on another shard and spend time there with the friend, and the visit reward is granted only once on the visitor's home shard.

### D5. Guilds

- **What to build:** A guild actor (Common DB, claiming ownership by a server and fencing), joining and leaving, a list of ranks and permissions, a capacity of 30 members, guild currency, guild storage (currency and items, permissions by rank, daily limits, deposit and withdrawal logs), guild scores and rankings, guild dungeons (permission to open, 5 concurrent dungeons per guild, day-of-week/always-available/weekend schedules), waking the guild actor to process work when no one is online, and batch jobs at scheduled times (weekly settlement, expiration cleanup).
- **Implementation order:** Guild creation and joining → ranks and permissions → guild currency → storage deposits and withdrawals → guild scores and rankings → opening and scheduling guild dungeons → batch jobs → (later) guild town.
- **Completion criteria:** The total item count remains correct when guild members from different shards deposit and withdraw items. Guild members without permission cannot open dungeons. A day-of-week dungeon opens only on its designated day.

### D6. Steam Login and Payments

- **What to build:** Add Steam as a login method to the publisher's login server (Steam ticket verification and linking with guest accounts), a Commerce service, in-game purchases (order creation → approval → finalization → grant), periodic settlement report queries and refund clawbacks (negative balances and paid shop restrictions), and granting and syncing Steam items (cosmetic only).
- **Implementation order:** Steam login → order state machine and Commerce DB → end-to-end purchase in the sandbox → disconnection and timeout handling → refund clawbacks → Steam item definitions and sync at login → server-side grants.
- **Completion criteria:** In the sandbox, no matter when the server process is killed after payment is finalized, the grant eventually happens exactly once. Currency is clawed back when a payment is refunded.

### D7. Trading

- **What to build:** Trade service (intermediary, escrow), deposits from both sides → exchange confirmation → payouts to both sides, and returns on cancellation or timeout.
- **Implementation order:** Trade state machine → same-shard trades → cross-shard trades → cancellation and timeout → recovery after killing the trade service process.
- **Completion criteria:** Items are not duplicated or lost, no matter which process is killed at any stage.

### D8. Cluster and Operations

- **What to build:** Multiple game servers in one shard (which server handles each player actor), draining and replacement without downtime, takeover when a server dies, Valkey redundancy and behavior on failure, deployment (systemd), monitoring and alerting (Prometheus, Grafana, Alertmanager), log collection (Fluent Bit) and Log DB, full expansion of the operations tool (for announcements, kicks, maintenance mode, and grants, add screens on top of the minimal tool in Design 12. P6.5), and DB backup and recovery drills.
- **Implementation order:** 06 cluster document → draining → multiple servers in one shard → failover → Valkey failure behavior → deployment scripts → monitoring and alerting → log collection → operations tool → backup and recovery drill.
- **Completion criteria:** Users are not disconnected when one server is drained and replaced. Users already connected can keep playing even if Valkey is taken down. Run through the procedure for restoring from a backup once.

### D9. Launch Preparation

- **What to do:** Load testing with 10,000 concurrent users per process on production-spec servers, 72-hour continuous testing, fault injection testing (server, DB, Valkey, network), security checks (packet forgery, speed manipulation, request replay, management port exposure), final decisions on field capacity and thread count, finalizing the data retention policy for closed accounts (Legal), specifying how currency is handled for refunds in the terms of service, and the Steam launch process.
- **Completion criteria:** Fill in every part of the launch readiness checklist (load, failures, security, operations procedures).

## 3. Rules for Every Stage

| Item | Details |
|---|---|
| Design first | If a structure not in the design document is needed, update the document first and get approval (AGENTS.md) |
| Definition of done | Code + tests + documentation updates + passing the quality gate + direct verification in the GUI test client |
| Test order | Pure logic unit tests → actor tests → tests with a real DB → multi-server integration tests → forced termination tests |
| Regression | Automated scenarios from earlier stages must continue to pass. Add scenarios for bugs that have been fixed |
| Bots and clients | Add both a bot scenario and a GUI panel for each new feature |
| Metrics | Each new feature reports processing time, failure count, and queue length as metrics |
| New libraries | Get approval before using them and list them in the table in Chapter 13 of the guide |
