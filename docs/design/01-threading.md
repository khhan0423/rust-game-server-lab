# Design 01. Thread Model

**English** | [한국어](01-threading.ko.md)

- Status: **v2.5** (2026-11-19). v2.5: confirmed that player actors are not pinned to OS threads (tokio multi-thread runtime), added debugging and tracing rules (5.8). v2.4: AOI cell-granular NPC activation (4.6), per-entity pending state (4.8), ban on C-style calls from operations commands and reload (5.7), startup order and startup recovery (10.0). v2.3: changed content from a synchronous thread to **per-player async actors** (a dedicated tokio runtime for content). v2.1: Directory and server registration moved to Redis (confirmed 2026-11-18). v2.2: Redis uses single commands only, decisions belong to the InGameSpaceManager actor, and consistency is handled by DB fencing. Changes from v1: GatewayLink due to the Gateway introduction, content thread addition, InGameSpace structure (core + mode), connection event policy, InGameSpaceManager two-layer structure
- Prerequisites: `02-session-network.md` (session, network), `03-data-ownership.md` (data ownership, batching), `04-database.md` (DB configuration)
- Related: `docs/RUST_GUIDELINES.md` chapter 6 (ownership, concurrency, async)

## 0. Requirements

### 0.1 Scale

| Item | Value |
|---|---|
| Concurrent users per process | 10,000 or more |
| Game tick | 10Hz (100ms) |
| Operations OS | Linux (development on Windows) |
| Panic policy | immediate process termination (`panic = "abort"`) + core dump + supervisor restart |

### 0.2 InGameSpace

Every place where a player can exist is called an **InGameSpace**. Placement rules follow chapter 5 of document 03.

| Kind | Description | Host | Lifetime | Persistent state |
|---|---|---|---|---|
| **Lobby** | MMO-style field for an individual or a small community. Many NPCs. Friends from other shards can visit | Owner's shard | Load on entry, unload when empty | **Permanent** (owner DB) |
| **MMO field** | Public field you move to from the lobby | Each shard (channel) | Resident | Player state only |
| **Instance dungeon** | One-time dungeon per party. Mixed-shard parties allowed | Battle server pool | Creation ~ clear/exit | Results only (to home shard) |
| **Guild dungeon** | Created by qualified players. Time-limited | Game server with low load (guilds do not belong to a shard, document 04 section 5.2) | Creation ~ expiry (persists across restarts) | **Permanent within the period** (Common DB) |
| **PvP field** | 1:1, N:N. Players from multiple shards | Battle server pool | Per match | Results only (to home shard) |

### 0.3 Impact on thread design

1. **There are very many InGameSpaces and most are lightweight.** Every connected player can have a lobby.
2. **InGameSpaces appear and disappear constantly, and may be on other servers.** Finding and placing them is a cluster-level job (chapter 6).
3. **Persistent data (content) and simulation (zone) have different owners.** Content belongs to the home shard, zones belong to the InGameSpace host (document 03). So the processing styles differ too (zone: synchronous thread, content: async actor).
4. **Client sockets exist only on the Gateway.** The backend deals only with Gateway links and inter-server links (document 02).

---

## 1. Design principles

1. **Everything with state has "its own job queue + a single flow of execution" (an actor).** Everyone else holds only a **handle** to put work into that queue. So there are no locks.
2. **Game logic runs single-threaded without locks.** The state of one InGameSpace is touched by exactly one zone thread, and the content state of one player is touched by exactly one **player actor**.
3. **Waiting is async, simulation is synchronous.** Work that is mostly waiting (sockets, DB, inter-server communication, coordination, **content**) runs as async on tokio; work where computation and a fixed tick are central (**zone**) runs as a synchronous loop on a dedicated OS thread.
4. **The tick does not jitter.** Network load or DB latency never affects the tick period.
5. **No thread ever blocks waiting for another thread.** Inter-thread communication uses only bounded channels and non-blocking sends.
6. **Thread counts come from configuration.**
7. **A dormant InGameSpace costs almost nothing.**

---

## 2. Overall structure

### 2.1 Game server (shard)

```
                      ┌──────────────────────────────────────────────────────┐
  Gateway ──link───>  │ I/O runtime (tokio multi-thread, io-worker-N)        │
  other servers ─link→│  GatewayLink × (Gateway count × link count)          │
                      │  ServerLink × (other server count × link count)      │
                      │  InGameSpaceManager task (in-process placement)      │
                      │  DB task (sqlx connection pool), KV task (Redis-compat) │
                      └──────┬──────────────────────────────┬────────────────┘
     deserialized messages  │ Content                      │ Zone
                             ▼                              ▼
          ┌───────────────────────────────┐   ┌──────────────────────────────────┐
          │ Content runtime (tokio, content-N)│   │ zone threads × Kz (zone-N)       │
          │  PlayerContent × (assigned players)│<─>│  zone-0: [MMO field A] [lobby ×1200]│
          │  inventory, currency, mail, quests│   │  zone-1: [lobby ×1150] [guild dungeon]│
          └───────────────────────────────┘   └──────────────────────────────────┘

  Others: main thread, tokio blocking pool, log writer thread
```

### 2.2 Battle server

The same binary as the Game server, launched in a different role. **No content runtime and no DB task.** Only instance dungeons and PvP fields run on zone threads. Player data is exchanged with each player's home shard via inter-server messages (document 03).

### 2.3 Gateway

No zone threads and no content runtime. Everything is a task on tokio (document 02, chapter 3).

### 2.4 Correspondence with a C++ server

| This design | C++ MMO server |
|---|---|
| I/O runtime workers | IOCP worker thread pool |
| GatewayLink, ServerLink | Inter-server communication sessions |
| Zone thread | Zone-dedicated thread (job queue + tick loop) |
| Player actor (content runtime) | Dedicated thread for player DB work/content processing + DB job callbacks. `.await` instead of callbacks |
| InGameSpaceManager + Redis Directory | ZoneManager singleton + world server |
| Handle (channel send end) | Singleton `Instance()` + job submission |

---

## 3. Thread and task list (Game server)

### 3.1 OS threads

| Name | Count | Does | Must not do |
|---|---|---|---|
| `main` | 1 | Load config, create runtimes and threads, register the server in Redis, orchestrate shutdown | Game logic |
| `io-worker-N` | config value | Run the tasks in 3.2. Packet deserialization and validation | Access game state, blocking calls |
| `zone-N` | Kz (config value) | InGameSpace ticking, Zone message processing | Socket I/O, direct DB calls, any blocking other than channel receive |
| `content-N` | Kc (config value) | Workers of the **content runtime** (dedicated tokio). Run player actors and ContentManager (chapter 5) | Blocking calls |
| tokio blocking pool | on demand | File loading, synchronous library calls | Resident loops |
| Log writer | 1 (`tracing-appender`) | Writing log files | |

Every thread gets a name (`thread::Builder::name`, tokio `thread_name`).

### 3.2 Tasks on the I/O runtime

| Task | Count | Does |
|---|---|---|
| Acceptor | 1 | Accept links from Gateways and other servers on the inter-server endpoint |
| GatewayLink | Gateway count × link count (reader/writer) | Envelope parsing, deserialization, validation, routing by category to the player actor (Content) or zone thread (Zone). Send responses |
| ServerLink | other server count × link count | Send and receive inter-server messages (document 02 section 4.4) |
| InGameSpaceManager | 1 | Create, place, and destroy InGameSpaces in the process. Lease renewal in Redis for hosted InGameSpaces, and entry waiting queue (chapter 6) |
| DB | 1 (connection pool used concurrently) | Load, save, run transactions. Reply results to the requesting thread |
| KV | 1 (connection pool used concurrently) | Access the Redis-compatible store (player directory lookup, online presence, etc., document 03 chapter 9). Reply results as messages |

---

## 4. Zone thread

### 4.1 Why a dedicated OS thread instead of a tokio task

- tokio's work-stealing scheduler is good for throughput but gives no guarantee about when a specific task runs. Mixed with I/O tasks for 10,000 concurrent users, ticks fall behind.
- InGameSpace state is owned by a single thread, so work-stealing has no benefit.
- When an InGameSpace is pinned to a specific thread, cache locality is good and profiling results are easy to read.

There is no async inside a zone thread. It is a plain synchronous loop.

### 4.2 ZoneHost

One zone thread = one ZoneHost. It owns multiple InGameSpaces, runs ticks, and delivers Zone messages to the relevant InGameSpace. It lives in the `server` crate.

### 4.3 InGameSpace structure: common core + mode

InGameSpace follows the C++ **abstract class + derived class** model. Rust has no class inheritance, so it splits into **common data (struct)** and **per-mode behavior (trait)**. It lives in the `game` crate and knows nothing about threads, channels, or tokio.

| C++ | Rust |
|---|---|
| Abstract class `InGameSpace` | `struct InGameSpace { core, mode }` |
| Protected members of the base | Fields of `InGameSpaceCore` |
| Pure virtual functions | **required methods** of the trait (compile error if not implemented) |
| Virtual functions with default implementations | **default methods** of the trait |
| Derived classes | `struct LobbyMode` etc. + `impl InGameSpaceMode` |
| Template method of the base | `InGameSpace::tick()` calls the hooks in order |

```rust
// Common data: "members of the base class"
pub struct InGameSpaceCore {
    pub key: InGameSpaceKey,
    pub players: HashMap<PlayerId, PlayerPresence>, // proxy object built from CombatProfile (document 03, 1.1)
    pub aoi: AoiGrid,
    pub npcs: NpcTable,
    pub dirty: DirtyTracker,
}

// Per-mode behavior: "pure virtual functions"
pub trait InGameSpaceMode: Send {
    fn kind(&self) -> InGameSpaceKind;
    fn on_enter(&mut self, core: &mut InGameSpaceCore, player: PlayerId, out: &mut Outbox)
        -> Result<(), EnterError>;
    fn on_leave(&mut self, core: &mut InGameSpaceCore, player: PlayerId, out: &mut Outbox);
    fn on_link_event(&mut self, core: &mut InGameSpaceCore, player: PlayerId,
                     event: &LinkEvent, out: &mut Outbox);
    fn on_tick(&mut self, core: &mut InGameSpaceCore, dt: Duration, out: &mut Outbox);

    fn persistence(&self) -> Persistence { Persistence::None }
    fn on_mode_command(&mut self, _core: &mut InGameSpaceCore, _cmd: ModeCommand,
                       _out: &mut Outbox) -> Handled { Handled::No }
}

// The abstract class's place: bundles common + mode and fixes processing order
pub struct InGameSpace {
    core: InGameSpaceCore,
    mode: Box<dyn InGameSpaceMode>,
}

impl InGameSpace {
    pub fn tick(&mut self, dt: Duration, out: &mut Outbox) {
        self.core.aoi.begin_tick();
        self.mode.on_tick(&mut self.core, dt, out);
        self.core.flush_dirty(out);
    }
}
```

- **Protected access becomes a parameter.** A mode receives common data as a `core: &mut InGameSpaceCore` argument. Where a mode touches common data is visible in the signature, and since `core` and `mode` are separate fields, the borrow checker allows borrowing both at once.
- **Adding a new mode does not modify existing code.** Create one file and implement the trait.
- **If a mode misses handling something, it does not compile.** Every mode must implement the required methods.
- Fixed small sets (`LinkEvent`, `CloseReason`, packet kinds) stay as enums, and modes handle them exhaustively with `match`.
- `Outbox`: a buffer that accumulates "outgoing things" produced during this processing (packets to send to sessions, messages to content, notifications to InGameSpaceManager). The InGameSpace only accumulates results; actual sending is done by ZoneHost. That is what makes unit testing possible without a network.

File layout:

```
crates/game/src/space/
├─ mod.rs          # InGameSpace (template method), InGameSpaceMode trait
├─ core.rs         # InGameSpaceCore
├─ factory.rs      # InGameSpaceKey/kind → Box<dyn InGameSpaceMode>
└─ modes/
   ├─ lobby.rs
   ├─ field.rs
   ├─ dungeon.rs
   ├─ guild_dungeon.rs
   └─ pvp.rs
```

### 4.4 Connection event policy (per-mode `on_link_event`)

The Gateway reports only facts (`LinkEvent`, document 02 section 4.3); game consequences are decided by the mode.

| Mode | Detached / Suspended | Resumed / Replaced | Closed |
|---|---|---|---|
| Lobby | Nothing | Send snapshot | Leave |
| MMO field | Stays in place. Invulnerable or hidden after N seconds (design data) | Send snapshot | Leave |
| Instance dungeon | Mark "disconnected" for the party. Whether to leave as-is or let AI fight is a design decision | Snapshot, notify party of return | Leave, notify party |
| Guild dungeon | Same as dungeon | Same | Same |
| PvP field | Notify the opponent + **forfeit-loss timer** (e.g., 30 seconds) | Cancel timer, snapshot | Forfeit loss confirmed |

- Game-side timers (forfeit loss, etc.) are measured by the mode itself, **separate** from the Gateway's grace period.
- `Replaced` (duplicate login handover) means the client started from scratch, so it **always sends a full snapshot**.

### 4.5 Tick loop

```rust
// server crate: zone-N thread body (sketch)
fn run_zone_host(mut host: ZoneHost, rx: Receiver<ZoneCommand>, period: Duration) {
    let mut next_tick = Instant::now() + period;
    loop {
        // 1) Process commands as they arrive until the next tick (with a cap)
        let mut handled = 0;
        while handled < MAX_COMMANDS_PER_TICK {
            let now = Instant::now();
            if now >= next_tick { break; }
            match rx.recv_timeout(next_tick - now) {
                Ok(ZoneCommand::Shutdown) => return host.shutdown(),
                Ok(cmd) => { host.dispatch(cmd); handled += 1; }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return host.shutdown(),
            }
        }

        // 2) Tick: only awake InGameSpaces
        host.tick_active_spaces(period);
        host.flush_outbox();   // try_send to GatewayLink/content/InGameSpaceManager

        // 3) Next tick time. If behind, skip instead of catching up
        next_tick += period;
        let now = Instant::now();
        if now > next_tick {
            tracing::warn!(overrun_ms = (now - next_tick).as_millis(), "tick overrun");
            next_tick = now + period;
        }
    }
}
```

- Commands are processed on arrival; the tick runs at a fixed period.
- There is a cap on commands processed per tick (`MAX_COMMANDS_PER_TICK`) so the tick never starves.
- If the tick falls behind, it skips rather than bursting. Every overrun logs a warning and is counted in metrics.

### 4.6 A dormant InGameSpace does not tick

| List | Condition | Tick |
|---|---|---|
| active | one or more players | every tick |
| dormant | no players, in unload grace period | none. When the grace period ends, notify InGameSpaceManager that it is empty |

Time progression of lobby NPCs (schedules, respawn, etc.) is not simulated every tick. On wake, elapsed time from "the last processed time" to now is computed in one pass.

**Even inside an awake InGameSpace, sleep is applied per AOI cell.** A wide field stays awake even if players are all on one side. So NPC AI updates only in AOI cells containing players and their neighboring cells; NPCs in the other cells pause. When a cell becomes active again, elapsed time is computed in one pass as above. The point is to spend no CPU on a monster patrolling where nobody looks. The AOI itself is a 2D grid. When a cell changes, only newly visible and newly invisible targets are computed and announced.

### 4.7 Load measure

- A zone thread's load is measured not by the number of InGameSpaces but by **the sum of "per-kind weight × player count"**.
- Each ZoneHost records its load (weight sum, previous tick duration) in an atomic variable (`AtomicU64`) every tick. InGameSpaceManager reads only this value when placing.

### 4.8 Actions that must wait for a result: put only that player into a pending state

Some zone actions cannot advance to the next step until a result arrives from elsewhere (DB task, player actor, another server). Examples: equip confirmation (document 03 section 4.2), transfer to another server (6.6), instance entry confirmation. In these cases, **do not stop the whole zone or the thread; put only that player's proxy object into a pending state.** This is the same shape as the C++ server pattern of submitting a request to a DB worker and continuing in the completion callback.

```rust
// game crate: fields of PlayerPresence (sketch)
pub enum Pending {
    None,
    Equip { request: RequestId, deadline: Instant },
    Transfer { request: RequestId, deadline: Instant },
}
```

| Rule | Content |
|---|---|
| Same-kind request while pending | Reject and reply "processing" to the client. This prevents a double-clicked equip from going out twice |
| Other actions while pending | Decided per action. Default: allow movement and combat, reject actions that touch the same data |
| Result arrival | Apply only when the `RequestId` matches the one currently pending. A stale result arriving after the player left or a different request took over is discarded |
| Timeout | Check `deadline` in the tick, release the pending state, and reply with failure. For requests that may or may not have been applied to the DB, check again with the idempotency key (5.5) |

The key point is that confirmations which can take long are never awaited inside the zone tick. The zone keeps running; only that one action of that one player waits.

---

## 5. Content: per-player async actors

### 5.1 Why async

- Content (inventory, currency, mail, quests) has almost no computation; it is **mostly waiting on the DB**. Per chapter 1 principle 3 ("work that is mostly waiting is async"), it is handled async.
- The zone (InGameSpace) is about computation and fixed ticks, so it stays a synchronous thread (chapter 4). The split **game simulation is synchronous, waiting is async** stays clean.
- In C++, you threw a DB job and received the result in a callback. That approach splits one feature into a request function and a callback function, and multiplies waiting states in multi-step flows. In an async actor, **one feature is one function**, and `.await` sits where the DB is awaited.

### 5.2 Structure

```
[Content runtime: dedicated tokio multi-thread runtime, Kc workers (content-N)]

  ContentManager (1 actor)
     - PlayerId → PlayerContent actor handle
     - Actor creation (login, messages addressed to offline players), destruction (logout)
     - Routes messages arriving on inter-server links to the player's actor

  PlayerContent actors × connected count (+ briefly woken offline players)
     - Sole owner of one player's content state
     - Takes commands one at a time from its own inbox (bounded) and processes them
```

- A **dedicated tokio runtime for content** is kept separate from the I/O runtime. A spike in network throughput does not delay content processing, and vice versa. The worker count `Kc` is a config value.
- With 10,000 players there are 10,000 actor tasks. For tokio that is lightweight (a few KB per task).
- Battle servers have no content runtime (2.2).

### 5.3 Player actor

```rust
async fn run(mut self, mut inbox: mpsc::Receiver<ContentCommand>) {
    while let Some(cmd) = inbox.recv().await {     // take commands one at a time
        match cmd {
            ContentCommand::Sell { item, req } => self.sell(item, req).await,
            ContentCommand::Grant(grant) => self.grant(grant).await,     // grants from zone/battle server
            ContentCommand::Logout => { self.drain_and_save().await; break; }
            // ... (own enum, so no _ => arm)
        }
    }
}

async fn sell(&mut self, item: ItemInstanceId, req: RequestId) {
    let price = self.tables.price_of(item);
    match db::sell_item(&self.db, self.id, item, price, req).await {   // while waiting, the worker does other players' work
        Ok(SellOutcome::Sold) => { /* update memory → reply to client (Reliable) */ }
        Ok(SellOutcome::ItemNotFound) => { /* reject reply */ }
        Ok(SellOutcome::GoldOverflow) => { /* reject reply */ }
        Ok(SellOutcome::AlreadyProcessed) => { /* idempotent: reply with the earlier result */ }
        Err(e) => { /* 5.5 rules */ }
    }
}
```

- **Per-player ordering is guaranteed structurally.** An actor must finish one command before taking the next. Two purchases by the same player can never interleave and drive currency negative. Other players' actors keep running meanwhile.
- There are no locks inside an actor. State is touched only by the actor itself via `&mut self`.
- The actor holds handles to the DB (`GameDb`), Redis (KV), and outgoing inter-server sends directly, and uses them with `.await`. The DB task in 3.2 remains the **window for the synchronous side or other actors to use the DB**, as with zone threads and InGameSpaceManager.

### 5.4 Incoming paths

| Sender | Receiver | Method |
|---|---|---|
| Client Content messages | That player's actor | GatewayLink's routing table holds `SessionId → actor handle` and sends directly. Handles are registered with `RouteUpdate` at game entry |
| Zone thread on the same server | That player's actor | The zone holds a `ContentAddr` (below) on the player's proxy object and sends directly |
| Another server (battle server's `GrantItem`, trade service's `Deliver`, visitor rewards from a friend's lobby on another shard) | ContentManager → that player's actor | ContentManager forwards messages received on inter-server links |
| **Messages addressed to offline players** (mail, trade grants, guild rewards) | ContentManager **briefly wakes** the actor | Create the actor, load what is needed from the DB, process, and bring it down after a while. Same path, so there is one idempotent processing code path |

```rust
enum ContentAddr {
    Local { actor: PlayerContentHandle },   // same server: actor handle
    Remote { server: ServerId },            // other server: ServerLink → that server's ContentManager
}
```

- When sending from a zone thread (synchronous), use `try_send` (chapter 7). The zone does not wait. If a result is needed, the reply returns to the zone thread's input channel (e.g., `GrantResult`).
- The inbox is bounded. When full, follow the sender-side rules (chapter 7).

### 5.5 DB processing rules

| Item | Rule |
|---|---|
| **Write timing** | Currency and items are applied **after commit** (write-through). Only after the DB succeeds do we change memory and reply to the client. InGameSpace state (lobbies, etc.) uses the zone side's **periodic saving** (write-behind, 6.4) |
| **Transactions** | One function in the `db` crate is one transaction (guide chapter 8). The actor `.await`s that function |
| **Timeouts** | Queries use PostgreSQL `statement_timeout`; the whole function uses `tokio::time::timeout`. On timeout the outcome is **unknown**, so re-run with the same idempotency key to check the result |
| **Retries** | Deadlock, serialization failure, and disconnection re-run the whole transaction (exponential backoff, up to N times). Other errors are not retried; reply with failure |
| **Idempotency keys** | Client requests (purchase, sale, reward claim) and inter-server messages (`GrantItem`, `Deliver`) all record the idempotency key in the same transaction. Re-running applies only once |
| **Concurrency cap** | The connection pool size is the concurrent DB work limit (document 04 chapter 3). If the pool is full, wait; past the wait limit, fail + metric |
| **Outbox** | Messages to other servers are recorded in an outbox table in the **same transaction** as the effect, and sent after commit. Deleted on acknowledgment (document 03 chapter 3) |
| **Logout** | After **fully finishing** in-flight commands, save remaining state and bring the actor down. Never cut midway |
| **Client replies** | Sent exactly once, after commit. Retransmissions of the same request are blocked by the combination of the Gateway's seq deduplication (document 02 chapter 7) and the idempotency key |

### 5.6 What must not be done inside an actor

- **Blocking calls** (synchronous file I/O, `std::thread::sleep`, heavy computation). They stall content runtime workers. Use `spawn_blocking` if unavoidable.
- **Holding a lock across `.await`** (guide 6.2).
- **`.await`ing a reply from another player's actor.** Two actors waiting on each other deadlock. Anything entangling players (trading, gifting) goes through the idempotent message flows of document 03, with replies arriving in its own inbox.

### 5.7 Three ways to call the DB (one implementation)

DB processing code is written **exactly once as async functions** in the `db` crate (guide chapter 8). It is used in **three ways** depending on the caller's situation. The same query is never written twice, sync and async.

| Way | Flow | Who uses it | Shape |
|---|---|---|---|
| **A. direct async call** | Wait for the result, then the next line (sequential) | Player actors, Entry server, trade service, other async code | `db::sell_item(&db, ...).await` |
| **B. request/reply messages** | Send the request and move on. The result arrives later on the input channel | **Zone threads** (synchronous, tick must not stop), InGameSpaceManager | `db_task.try_send(DbRequest::SaveSpace { .. })` → later `ZoneCommand::DbResult` |
| **C. blocking call** | The **thread blocks** until the result arrives | Initial load at server start (design data validation, migration version check), operations tools and batch jobs (CLI), tests | `blocking.run(db::load_shard_config(&db))` |

- **A** is sequential in flow but does not stop the thread. While waiting, the worker does other work.
- **B** is the same as C++'s "submit DB job + receive result job". The DB task in 3.2 receives it, executes it as A, and sends the result back.
- **C** is the same as a blocking DB call in C++. Internally it runs the same async function with a dedicated tokio runtime's `block_on`.

**Where C must not be used (the compiler cannot stop it, so reviews do)**

| Forbidden location | Reason |
|---|---|
| Zone thread | The tick stops. Use B |
| Inside async code (player actors, I/O tasks) | It stalls a tokio worker, and calling `block_on` inside a runtime panics. Use A |

C is used only "before the game is running" (startup) or "outside the game" (tools, batch, tests). The blocking wrapper type (`BlockingDb`) is constructible only in binaries and startup code for this purpose. When startup code finishes, `BlockingDb` is discarded and never placed into `ServerContext` (chapter 8). Then code that runs during the game has no way to call C at all.

**Operations commands and reload also never use C.** If an operations command reused a startup blocking function, every zone on the server would freeze while the command runs. When shared data (market prices, event schedules, etc.) must be re-read during service, we do the following.

1. The task that received the `/admin` request (document 08 chapter 2) reads the new data with A. Meanwhile the game keeps running on the old data.
2. When the read finishes, package the new data as a read-only `Arc` snapshot and send it as a message to the zone threads and actors that use that data.
3. The receiver swaps the whole reference on its own turn. A partially-updated state never exists mid-swap.

In C++ terms, it is like building the new table as a `std::shared_ptr<const T>`, handing it over, and the receiver swapping the pointer on its own thread. The old table is freed automatically when the last reference goes away.

### 5.8 Actors and threads, debugging and tracing (owner decision, 2026-11-19)

**Decision.** Player actors are not pinned to a specific OS thread. Any worker of the content runtime (tokio multi-thread, Kc workers) may run one, and after pausing at `.await` it may resume on a different worker. Two workers never run the same actor simultaneously, so no locks are needed. If work piles up on one worker, the other workers share it.

**Thread-local storage and memory pools.** Both are usable. There is one rule: never carry something taken out of `thread_local!` across an `.await`. You cannot use `.await` inside `with(|v| ...)`, so the compiler enforces this rule. Per-user data lives inside the actor, not in thread-locals.

**Tracing rules.** Thread numbers cannot identify a user, so all tracing is keyed on **user ID and request ID**.

| Rule | Content |
|---|---|
| Log context | While an actor processes one command, the `tracing` span carries `player_id`, `request_id` (command number), and the command name. Every log emitted inside it (including DB calls and message sends) gets these values attached automatically. Inter-server messages also carry `request_id` so logs on the receiving side connect |
| Marking wait points | Open a child span at every `.await` wait, such as DB calls or requests to other actors or servers. Logs show **what** a paused actor is waiting on |
| Slow command warning | If one command exceeds a threshold (config value, e.g., 1 second), log a warning without stopping. The log contains `player_id`, the command name, and the current wait point. If the same actor keeps exceeding it, warn again at each multiple of the threshold |
| Inbox backlog warning | Warn when the number of commands waiting in the inbox exceeds a threshold. This catches an actor that stalled or a command flood |
| Metrics | Command processing time, inbox wait count, slow command count (document 08 chapter 3) |

**Preventing and finding deadlocks.** The actor structure uses no locks, so C++-style lock deadlocks cannot occur. What can occur instead is "two actors each waiting for the other's reply".

- **Prevention:** never `.await` a reply from another player's actor (5.6). Send entangled work as idempotent messages and receive replies in your own inbox. Channel sends are `try_send`, so a full peer inbox never blocks you (chapter 7).
- **Finding:** if an actor still stalls, the "slow command warning" above tells you which player's command is waiting on what. If two actors wait on each other, the two warnings point at each other's `request_id`.
- **Development tool (optional):** tokio-console shows the state of every running task (running, waiting, how long). It is an external crate (`console-subscriber`), so the owner's approval is required before use.
- **Panic:** release aborts immediately with a core dump (10.2). Trace using the dump's stack and the `player_id`, `request_id` of the most recent logs.

---

## 6. Finding and placing InGameSpaces: two-layer structure

Because an InGameSpace may live on another server ("which server is my friend's lobby on"), finding and placing splits into two layers.

| Layer | Location | Role |
|---|---|---|
| **Directory** | **`space:{key}` in the Redis-compatible store** (document 03, 9.6). No separate service | A public bulletin board that records only **who is the host**. TTL lease means a dead host's entry disappears on its own |
| **InGameSpaceManager** | Each Game/battle server (one task on the I/O runtime, an actor) | **The owner of decisions.** Lifetime state (Loading / Active / Unloading) of InGameSpaces this server hosts, entry waiting queue, lease renewal, zone thread placement, creation and destruction |

In C++ terms, this splits the work a world server's ZoneManager used to do. "Who owns it" is written on the Redis bulletin board where everyone can see. Decisions inside the owning server are processed one at a time through that server's InGameSpaceManager actor job queue. Decisions are never delegated to Redis (document 03, 9.1).

### 6.1 InGameSpace key

```rust
pub enum InGameSpaceKey {
    Lobby(LobbyOwner),                // Personal(PlayerId) | Community(CommunityId)
    MmoField(FieldId, Channel),
    Dungeon(InstanceId),
    GuildDungeon(GuildDungeonId),
    Pvp(MatchId),
}
```

Only one InGameSpace with the same key exists **in the whole cluster** at a time. Redis's **preemption (`SET ... NX`) and TTL lease** guarantee this, and the narrow gap that could appear in a one-in-a-thousand case is blocked by the **DB's fencing number** (6.4).

### 6.2 Lifetime states (managed by the host's InGameSpaceManager)

```
Redis:  space:{key} = { host, epoch }        ← who is the host (bulletin board)
host memory: lifetime state + entry queue      ← decisions (actor)

            entry request (no key → preempt)
  [none] ───────────────────────────> [Loading] ──load done──> [Active] ──empty──> [Unloading] ──save committed──> [key deleted]
                                          │                        ▲                    │
                                          │ entry requests during    │ entry requests during │
                                          └─ load queue in the host   └─ unload are held by the host until deletion, then re-preempted
```

- **Preemption**: get a fencing number with `INCR space_epoch:{key}`, then `SET space:{key} "{host, number}" NX PX 15000`. The server that succeeds is the host. On failure a host already exists, so `GET` it and forward the entry request there.
- Entry requests arriving **during loading** are queued by the **host's InGameSpaceManager** and processed together when loading finishes. The same lobby is never loaded twice.
- **During unloading**, the key is not deleted until the final save is committed to the DB. So during that time **no server** loads it anew (prevents a write-then-read race). After commit, `GET` to confirm we are still the host, then `DEL`.
- **Lease renewal**: the host checks every 5 seconds with `GET` that it is still the host and extends with `PEXPIRE` to 15 seconds. If the host dies the key disappears, and the next entry request preempts on another server and loads from the DB again.
- **If renewal check shows a different host, or renewal fails because Redis is unreachable**, that InGameSpace **stops accepting new entries on its own.** Play already inside continues; if a different host appeared, players are cleaned up (document 03, 9.10).
- State transitions (Loading → Active → Unloading) happen in exactly one place, the host actor, so race conditions are structurally impossible.

### 6.3 Entry flow (example: a lobby, a stateful InGameSpace)

```
Requesting server (e.g., player's home shard) ──GET space:{key}──> Redis
   [no key] → choose a server to place it (lobby: owner's shard; dungeon/PvP: least-loaded battle server by server:{id})
             ──CreateSpace(key, player)──> that server's InGameSpaceManager
                    InGameSpaceManager: INCR space_epoch + SET space:{key} NX PX  → preempt success = host
                                        (on failure, forward the request to the host that already appeared)
                    InGameSpaceManager ──Load──> DB task ──> data
                    choose the idlest zone thread (4.7) → create → Active (in memory)
   [key exists] ──EnterSpace(key, player)──> host's InGameSpaceManager (queued if Loading)

Home shard content ──CombatProfile──> host zone thread
host ──SwitchZone(session, host)──> Gateway   (only when different servers, document 02 chapter 6)
host ──RouteUpdate(session, zone)──> the corresponding GatewayLink
```

- Preemption is done **by the server that will become the host itself**. So the server that wins preemption and the server that actually hosts the InGameSpace are always the same.
- Zone threads wait on neither the DB nor Redis. Loading is done by the DB task, preemption and renewal by the KV task, and results arrive as messages.
- MMO fields are created at server startup and preempt their `space` keys, so they are Active from the beginning. Instance dungeons and PvP are created directly on battle servers with no DB load.

### 6.4 Save policy and fencing (lobby, guild dungeon)

- The InGameSpace marks changes dirty and issues DB save requests through the Outbox.
  - Important results (interactions tied to reward grants): save **immediately**
  - Ordinary state: **periodic** (config value, e.g., 30 seconds)
  - On unload: **final** save. Delete the key only after the completion ack
- Save requests carry two numbers.
  - **Fencing number** (the `space_epoch` received at preemption): the DB records the last accepted fencing number per InGameSpace and **rejects saves with a lower number**. Even if an old host lingers briefly through a lease-renewal gap (document 03, 9.6), its saves are discarded.
  - **Version number** (monotonically increasing within the same host): even if saves from the same host arrive out of order, a lower version is ignored.
- Conditional saves are implemented as queries of the form `UPDATE ... WHERE fencing <= $mine AND version < $mine`, with zero affected rows treated as rejection (guide chapter 8. No SPs).
- Results of visitor actions are split per document 03 section 6.2. Lobby state goes to the owner DB (local save); visitor rewards go as messages to the visitor's home shard.

### 6.5 Guild dungeon expiry

- The expiry time (`expires_at`) is **based on the DB value**. The period survives server restarts.
- A guild dungeon checks the expiry time in its tick, and when it passes, removes players and notifies InGameSpaceManager that it has expired.
- If the expiry time passes while unloaded, the server attempting preemption on the next entry attempt reads the DB expiry time and refuses. Cleanup is done by a separate batch job.

### 6.6 Transfer

- **Within the same server** to another InGameSpace: the source InGameSpace removes the player and hands them over to the destination zone thread via InGameSpaceManager. The player's proxy object exists **in exactly one place** during the transfer (Rust's move). Send `RouteUpdate` to the GatewayLink.
- **To another server**: the source removes the proxy object; the destination builds a new one from the `CombatProfile` sent by the home shard. The Gateway's zone destination changes (document 02 chapter 6).

---

## 7. Inter-thread communication rules

| Direction | Channel | Sender action | When full |
|---|---|---|---|
| GatewayLink → zone thread | `std::sync::mpsc::sync_channel` (bounded) | `try_send` (`send` forbidden from async, it blocks) | Kick request to that session + warning log |
| GatewayLink → player actor | `tokio::sync::mpsc::channel` (bounded) | `try_send` | Kick request to that session + warning log |
| Zone thread, player actor → GatewayLink | `tokio::sync::mpsc::channel` (bounded) | `try_send` | **Treated as a gateway link failure.** Slow-client handling is the Gateway's job, so a full channel here is not a normal situation. Error log + metric |
| Zone ↔ player actor (same server) | peer's input channel (actor inbox, zone input channel) | `try_send` | Error log + metric. Reliable-natured messages (`GrantItem`, etc.) are retried from the sender's outbox |
| Zone, player actor → ServerLink | `tokio::sync::mpsc::channel` (bounded) | `try_send` | Same as above |
| Zone, InGameSpaceManager → DB task (player actors `.await` the DB directly) | `tokio::sync::mpsc::channel` (bounded) | `try_send` | **Critical.** Generous capacity with utilization monitoring. When full, error log + retry wait |
| GatewayLink routing updates | GatewayLink control channel | `try_send` (`RouteUpdate`) | Error log. Packets during transfer are forwarded by the source zone, so consistency holds |

- All channel capacities are config values.
- The only point where a zone thread blocks is `recv_timeout` on its own input channel. Player actors never block; they wait with `.await`.
- Locks (`Mutex`) are not used for inter-thread communication. Read-only shared data (design tables) is shared via `Arc`; load metrics via atomic variables only.

---

## 8. Passing handles: ServerContext

Handles to managers and services are not global singletons. Instead, **one struct collecting the handles** is passed through constructors. It is explicit (dependencies are visible in constructors) and easy to extend (a new manager adds one field).

```rust
#[derive(Clone)]
pub struct ServerContext {
    pub config: Arc<Config>,
    pub data_tables: Arc<DataTables>,          // design data (read-only)
    pub space_manager: InGameSpaceManagerHandle,
    pub kv: KvHandle,                          // Redis-compatible store (Directory, server registration, etc.)
    pub db: DbHandle,
    pub zones: Arc<[ZoneSender]>,              // zone thread input channels
    pub contents: ContentManagerHandle,        // content runtime's ContentManager
    pub links: ServerLinksHandle,
}
```

`main` creates it once and passes `ctx.clone()` when creating each task and thread. "Only one exists" is guaranteed by `main` creating it exactly once.

---

## 9. Sizing (Game server, 10,000 concurrent / 10Hz, initial assumptions)

| Item | Assumed value | Rationale |
|---|---|---|
| Server cores | 16 | Assumption. Adjust after measurement |
| Zone threads Kz | 6 | Lobbies, fields, guild dungeons (dungeons and PvP go to battle servers) |
| Content runtime workers Kc | 2 | Little computation, mostly waiting on the DB. While waiting, workers run other actors |
| I/O workers | 8 | Deserialization, links, DB |
| Tick budget | **50ms or less** of the 100ms | The rest for command processing and spikes |
| Active lobbies | up to 10,000 | A dormant lobby costs 0 ticks (4.6) |

Battle servers have no content runtime, so cores are split between zone threads and I/O workers (e.g., 16 cores → Kz 10, I/O 6). All values are tuned after load testing with the bot crate.

---

## 10. Startup, shutdown, and failures

### 10.0 Startup order (Game server)

On a game server, the only legitimate place for thread-blocking DB calls is the startup phase: calls that prepare or recover shared data like guilds, market prices, and seasons. This order is not left to code flow and comments alone. It is exposed as a stage list, and only when all stages complete does the server accept players (**startup barrier**).

1. `main` reads config. Secrets come from environment variables or a secret store, not config files (document 08 chapter 8).
2. Connect to the DB and Redis and check the migration version (C style).
3. **Read shared data in dependency order** (C style). The order is written in code as a stage list. Example: design data → shard shared state → features depending on that state (seasons, schedules). Each stage returns `Result`; no stage is allowed to ignore failure.
4. **Startup recovery**: process things that should have happened while the server was down (settling finished events, season rollover). One recovery is **one transaction**, or has a **per-run idempotency key** (e.g., event ID + round). That way, dying mid-recovery and coming back up never pays out twice. Calling event settlement, currency refunds, and history recording separately loses this guarantee.
5. The loaded results are passed as initial values when creating the zone threads and actors that own that data. `BlockingDb` is discarded at this point (5.7).
6. Start the runtimes, zone threads, and ContentManager, and register the server in Redis (document 03, 9.7).
7. Turn on `/health/ready` (document 08 chapter 2). From this point players can come in.

**Readiness failure policy**: if any of steps 1~6 fails, startup ends in failure and the process terminates. The supervisor restarts it, and the failed stage is recorded in the error log. Partial startup like "serve with some features disabled" is not allowed. If any stage can ignore failure, the server could enter service not ready.

### 10.1 Normal shutdown order (Game server)

1. `main` receives the termination signal (SIGTERM, Ctrl+C).
2. Send a shutdown notice to the Gateways. Each Gateway sends maintenance guidance to players in this server's zones and proceeds with transfer or cleanup.
3. Set `server:{id}` in Redis to `Draining`. No new InGameSpaces are placed on this server (document 03, 9.7).
4. Send `ZoneCommand::Shutdown` to every zone thread. Each InGameSpace issues its final save request and ends its loop.
5. Send shutdown to ContentManager. Each player actor finishes in-flight commands completely, saves, and comes down (5.5). Unsent outbox messages remain in the DB (resent after restart).
6. `join` all zone threads, wait for every actor in the content runtime to finish, then shut the runtime down.
7. Wait until the DB task commits all remaining requests (with a timeout; on timeout, list unsaved items in the error log).
8. Close inter-server links and shut down the runtime.

Shutdown signal propagation uses a `CancellationToken` (I/O runtime side) and `Shutdown` commands (zone threads, ContentManager side).

### 10.2 Panic

- Set `panic = "abort"` in the release profile. A panic immediately terminates the process and leaves a core dump; the supervisor (systemd, etc.) restarts it.
- Panic paths (`unwrap`, `panic!`) are forbidden by the guide, so a panic is by definition a bug.
- InGameSpace changes since the last save may be lost. Currency and rewards are never lost because player actors apply them **only after DB commit** (5.5). The content outbox lives in the DB, so sending resumes after restart.
- If a battle server dies, its dungeons and PvP matches are gone. Rewards are delivered directly to home-shard player actors; behavior when the server dies before acknowledgment follows document 03 section 3.4.

---

## 11. Follow-up documents

| Document | Content |
|---|---|
| 05 Space lifecycle management (**draft written**, `05-space-lifecycle.md`) | Redis Directory (`space:{key}`) preemption, lease times, fencing, InGameSpaceManager details, unload grace period, MMO field channels (replication on capacity overflow), guild dungeon creation rights and count limits |
| 06 Cluster | Redis-based server registration and discovery (document 03, 9.7), scale in/out, drain, Redis redundancy and behavior on failure |

---

## 12. Guide rules this design upholds

- Game state is owned solely by one thread. InGameSpaceManager is also an actor, and decisions belong to actors. Redis records only "who owns it" (section 6.1).
- Never hold a lock guard across `.await`. Zone threads have no `.await` at all, and player actors have no locks (section 6.2).
- Never block in async code. GatewayLink uses only `try_send` (section 6.3).
- All channels are bounded (section 6.3).
- The `game` crate knows no I/O. InGameSpace only accumulates results into the Outbox (chapter 2).
- DB calls are never made directly from zone threads. Player actors are async, so awaiting the DB directly never affects ticks (chapter 8).
- Handles are passed through constructors instead of global singletons (chapter 14).
