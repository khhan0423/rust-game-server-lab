# Design 04. Database Layout

**English** | [한국어](04-database.ko.md)

- Status: **Draft** (2026-11-18). D1 through D7 are final. Reflects the separation of the publisher LoginDB and the game Account DB.
- Prerequisite: `03-data-ownership.md` (data ownership and the one DB per step rule)
- Related: `docs/RUST_GUIDELINES.md`, section 8 (PostgreSQL, `sqlx::query!`, no stored procedures, transactions in Rust)

## 0. Decision Summary

| # | Decision | Details |
|---|---|---|
| D1 | Database product | **PostgreSQL** for all databases |
| D2 | Database types | Publisher domain: **LoginDB** (publisher accounts, document 07). Game domain: **Account** (game accounts, one globally), **Game** (per shard), **Log** (per shard), **Common** (information shared across all servers, one globally) |
| D3 | Sharding scope | Shard **only Game DB and Log DB**. There is one Account DB and one Common DB globally. LoginDB is the publisher's responsibility |
| D4 | Connection rule | A server has connection pools **only for databases its role needs**. **It never connects to another shard's Game DB** |
| D5 | Guilds | Guilds live in **Common DB**. **One guild actor** handles decisions and writes for each guild (section 5) |
| D6 | Friends | Friendships span **all servers globally**. Friend relationships live in **Common DB** |
| D7 | Trading | Trading spans **all servers globally**. Trade state lives in the trade table in **Common DB** (owned by the trade service) |
| D8 | Log routing | Send game analytics and operational logs through **files → Fluent Bit → routing rules**. Game servers never connect directly to Log DB. Only transactional logs are written in the same Game DB transaction (1.1) |

---

## 1. Database Types

```
  [Publisher domain]  ┌──────────────┐
                      │   LoginDB    │   Publisher accounts, sign-in methods, owned games, membership status (owned by the Login server)
                      └──────────────┘
  [Game domain]
                      ┌──────────────┐
                      │  Account DB  │   One globally. Publisher AccountID → character in this game, home shard
                      └──────────────┘
        ┌──────────────┐  ┌──────────────┐  ┌──────────────┐
        │  Game DB 1   │  │  Game DB 2   │  │  Game DB N   │   Per shard
        │  Log DB 1    │  │  Log DB 2    │  │  Log DB N    │   Per shard
        └──────────────┘  └──────────────┘  └──────────────┘
                      ┌──────────────┐
                      │  Common DB   │   One globally (shared across all servers)
                      └──────────────┘
```

| DB | Count | Contents | Write owner |
|---|---|---|---|
| **Login** (publisher) | 1 at the publisher | Publisher accounts, sign-in methods (guest, Google, Apple, Steam), owned games, membership status, platform sanctions. See document 07, section 2.4, for the schema | Publisher Login server (the game does not connect) |
| **Account** | 1 globally | **Publisher AccountID (unique key) → character in this game (`PlayerId`, one) and home shard** = source of truth for the player directory; in-game sanctions. See document 07, section 6, for the schema | Game Entry server |
| **Game** | Per shard | Player content (inventory, wallet, equipment, quests, mail), lobby state, transactional logs (changes to items and currency), idempotency keys, outbox | That shard's Game server (player actors, lobby host) |
| **Log** | Per shard (split according to Fluent Bit routing settings) | Game analytics logs (connections, movement, combat statistics, and so on) | **Fluent Bit** writes to it. Game servers do not connect directly (1.1) |
| **Common** | 1 globally | Information shared by all servers: **guilds**, guild dungeon state, friend relationships, cross-shard trade state, shard list and settings, seasons, archived rankings, announcements | The actor that owns each category (section 5) |

### 1.1 Three Log Types and Their Routes

Follow `08-operations-observability.md` for code rules (`tracing` for operational logs and typed events for analytics logs) and the Fluent Bit setup (agent + aggregator, Log DB partitioning).

| Type | Examples | Route | Reason |
|---|---|---|---|
| **Transactional logs** | Item creation and deletion, currency changes | **Game DB, same transaction** (bypasses Fluent) | A sale must never go unlogged. These records support customer service |
| **Game analytics logs** | Connections, movement, combat statistics, item acquisition analytics | Server writes **only to JSON files** → **Fluent Bit** collects them → **Log DB** or file archive, according to the rules | High volume. Loss does not affect the game |
| **Operational logs** | Errors, warnings, tick overruns (`tracing`) | Server writes **only to JSON files or standard output** → Fluent Bit → **file archive** (default). **No writes to any database.** If search becomes necessary, a dedicated log store such as Loki is optional (document 08) | Used to check server health |

```
Game server ──(transactional logs, same transaction)──────────────> Game DB
Game server ──(analytics logs)   → game-analytics.jsonl ─┐
Game server ──(operational logs) → ops.jsonl ────────────┤
                                                        ▼
                                             Fluent Bit (one per server)
                                             Routing rules by tag (config file)
                                               ├─ analytics.* ──> Log DB (pgsql output plugin, JSONB)
                                               ├─ analytics.* ──> File archive (compressed, long-term retention)
                                               └─ ops.*       ──> File archive (no DB writes; optionally use Loki or similar if search is needed)
```

- **Game servers do not connect directly to Log DB.** They just write logs to files. The **Fluent Bit configuration** determines where the logs go. Changing or adding log destinations (files, Log DB, or a future analytics store) therefore requires no changes to game code.
- Slow logging or a Log DB outage does not affect the game. A separate task buffers file writes, and Fluent Bit resends from its own buffer (a filesystem buffer is recommended).
- Every log line contains common fields such as a **type tag** (`log_type`), shard, server ID, timestamp, and `player_id`. Routing and queries use these fields.
- The Fluent Bit PostgreSQL output plugin stores each log line in one JSONB column in a table. If structured tables are needed for analytics, transform the data periodically within Log DB.
- Analytics logs have **at-least-once** delivery, so duplicates are possible. This is fine for analytics. Records that must be exact go in transactional logs (Game DB).

---|---|---|---|
| **Transactional logs** | Item creation and deletion, currency changes | **Game DB, same transaction** | A sale must never go unlogged. These records support customer service |
| **Analytics and behavior logs** | Connections, movement, combat statistics | **Log DB** | High volume. Loss does not affect the game |

Analytics logs do not block game processing. Collect them and write them in batches through a separate connection pool. If Log DB slows down or stops, drop excess records when the buffer fills and record only a metric.

---

## 2. Database Access by Server

| Server | Account | Game | Common | Log |
|---|---|---|---|---|
| **Entry server** | **Read/write** | - | Read (shard list, maintenance settings) | - |
| **Gateway** | - | - | - | - |
| **Game server (shard N)** | - | **N only** | Read/write (only categories it owns, such as guild actors and friends) | - |
| **Battle server** | - | - | Read (settings) | - |
| **Trade service** | - | - | Read/write (trade table only) | - |
| **Fluent Bit** | - | - | - | **Write** |

All game servers write logs only to files. Fluent Bit alone writes to Log DB (1.1).

**Mapping between shards and Game servers (owner approval, 2026-11-19)**

```
Game server 1 ── Game DB 1   (shard 1)
Game server 2 ── Game DB 2   (shard 2)
Game server 3 ── Game DB 3   (shard 3)
```

- Start with **one Game server per shard (1:1)**. Game server N connects only to Game DB N.
- A player permanently belongs to the shard assigned on first entry (document 07, section 4). Only that shard's Game server writes the player's data.
- For interactions involving a player from another shard (mail, rewards, visit rewards), send an idempotent message to that shard's Game server (document 03, section 3). Players from any shard can play together in dungeons and PvP in the battle server pool. Rewards return to each player's home shard.
- Document 06 on clustering will specify how to run two or more Game servers on one shard.

- **No game server connects to the publisher LoginDB.** The game communicates with the publisher only through the publisher API (ticket validation; document 07, section 2.5).
- **Gateway does not use a database.** The Entry server signs and issues the game's `login_token`. Gateway only verifies the signature. Connection state lives in Redis (document 03, section 9.4).
- **Battle servers do not write to Game DB** (document 03, section 3.4). They write analytics logs to files just like other servers. Fluent Bit determines which Log DB receives them.
- When a Game server looks up the player directory, it checks the Redis cache (`home:{player}`) first. On a miss, it **asks the Entry server**. The Game server does not connect directly to Account DB (the Entry server owns Account).

### 2.1 Why Servers Do Not Connect to Other Shards' Databases

- One player actor on the player's home shard owns the player's data (document 03, section 2). If another server modifies that database directly, the state in memory and the state in the database fall out of sync.
- Cross-shard operations use **idempotent messages** (document 03, section 3). To reward a player on shard 2, send `GrantItem` to server 2 instead of writing to DB 2.
- If every server opens a pool to every shard database, PostgreSQL connections quickly run out.

---

## 3. Connection Pools

There is no dedicated DB thread pool. `sqlx` is an asynchronous driver, so waiting for queries does not block the thread. The **connection pool size** determines how many queries can run concurrently (the DB task in document 01 runs on the I/O runtime).

| C++ (blocking driver) | Rust + sqlx (asynchronous) |
|---|---|
| The thread blocks while waiting for a query → a DB thread pool is needed | The thread does other work while waiting → only a connection pool (`PgPool`) is needed |
| Concurrent queries = number of DB threads | Concurrent queries = connection pool size |

### 3.1 Settings (Initial Values, Assumptions)

| Pool | Size per server | Notes |
|---|---|---|
| Game DB | 16 ~ 32 | The number of Game servers per shard × pool size must fit within that database's `max_connections` |
| Common DB | 4 ~ 8 | Keep it small because every Game server connects |
| Account DB (Entry server) | 8 ~ 16 | Used only on entry, so it does not need to be large |

| Setting | Value |
|---|---|
| Connection acquisition wait limit | A few seconds. Return an error and record a metric if exceeded |
| Query timeout | Set with PostgreSQL `statement_timeout` |
| When the number of servers grows | Put **PgBouncer** (a connection pooling proxy) in front of the database |

- The **player actor** preserves the order of a player's database operations. The connection pool does not enforce it (document 01, section 5.3: an actor finishes one command before taking the next, so transactions for the same player never run concurrently).
- Use `tokio::task::spawn_blocking` only in the exceptional case where a blocking library is required. This does not apply to PostgreSQL + sqlx.

### 3.2 Use a Separate Type for Each Database

With several pools all typed as `PgPool`, the compiler would accept a shard query sent to the Common pool by mistake. Wrap each database pool in its own newtype.

```rust
pub struct AccountDb(PgPool);
pub struct GameDb(PgPool);
pub struct CommonDb(PgPool);

pub async fn sell_item(db: &GameDb, ...) -> Result<SellOutcome, DbError>;   // Accepts only GameDb
// sell_item(&ctx.common_db, ...)  → Compile error
```

- Each server role's `ServerContext` (document 01, section 8) contains **only the database types that role uses**. The battle server context has no `GameDb`, so battle server code that writes to Game DB does not compile.
- Divide modules in the `db` crate by database as well (`db::game`, `db::common`, `db::log`, `db::account`).

---

## 4. Migrations

- Keep a separate migration directory for each database type: `migrations/account/`, `migrations/game/`, `migrations/log/`, `migrations/common/`.
- Apply **the same Game DB migrations to every shard**. All Log DBs also use the same schema (the JSONB table Fluent Bit writes to plus tables for transformation). The deployment tool upgrades all shards in sequence to prevent schema drift.
- Generate the `sqlx` offline data (`.sqlx/`) for compile-time checks separately for each database type.
- For deployments without downtime, split schema changes into **add → switch the code → remove** (do not immediately drop or rename a column).

---

## 5. Common DB Write Owners

All servers share Common DB, so assign **one write owner per category**. Servers that are not the owner only read or send a message to the owner.

| Category | Write owner | Notes |
|---|---|---|
| **Guild** (details, members, ranks, guild currency) | **Guild actor** (one per guild) | 5.1 |
| Guild dungeon state | Host of the guild dungeon InGameSpace | Creation requires permission from the guild actor. Saves include fencing (document 01, section 6.4) |
| Friend relationships | The component that handles friend requests and acceptances | Handle one relationship row with a conditional write. Notify both players |
| Cross-shard trade | **Trade service** | Document 03, section 6.3 |
| Shard list, settings, seasons, announcements | Operations tool | Servers only read |

### 5.1 Guild Actor

- **One guild actor** makes decisions for a guild (joining, leaving, rank changes, capacity, spending guild currency, and permission to create guild dungeons). It processes them one at a time through a Job queue. It works like the party actor (document 03, section 9.8), except that state is **stored permanently in Common DB**.
- While any guild member is connected, the guild actor runs on **any Game server**. It records its location in Redis with `SET guild:{id} server NX PX` and renews it periodically. This uses the same mechanism as claiming an InGameSpace (document 03, section 9.6).
- When the guild actor writes to Common DB, it includes a **fencing number** (obtained with `INCR guild_epoch:{id}`). The database rejects delayed writes from a guild actor still running on the old server.
- When all guild members leave, the actor saves and shuts down. It starts again the next time someone connects.
- The guild actor determines **rewards that individual players earn from guild activity** (guild rewards, attendance rewards, and so on). It sends the grant to the player's home shard as an idempotent message (document 03, section 3). Player inventories are not stored in Common DB.

#### 5.1.1 Guild Content Scope (Owner Decision, 2026-11-19)

| Category | Details | Storage |
|---|---|---|
| Capacity | **About 30 members** (configurable) | Common DB |
| Guild storage | Members can deposit and withdraw both **guild currency and items**. Rank permissions and daily limits restrict withdrawals. Record every deposit and withdrawal | Common DB (storage, deposit and withdrawal records) |
| Guild town decoration | Add later. Build it as a guild-only space (InGameSpace) with placement data | Common DB |
| Guild rankings | The guild actor manages the guild score (influence) and publishes the leaderboard to Redis `rank:{board}` (document 03, section 9.3). Common DB is the source of truth for scores | Common DB + Redis (display only) |
| Events that occur when no guild members are online | For events that happen as they come in, such as membership applications, briefly wake the guild actor to handle them. Scheduled work such as weekly settlement runs as a batch job | - |

**How items are deposited and withdrawn.** The player's inventory is in the home shard's Game DB. Guild storage is in Common DB. Do not put both databases in one transaction. Follow "one DB per step + idempotent messages" in document 03, section 3.

```
Deposit:  player actor ─ Game DB: remove item + outbox (idempotency key) ──message──► guild actor ─ Common DB: add to storage + idempotency key
Withdraw: guild actor  ─ check permissions and limits, Common DB: remove from storage + outbox ──message──► home shard player actor ─ Game DB: grant
```

In both directions, the item is "removed from one place, then added to the other." Even if a process dies partway through, outbox retries and the idempotency key ensure the item is eventually added exactly once.

### 5.2 Guild Dungeon Placement Change

Guilds no longer belong to a particular shard, so select **any Game server with a low load** to host the guild dungeon InGameSpace rather than using "the guild's shard" (document 03, section 5). Load state from Common DB and save it there.

### 5.3 Sharding, Common DB, and Load

**Sharding applies to individual player data.** Split high-volume data that is independent per player, such as inventories, wallets, and quests, across Game DBs. **Relationships that connect players across shards (guilds, friends, and trades)** cannot be split, so keep them in one Common DB. Keeping only social features and trading global is a common hybrid design in sharded games.

Guilds, friends, and trading all use the same Common DB, so this database may be the first bottleneck as concurrent users grow.
- Start with one Common DB.
- If load becomes a problem, **split by feature first** (trade table → separate Trade DB, guilds → separate Guild DB). If that is still insufficient, **partition by ID** (guild ID for guilds, player ID for friend relationships). The data IDs are UUIDs and each table has one write owner (section 5), which makes partitioning manageable.
- Trading involves currency, so even if Common DB is split, **move the trade table and the trade service together**.

---

## 6. Remaining Decisions

| # | Question | Notes |
|---|---|---|
