# Design 03. Player Data Ownership and Cross-Shard Play

**English** | [한국어](03-data-ownership.ko.md)

- Status: **Draft** (2026-11-18). O7~O10, O12, O14 confirmed. Combat server rewards (O8) confirmed as A+D+E after a case study. Redis confirmed as a "shared bulletin board" using single commands only, no Lua
- Prerequisite document: `02-session-network.md` (Gateway, two destinations, inter-server links)
- Related documents: `01-threading.md` (player actor, zone thread, InGameSpace structure)

## 0. Decision Summary

| # | Decision | Content |
|---|---|---|
| O1 | Separation of data and presence | Player **data never moves from the home shard.** Only the player's **presence** moves into another server's InGameSpace (chapter 1) |
| O2 | Content and zone | Permanent data is **content** (player actor on the home shard), transient simulation is the **zone** (zone thread of the InGameSpace host) (chapter 2) |
| O3 | Core principle | **One DB per step.** The next step is handed over with a message carrying an idempotency key, and the message is recorded before sending (outbox). No distributed transactions (2PC) (chapter 3) |
| O4 | One-way and two-way | One-way delivery (looting, rewards) uses **direct messages.** Two-way exchange (trading) goes through an **intermediary (escrow)** (3.3) |
| O5 | Placement | An InGameSpace with permanent state lives on **the shard that owns that state**; one-shot InGameSpaces go to the **combat server pool.** All instanced dungeons and PvP use the combat server pool (chapter 5) |
| O6 | Cross-shard features allowed | Cross-shard trading, mixed-party dungeons, visiting and contributing to another shard friend's lobby are **all allowed.** However, all of them are built on top of the single mechanism of O3 (chapter 6) |
| O7 | Global IDs | Data IDs (`PlayerId`, `GuildId`, item instances, idempotency keys) are **UUID v7.** The home shard is carried along with the session and the `CombatProfile`; offline targets are looked up via the player directory (chapter 7) |
| O8 | Combat server rewards | Important rewards: **send only the result and the home shard computes** (D). Drops are **delivered directly to the home shard player actor** (A, in-memory retransmission + idempotency key). If the inventory is full, use the **mailbox** (E). Only the case where the combat server dies before receiving acknowledgment is treated as a dungeon failure. Going through Redis Streams is optional (3.4) |
| O9 | Lobby visitor permissions | Design defines the permission items and defaults, and **the owner configures them per visitor.** The configuration lives in the owner's DB (6.2) |
| O10 | MMO field visits | Players from other shards may enter the channel where their friend is **only when following that friend** (chapter 5) |
| O12 | Shared key-value store | **A Redis-compatible store = a bulletin board every server reads together.** Used for the address book (TTL auto-cleanup), session mirror, queues and rankings, Pub/Sub, and cache. **Decisions belong to actors, truth and consistency belong to the DB.** Only single commands are used with Redis, no Lua (chapter 8, 9) |
| O14 | Redis failure policy | Redundancy (Sentinel or Cluster) is mandatory. When it stops, **block new starts and keep ongoing play** (9.10) |

---

## 1. Home Shard and Host Shard

| | The player's **home shard** | The InGameSpace **host** |
|---|---|---|
| Meaning | Where the player data lives | Where the InGameSpace runs |
| Does it change | **It never changes** | Different for each InGameSpace |
| What it holds | Inventory, currency, equipment, quests, personal progress | Position, combat state, monsters, the InGameSpace's own state |
| Server | Game server (shard) + Game DB | Game server (lobby, field, guild dungeon) or combat server (dungeon, PvP) |

### 1.1 Presence is created as a proxy object

When a player enters an InGameSpace, the host gets a **combat proxy object**. The proxy object is built from the **`CombatProfile`** sent by the home shard content.

```rust
struct CombatProfile {
    player: PlayerId,
    home_shard: ShardId,       // Home shard. Where rewards and results are sent back (7.2)
    version: u64,              // Increases whenever equipment changes (4.2)
    stats: CombatStats,        // Final stats reflecting equipment, buffs, etc.
    visuals: EquipmentVisuals, // Appearance
    // Only information usable during combat. The full inventory is not sent
}
```

- **It is built the same way when entering an InGameSpace on the same server.** If local and remote are not distinguished, the code path becomes one, and cross-shard play stops being a special case.
- Stat calculation lives in a single pure function of the `game` crate (`compute_combat_profile`) and is called **only in content.** The zone receives only the computed result. This prevents the two places from computing different values.

---

## 2. Separation of Content and Zone

| | Content | Zone |
|---|---|---|
| What it handles | Inventory, currency, shop purchases, mail, quest rewards (the guild itself is the guild actor, document 04 section 5.1) | Movement, combat, HP, monsters, looting decisions, NPC interaction |
| Character | **Permanent data.** The DB is the truth | **Transient simulation** |
| Location | Fixed on the home shard | InGameSpace host (moves) |
| Execution unit | Player actor (async, document 01 chapter 5) | Zone thread (synchronous) |
| Message category | Content (`0x1000` ~ `0x7FFF`) | Zone (`0x8000` ~ `0xFFFF`) |

The categories and routing of client messages follow `02-session-network.md` section 3.2. The Gateway sends Content messages to the home shard and Zone messages to the current host.

### 2.1 Split the owner of each decision

Some actions involve both content and zone (looting, equipping). In that case, **split the action into decision units and give each decision a single owner.**

| Decision | Owner |
|---|---|
| Who gets this drop (first come, party distribution) | **Zone** |
| May equipment be changed right now during combat (forbidden while casting, etc.) | **Zone** |
| Is there room in the inventory, does the player own this item | **Content** |
| DB record | **Content** (home shard DB) |
| Apply the equipment change result to combat | **Zone** |

So every boundary-crossing action follows the flow **"zone decides first → content confirms (DB) → zone applies the result."**

---

## 3. Core Principle: One DB per Step

### 3.1 Rules

1. **No step ever touches two DBs at once.** No distributed transactions (2PC).
2. A step writes to one DB only, and the next step is handed over with **a message carrying an idempotency key.**
3. Messages are **recorded before sending** (outbox) and retransmitted until the counterpart acknowledges.
4. The receiver filters duplicates by idempotency key. Guaranteed by a unique constraint in the DB.

In finance terms, this is not a distributed transaction but **a saga + message numbers (idempotency keys) + retransmission of unsent messages.**

### 3.2 Common foundation (built once)

```
[Common foundation]  inter-server transport (document 02, 4.4) + outbox + idempotent processing
     │
     ├─ One-way delivery: direct messages (looting, rewards, visitor rewards, mail delivery)
     │
     └─ Two-way exchange: trade service (intermediary, escrow) + deposits and grants via the direct messages above
```

| Component | Content |
|---|---|
| outbox | Store outgoing messages first. The home shard, which has a DB, uses the DB's outbox table; combat servers without a DB use an in-memory outbox (loss window in 3.4) |
| Idempotency key | Generated by the message creator. UUID v7 (chapter 7) |
| Receive processing | Recording the key in the DB and applying the effect happen in **the same DB transaction.** If the key already exists, reply with the previous result as-is |
| Retransmission | After link reconnection, or when no acknowledgment arrives within a certain time, retransmit with the same key |

### 3.3 One-way and two-way

| Flow | Example | Intermediary | Reason |
|---|---|---|---|
| **One-way delivery** | Looting, dungeon rewards, visitor rewards | Not needed | The giving side has already decided; the receiving side just receives |
| **Two-way exchange** | Item trading, item ↔ currency | **Needed** | "Both happen or neither happens" must be guaranteed |

Finance analogy: one-way delivery is a **plain transfer**, two-way exchange is **DVP (Delivery Versus Payment)** through a settlement institution.

### 3.4 Messages sent by the side without a DB (combat server) (confirmed: A + D, supplemented by E)

The combat server has no DB and its state lives only in memory. After comparing the studied approaches (direct delivery, via queue, direct DB write, result-only delivery, mailbox), the following combination was chosen.

| Reward type | Method | Path |
|---|---|---|
| **Important rewards** (clear rewards, paid currency, season rewards) | **D. Send only the result, home shard computes** | The combat server sends only `DungeonResult { result_id, cleared, records }` to the home shard. The home shard player actor rolls the rewards and grants them in the DB |
| **Combat drops** (ground looting) | **A. Direct delivery** | Combat server → inter-server link → home shard player actor (`GrantItem`, 4.1) |
| When the inventory is full (important rewards) | **E. Mailbox** | The home shard grants via mail instead of the inventory. Drops return to the ground as before (4.1) |

**Rules of direct delivery (A)**

- The combat server sends a message as soon as it is created and retransmits from the **in-memory outbox** until it receives the home shard's acknowledgment.
- Every message carries an idempotency key (`grant_id`, `result_id`), so even if retransmitted, it is applied exactly once in the home shard DB (3.2).
- If the home shard is unreachable because it is restarting, the combat server keeps holding and retrying. When the link recovers, it continues sending.

**Remaining loss**

- If the combat server dies before receiving the acknowledgment, that message is lost. Since the dungeon progress itself is lost at that moment, it is **treated as a "dungeon failure"** (whether the entry ticket is refunded is an operations policy).
- This is a rare bug-driven scenario, so we do not pay the cost of an extra queue hop now. If loss turns out to be a real problem in operation, the option of **going through Redis Streams (9.9)** remains available.

**Why direct DB writes (C) are not used**

If the combat server writes directly to the player's shard DB, the DB diverges from the inventory the player actor holds in memory. It would break the principle that "the owner of player data is the home shard player actor alone" (chapter 2), so it is not used.

---

## 4. Boundary-Crossing Flows

### 4.1 Looting

```
Client ──Loot(drop_id)──> Zone (InGameSpace host. May be a combat server)
Zone:  "Reserve" the drop for this player (so others cannot take it)
Zone ──GrantItem { grant_id, player, item, source }──> Player's home shard content
Content: check inventory room → DB record (grant_id unique)
Content ──GrantResult { grant_id, Ok | InventoryFull }──> Zone
Zone:  Ok → remove the drop      InventoryFull → release the reservation, restore the drop
Content ──InventoryChanged (Reliable)──> Client
```

- If party members are on different shards, `GrantItem` goes separately to each home shard.

### 4.2 Equip and unequip (in the zone)

```
Client ──Equip(item_id, slot)──> Zone (Zone category)
Zone:  check combat rules (while casting, cooldown, no-equipment-change area) → reject immediately if not allowed
Zone ──EquipRequest { req_id, player, item_id, slot }──> Home shard content
Content: ownership check → equipment change in DB → compute_combat_profile
Content ──CombatProfile { version, stats, visuals }──> Zone
Zone:  apply if version is newer than the current one (takes effect from the next tick, broadcast appearance to surroundings)
```

- Rapid equipment changes can interleave reply order, so the zone applies **only the newer version.**
- Items purchased in the zone: purchase is a Content message handled on the home shard; once it lands in the inventory, equipping follows the flow above. The client sends the equip request after receiving the purchase response (order across categories is not guaranteed, document 02 section 3.2).

### 4.3 When latency matters: checkout (lease)

The flows above require an inter-server round trip each time. Within the same datacenter this is a few milliseconds, which is enough for looting and equipping. For cases where latency is a real problem, such as spamming consumables in fast PvP, use the **checkout approach.**

| | A. Home shard stays the owner (default) | B. Lease part of the data to the zone on entry |
|---|---|---|
| Method | Confirmation request to the home shard each time | On entry, check out N consumables to the zone. The zone deducts locally and settles on exit |
| Latency | Inter-server round trip | None |
| On failure | Simple. Retry with idempotency key | Needs settlement rules (lease expiry, unsettled recovery) |
| Finance analogy | Per-transaction authorization | Pre-authorization (hold) then settlement |

**Use A as the default, and move to B only for items that measurement shows are latency-bound.**

---

## 5. InGameSpace Placement Rules

**An InGameSpace with permanent state lives on the shard that owns that state; a one-shot InGameSpace lives in the combat server pool.**

| InGameSpace | Host | Reason |
|---|---|---|
| Lobby | Fixed to **the owner's shard** | Lobby state is stored in the owner's DB (data locality) |
| Guild dungeon | **A Game server with low load** | Guilds belong to no shard and live in the Common DB. State is loaded from and saved to the Common DB (document 04 chapter 5) |
| MMO field | Each shard (channels) | Public field per shard. Players from other shards may enter the channel where their friend is **only when following that friend** (O10) |
| Instanced dungeon | **Combat server pool** (always, regardless of party composition) | One-shot. No persistent state |
| PvP field | **Combat server pool** | One-shot |

**Why all instanced dungeons run in the pool.** Splitting into "same-shard parties on the shard, mixed parties in the pool" creates two paths, and bugs hide in the rarely-exercised one. Running everything in the pool means the cross-shard paths (looting, equip messages) are **used every day in every dungeon**, so they stay continuously verified. Combat load is also separated from shards, so you can respond by scaling the pool alone. The cost is that same-shard parties also use inter-server messages, which is imperceptible within the same datacenter.

Finding and placing InGameSpaces becomes a cluster-level concern ("which server is the friend's lobby on"). Hence the two-layer structure: **a cluster-level Directory (Redis `space:{key}`, 9.6) + an in-process InGameSpaceManager.** Details are covered in documents 05 (space lifetime) and 06 (cluster).

---

## 6. Cross-Shard Features

### 6.1 Mixed-party dungeon

Solved by the placement rules of chapter 5. Each member's home shard sends a `CombatProfile` to the combat server, and looting and rewards return to each member's home shard.

### 6.2 Visiting and contributing to another shard friend's lobby

The lobby runs on the owner's shard. **Split a visitor's action into pieces** and write each piece to a single owner DB.

| Result of the visitor's action | Where it is recorded | Method |
|---|---|---|
| Lobby state change (e.g. helping the owner's farm) | **The owner's DB** | Local write (because the lobby runs on the owner's shard) |
| Visitor's personal rewards, quest progress | **The visitor's DB** | Idempotent message to the visitor's home shard (same flow as 4.1) |
| Picking up an item in the lobby | The visitor's DB | 4.1 looting flow |

- The lobby state is **the truth and is recorded first.** Visitor rewards arrive **later and reliably** as a consequence. The two do not need to be atomic.
- **Visitor permissions (confirmed: the owner configures per visitor)**
  - Design defines the **available permission items** (e.g. helping, gathering, harvesting, NPC conversation) and the **defaults for new visitors.**
  - The lobby owner toggles items per visitor (friend). The configuration is stored in **the owner's DB** as part of the lobby state.
  - The server checks `(visitor, permission item)` for each visitor action. A visitor without a configuration follows the defaults.
  - If the owner changes permissions during a visit, it is reflected immediately in the lobby InGameSpace, because the lobby runs on the owner's shard and the configuration lives in the same place.

### 6.3 Cross-shard item trading

The two shard DBs are never changed at once. The **trade service (cluster service; trade state lives in the Common DB trade table, document 04 D7)** acts as the intermediary (escrow).

```
1. A's home shard:  move A's items to "trade escrow" (A's DB only)  ──Deposit(trade_id)──> trade service
2. B's home shard:  move B's items to "trade escrow" (B's DB only)  ──Deposit(trade_id)──> trade service
3. Trade service:  confirm both deposits → record "settled" (Common DB trade table only)
4. Trade service ──Deliver(trade_id)──> A's home shard: give B's items to A (A's DB only)
               ──Deliver(trade_id)──> B's home shard: give A's items to B (B's DB only)
   Cancel or timeout → return each party's own items (Return)
```

Trade state machine (recorded in the Common DB trade table):

```
Open ──both Deposits──> Settled ──both Deliver confirmations──> Completed
  │
  └─cancel, timeout──> Cancelling ──Return of deposits confirmed──> Cancelled
```

- Every message carries `trade_id`, so retransmissions are applied exactly once.
- Even if a server dies, the trade resumes from the state in the trade table after restart. The trade history serves as evidence for customer support.
- If delivery (Deliver) is **implemented as mail delivery**, trading, the auction house, and operator reward grants share the same delivery mechanism.

### 6.4 Implementation order

Common foundation (3.2) → mixed-party dungeon (6.1) → lobby visit (6.2) → trading (6.3). Trading is implemented last, but its design is confirmed now under this principle.

---

## 7. Global IDs (confirmed: UUID)

If IDs that cross shards were auto-increment values of shard DBs, player 100 on shard 1 and player 100 on shard 2 would collide in the same InGameSpace. Therefore **data IDs are UUIDs.**

| ID | Format |
|---|---|
| `PlayerId`, `GuildId`, `AccountId`, `CommunityId` | **UUID v7** |
| Item instance IDs (can cross shards via trading) | **UUID v7** |
| Idempotency keys (`grant_id`, `trade_id`, `req_id`) | **UUID v7** |
| `SessionId`, `ConnId`, `ServerId` (infrastructure IDs, not data) | As before (document 02 sections 2.1, 4.4) |

### 7.1 Why UUID v7

- UUIDs come in several versions. The fully random **v4** inserts at random positions in DB indexes, causing frequent page splits and worse write performance.
- **v7** starts with the creation timestamp (milliseconds), so it **increases monotonically over time.** Like auto-increment keys it appends to the end of the index, which greatly reduces the UUID indexing performance penalty. The creation timestamp can be read directly from the ID, which is also useful for operations queries.
- The Rust `uuid` crate supports v7 generation. Add it to the dependency list in chapter 13.
- In the DB it is stored as a **16-byte binary value**, not a string, and on the wire (Protobuf) it is sent as `bytes` (16 bytes). The string form is used only in human-readable logs.

### 7.2 What choosing UUID means: the home shard cannot be derived from the ID

Unlike schemes that embed the shard number in the ID, the home shard cannot be known from the UUID alone. "Which shard should this reward go to" must be found out elsewhere.

| Situation | How the home shard is known |
|---|---|
| A connected player (most cases) | Determined at login and carried along. The Gateway uses `Routes.content` (document 02 section 3.2), the InGameSpace uses `CombatProfile.home_shard` (1.1) |
| Sending to an offline player (mail, trade delivery, guild rewards) | **Player directory** (chapter 8): `PlayerId → home shard` lookup. The truth is the game Account DB (owned by the Entry server), cached in the Redis-compatible store (`home:{player}`, 9.3) |

- The player directory is written once at character creation and changes only on server transfers or merges. Reads dominate, so caching fits well.
- There is an upside too. Even with server merges or character server transfers, **the ID does not change.** Only the value in the player directory needs updating.

---

## 8. Cluster Services

What a single shard cannot see is handled outside the shard. The Redis-compatible store (chapter 9) is used as a "shared bulletin board", with **our actors making decisions and the DB holding the truth.** Separate service processes are kept to a minimum.

| Function | Who decides | What is written to Redis | Truth |
|---|---|---|---|
| InGameSpace Directory | The host server's **InGameSpaceManager actor** | Who is the host (`space:{key}`), fencing number (9.6) | Redis (transient state) |
| Server registration and discovery | Each server itself | List of live servers (9.7) | Redis (transient state) |
| Account directory (duplicate login) | The new Gateway's **session handling code** (judges from the old value returned by `SET ... GET`) | Which Gateway and session the account is on (9.4) | Redis (transient state) |
| Player directory (`PlayerId → home shard`) | Entry server | Cache (7.2) | **Game Account DB** (document 04, document 07 chapter 6) |
| Guild | **Guild actor** (one per guild, document 04 section 5.1) | Which server the guild actor is on | **Common DB** |
| Party | **Party actor** (one on the server that created the party) | Which server the party actor is on (9.8) | Party actor's memory (transient state) |
| Friend relations | Home shard content | Online presence only | **DB** |
| Chat | - | Pub/Sub channels (9.8) | No general chat persistence (volatile). What needs archiving goes to the DB |
| Matchmaking, login queue | Matching actor / Gateway | Sorted sets (9.8) | Redis (transient state) |
| Trade service (cross-shard exchange) | **Service process** (6.3) | - | **Common DB** trade table (document 04 D7) |

Only trades that need the truth in the DB and have a long state machine remain as a service process. Everything else exists as code (actors) inside Game servers and Gateways.

---

## 9. Shared Key-Value Store (Redis-compatible): Shared Bulletin Board

- Status: **Confirmed** (2026-11-18). Settled on an approach that does not use Lua scripts.

### 9.0 One-line definition

**Redis is a bulletin board that every server reads together.** Facts are posted and shared, and if the writer dies, its posts are wiped automatically by TTL. Data structures like ordering (sorted sets) and fan-out to many servers (Pub/Sub) come as a bonus.

| Layer | Responsibility | Where |
|---|---|---|
| **Decision** | Who wins, is the capacity full, state transitions | **Our actors** (session actor, party actor, InGameSpaceManager, etc.) |
| **Truth** | Currency, items, inventory, trades, outbox, idempotency keys | **DB** |
| **Consistency guarantee** | Duplicate-application prevention, blocking late writes from an old owner | **DB** (idempotency keys, fencing numbers) |
| **Shared facts** | "Where is it", temporarily entrusted data, queues, notifications, cache | **Redis** |

This design stands even without Redis. In that case you must build the address book, queues, rankings, chat relay, and session storage as services yourself and implement redundancy for each. Redis replaces those several services with one already-verified, redundant component (build vs buy).

The product (Redis 8.x or Valkey) is decided at the operations stage (O13). The commands we use are the same for both.

### 9.1 Principles

1. **Never use it as the truth.** Redis disk writes (AOF) are usually every 1 second, so the last second can be lost on failure, and replication is asynchronous, so the most recent write may be missing after failover.
2. **Never make decisions inside Redis. Use single commands only.** No Lua scripts, no `MULTI`/`EXEC`. When "read, decide, write" is needed, the **actor** that owns that decision does it.
   - Reason: Lua scripts stop the whole Redis while running, have no rollback on mid-script errors, and the Lua engine itself has a history of security vulnerabilities (e.g. CVE-2025-49844). Putting decisions in Rust actors gets compiler help and allows testing without Redis.
   - Commands used: `GET`, `SET` (with `NX`, `GET`, `PX` options), `DEL`, `PEXPIRE`, `INCR`, hashes, lists, sorted sets (`ZADD`, `ZPOPMIN`, etc.), Pub/Sub. Each single command is atomic.
3. **Design so that missing Redis values degrade to the more conservative path.** Without a session mirror, use Resync instead of Replay; if a space lease is gone, reload from the DB.
4. **Every key has a TTL or an explicit deletion path.** If the owner dies, the key eventually disappears too.
5. **The narrow gap between two commands is fenced by the DB.** Example: the owner changes between a lease renewal (`GET` check then `PEXPIRE`). Data writes carry a fencing number and the DB rejects writes with an old number (9.6).

### 9.2 What Redis does

| # | Role | Used for |
|---|---|---|
| 1 | **"Where is it" shared address book** (auto-cleanup by TTL) | An account's connection location, a session's current Gateway, an InGameSpace's host, a party actor's location, list of live servers |
| 2 | **Data temporarily entrusted between servers** | Session mirror (Reliable retransmission buffer) |
| 3 | **Ordering and ranking** (sorted sets) | Matchmaking queue, login queue, rankings |
| 4 | **Fan-out to many servers at once** (Pub/Sub) | Chat, friend connection notifications |
| 5 | **Cache** | A player's home shard, online presence, other-shard player profiles |

### 9.3 Key list

| Key | Value | TTL / deletion | Purpose (section) |
|---|---|---|---|
| `online:{account}` | `{gateway, session}` | Renewed periodically by the Gateway | Duplicate login (9.4) |
| `sess:{session}:owner` | The Gateway currently holding the session | Grace period plus margin | Session takeover (9.5) |
| `sess:{session}` | `{routes, last_server_seq, last_client_seq, ...}` | Same as above | Session mirror (9.5) |
| `rbuf:{session}` | List: unacknowledged Reliable frames (with original seq) | Same as above. Removed from the front on ACK | Session mirror (9.5) |
| `space:{key}` | `{host, epoch}` | Renewed periodically by the host (lease) | InGameSpace Directory (9.6) |
| `space_epoch:{key}` | Integer (fencing number) | Never deleted (monotonically increasing) | Blocking late writes from an old host (9.6) |
| `server:{id}` | `{role, addr, load, state}` | Renewed periodically by the server | Server registration and discovery (9.7) |
| `home:{player}` | Home shard | Cache TTL | Player directory cache (7.2) |
| `presence:{player}` | Connection status, position summary | Short TTL, renewed | Friend online presence |
| `party:{party}` | Server where the party actor lives | Renewed periodically by the party actor | Party location (9.8) |
| `mm:{queue}`, `loginq:{shard}` | Sorted sets | Per-item expiry cleanup | Matchmaking, login queue (9.8) |
| `rank:{board}` | Sorted set | Deleted at season end | Rankings (source scores in the DB) |
| `bsout:{shard}` | Streams | Cleaned after consumption | (Optional) combat server reward outbox (9.9) |

Keys are prefixed with the environment and cluster prefix (e.g. `live:c1:`).

### 9.4 Duplicate login: a single `SET ... GET` command

```
New login → new Gateway:
  SET online:{account} "{my gateway, new session}" GET PX 30000
    → writes the new value and returns the old value (atomic, one command)
  No old value → first connection
  Old value exists → Kick the old Gateway (inter-server link) → duplicate login procedure of document 02 section 3.1
```

- Late-arriving old processing (e.g. a cleanup request the old Gateway belatedly sends) is filtered out **by comparing SessionId.** Each new login gets a new SessionId, so no separate number is needed.
- The Gateway periodically extends the key TTL of the connected account. If the old Gateway dies, the key disappears via TTL.
- If the last write is lost during a Redis failover, the decision can very rarely be wrong. Even then, the Player exists on exactly one Game server and handoff goes through the server holding the Player, so **the character is never duplicated.** The worst case is the old device disconnecting late.

### 9.5 Session mirror: reconnect through any Gateway

Session information and the **Reliable retransmission buffer** are entrusted to Redis. Even if the original Gateway dies or scales in, another Gateway can take over the session and **Replay** it. Reliable messages are rare ones like purchases and rewards (document 02 section 7.1), so Redis write volume is small.

**Write order (Redis first, then transmit):**

| Direction | Order | Reason |
|---|---|---|
| Server → client Reliable | Gateway assigns seq → append to `rbuf` → **then** transmit to the client | Every frame the client could have seen is in Redis, so any Gateway can Replay without gaps |
| Client → server Reliable | Gateway receives → update `sess.last_client_seq` → **then** forward to the backend | Resuming from another Gateway does not re-forward already processed requests |
| ACK received | Remove up to that seq from `rbuf` | Keep the buffer small |

Volatile messages are not written to Redis.

**Resuming from another Gateway:**

```
Client ──Resume(session_key, last_server_seq)──> Gateway G2
G2: SET sess:{session}:owner G2 GET   → returns the old owner G1 (one command)
G2 ──Superseded(session)──> G1        (inter-server link. Skip if G1 is already dead)
G1: clean up its session actor as Superseded (close connection, discard memory buffers)
G2: Replay from last_server_seq + 1 out of rbuf. If anything is missing, Resync
G2 ──SessionResumed { gateway: G2, mode }──> backend (update PlayerLink address)
```

- If the client reconnects to the original Gateway, Replay happens straight from memory. Redis is for **when the client comes to a different Gateway** and **when the original Gateway died.**
- `session_key` validation also uses the `sess` key, so any Gateway can validate it.

### 9.6 InGameSpace Directory: preemption, lease, fencing

"Which server holds which InGameSpace" is written to `space:{key}`. **The lifecycle state (Loading / Active / Unloading) and the entry queue are held and decided in memory by the host server's InGameSpaceManager actor** (document 01 chapter 6). Redis holds only "who is the host."

| Action | Commands |
|---|---|
| **Preempt** | ① Get a fencing number with `INCR space_epoch:{key}` ② `SET space:{key} "{my server, number}" NX PX 15000`. On success, I am the host. On failure, a host already exists, so `GET` the host and forward the entry request there |
| **Lease renewal** (host, every 5 seconds) | `GET space:{key}` → if it is me, `PEXPIRE space:{key} 15000` |
| **Release** (after unload save is committed to the DB) | `GET space:{key}` → if it is me, `DEL space:{key}` |
| **Entry request** | `GET space:{key}` → if present, go to the host; if absent, preempt |

- **If the host dies,** renewals stop, the key disappears via TTL, and the next entry request preempts on another server and reloads from the DB.
- **If a renewal check shows a different owner,** the host immediately stops new entries for that InGameSpace and drains its players. Even when renewal fails because Redis is unreachable, new entries stop (9.10).
- If `INCR` yields a number but preemption fails, one number is skipped. That is fine; the numbers only need to grow.

**Fencing: the DB blocks the narrow gap**

Lease renewal and release are two commands, `GET` and `PEXPIRE`/`DEL`. There is a **very narrow gap** in between where the key expires and another server preempts. For this gap to occur, the host must stall for nearly 15 seconds or be cut off from the network. Even then, data must not be corrupted.

- When saving InGameSpace state to the DB, the host sends **its own fencing number** along.
- The DB records the last received number per InGameSpace and **rejects saves with a lower number** (conditional `UPDATE ... WHERE epoch <= $mine`; zero affected rows means rejected. Guide chapter 8).
- So even if an old host lingers, its save is discarded. The worst case is the same lobby being visible on two servers for a few seconds; only the new host's data survives.
- In finance terms, **consistency is guarded by the DB while Redis only tells "who is responsible."**

### 9.7 Server registration and discovery

- Every server writes `server:{id} = {role, addr, load, state}` at startup and renews it periodically (TTL). `state` is `Active` or `Draining`.
- The Gateway uses this list to connect to backends, and backends also use it to form inter-server links (document 02 section 4.4).
- If a server dies, TTL removes it from the list. Scaling out is "start a new server and it registers itself."
- Scaling in switches `state = Draining` to block new placements, drains existing InGameSpaces, then exits (document 06).

### 9.8 Party, chat, queues

| Function | Method |
|---|---|
| **Party** | One **party actor** is created on the server that formed the party. Invites, accepts, leaves, leader handover, capacity checks are all decided serially in this actor's job queue. Only the location is written to Redis with `SET party:{id} server NX PX` and renewed periodically by the actor. If the server with the actor dies, the party disbands and members are notified (a party is transient state) |
| Online presence | `presence:{player}` TTL key. Shown combined with the friend list (DB) |
| Chat | Pub/Sub per channel. Each Gateway subscribes only to the channels its sessions belong to and relays. Only messages that may be dropped. Whisper archiving and records for reports go to the DB |
| **Matchmaking** | `ZADD` into the `mm:{queue}` sorted set (score = queue join time or MMR). The **matching actor** pops items (`ZPOPMIN` is a single command, so multiple poppers never get the same item twice), forms matches, and preempts a PvP InGameSpace on a combat server (9.6). Matching rules (MMR range widening, party units, region) change in the matching actor's code. It is **global matchmaking**, independent of shards |
| Login queue | When the concurrent user cap is exceeded, `ZADD` into `loginq:{shard}` and report the position (`ZRANK`). When a slot opens, admit from the front |

### 9.9 (Optional) Combat server reward outbox: Redis Streams

**Not used by default.** Direct delivery of combat server rewards to the home shard player actor is the default (3.4). This is an option to enable if reward loss from combat server failures becomes a real problem in operation.

When enabled: messages the combat server would send to the home shard (`GrantItem`, `DungeonResult`) are **first recorded in the Redis Streams `bsout:{home shard}`**, and the home shard reads them with a consumer group. Streams provide at-least-once delivery, so the home shard filters duplicates in the DB by idempotency key (3.2).

### 9.10 When Redis stops

Redis **must be deployed redundantly (Sentinel or Cluster).** Even so, the basic policy when it stops is **"block new starts, keep ongoing play."** Since neither decisions nor the truth live in Redis, ongoing play is unaffected.

| Function | When Redis stops |
|---|---|
| New login (9.4) | **Rejected.** Guide the user to "try again shortly" |
| Reconnect (9.5) | Reconnect to the original Gateway is handled in memory. Reconnect to a different Gateway gets `ResumeFail` → log in again (also after Redis recovers) |
| New InGameSpace entry (9.6) | **Rejected.** Play inside existing InGameSpaces continues. The host stops new entries if lease renewal fails |
| Server discovery (9.7) | Continue with the last known list. New servers cannot join |
| Party, chat, queues (9.8) | New creations and matching stop. Existing party actors keep running (only location renewal fails) |
| Combat server rewards (3.4) | **Unaffected.** The default path (direct delivery) does not use Redis |
| Content (purchases, inventory) | **Unaffected.** Uses the DB only |

### 9.11 Code structure

- Zone threads **never call Redis directly.** The I/O runtime's KV task handles it asynchronously and returns the result as a message. Player actors are async, so they `.await` the KV functions of the `db` crate directly (document 01 section 5.3).
- Redis access code lives **only in the `db` crate (storage layer)**, same as SQL. Commands are wrapped in per-key-kind functions (e.g. `claim_space(key, me) -> ClaimResult`) so no other crate can send arbitrary Redis commands.
- No Lua scripts, `MULTI`/`EXEC`, or `WATCH` (9.1).
- The Rust client (`redis` or `fred`) is decided at the implementation stage.

---

## 10. Remaining Decisions

| # | Question | Note |
|---|---|---|
| O11 | Which items use the checkout (4.3 B) | After load testing |
| O13 | Redis-compatible store product | **Confirmed (2026-11-19): Valkey.** The license (BSD) is simple. All usage is single commands, so it is compatible with Redis |
