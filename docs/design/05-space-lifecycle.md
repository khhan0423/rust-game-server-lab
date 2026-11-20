# Design 05. Space (InGameSpace) Lifecycle

**English** | [한국어](05-space-lifecycle.ko.md)

- Status: **Draft** (2026-11-19)
- Prerequisites: `01-threading.md`, section 6 (two-layer Directory and InGameSpaceManager structure, claiming, lifecycle states, fencing); `03-data-ownership.md`, sections 5 (placement rules) and 9.6 (Redis commands)
- Scope: **When to start an InGameSpace, when to unload it, and how many to allow.** Document 01, section 6, already covers who owns each space and how to prevent two servers from owning the same space.

## 0. Decision Summary

| # | Decision | Details |
|---|---|---|
| L1 | Unload grace period | How long an InGameSpace stays in memory after the last player leaves. **Configurable, 10 minutes by default.** This accounts for players who frequently disconnect on mobile networks or while streaming to a phone (owner decision, 2026-11-19) |
| L2 | Lobby visitors | Keep the lobby active while visitors remain, even if the owner leaves. The grace period starts **after everyone has left** (owner decision, 2026-11-19) |
| L3 | MMO field channels | Open another channel for the same field when capacity is reached. **The server assigns channels automatically**, giving priority to channels with party members and friends. Players can switch channels if they want (owner decision, 2026-11-19) |
| L4 | Permission to open guild dungeons | **The guild leader or an officer authorized by the guild leader** (owner decision, 2026-11-19). The guild actor checks permissions (document 04, section 5.1) |
| L5 | Guild dungeon count and types | **Up to 5 open at once per guild** (configurable). Types are day-specific dungeons (open only on that day), always-open dungeons, and two-day weekend special dungeons. Exact periods come from design data (owner decision, 2026-11-19) |
| L6 | All timing values are configurable | Do not hard-code grace periods, leases, capacity limits, or channel opening thresholds (section 5) |

---

## 1. Lifecycle by Space Type

| Type | When it starts | When it unloads | Persistence |
|---|---|---|---|
| Personal/community lobby | When the owner or a visitor enters | After it stays empty for the **unload grace period** (L1) | Periodic saves + final save on unload (document 01, section 6.4) |
| MMO field channel 1 | At server startup | Never unloads | None (player state only) |
| MMO field channel 2 and higher | When another channel opens because existing channels are full (section 3) | After it stays empty for the **channel grace period** | None |
| Guild dungeon | When opened (section 4) or when a guild member enters | Unload from memory after it stays empty for the unload grace period. **It stays in the database until its period ends** | Common DB, with fencing |
| Instanced dungeon, PvP | Created on a battle server on entry | Immediately on clear, departure, or match end | Send only results to the home shard |

## 2. Unload Grace Period (L1, L2)

### 2.1 What Counts as Empty

- Players who disconnect while waiting to reconnect (Detached, document 02) **still count as present.** They leave the space only when their session ends completely (reconnect grace period `resume_grace` of 120 seconds, or `suspend_grace` of 600 seconds if they deliberately suspend the app; see the session contract).
- A lobby with visitors is still occupied even if the owner is absent (L2).
- The unload grace period starts when the last player leaves. If anyone enters again during that time, cancel the grace period.

### 2.2 Why 10 Minutes

Connections often drop on mobile networks or during remote play through services such as Steam Link. If the grace period is short, the server must reload the lobby from the database each time a player returns after a brief disconnect. That lengthens the loading screen and adds database load. Ten minutes equals `suspend_grace` (600 seconds), so the lobby stays as it was for players who briefly suspend the app and return.

Memory cost: A dormant InGameSpace does not tick (document 01, section 4.6), so memory is its only cost. Monitor the size of each lobby and the number of lobbies in the grace period with metrics (section 5). Lower the setting if needed.

### 2.3 When the Server Is Under Memory Pressure

If too many InGameSpace instances in the grace period push memory above the configured limit, unload early, **starting with those that have been in the grace period the longest**. The unload procedure stays the same (final save → confirm commit → delete the Directory key).

## 3. MMO Field Channels (L3)

### 3.1 Why Channels Are Needed

One zone thread, and thus one CPU core, handles each field. When players crowd into one place, the work grows with the square of the player count because each player must see the others. **Every field therefore needs a capacity limit.** When it fills, start another copy of the same field (a channel) and split the players between them.

### 3.2 Two Capacity Limits

| Value | Meaning | Default |
|---|---|---|
| `soft_cap` | At or above this value, send newcomers to another channel. The only exception is players following a party member or friend | Per-field setting (set by load test scenario B) |
| `hard_cap` | At or above this value, no one can enter. The host refuses admission | 120% of `soft_cap` |

- Keep each field's capacity limits in design data or configuration. Set the initial values from the load test, using the player count that keeps tick p99 at 50 ms or below.

### 3.3 Automatic Assignment Order

When choosing a channel for an incoming player:

1. A channel with a **party member** (if below `hard_cap`)
2. A channel with a **friend** (if below `soft_cap`)
3. The **most populated channel** below `soft_cap` (to avoid spreading players across several nearly empty channels)
4. If all channels are at or above `soft_cap`, **open a new channel**

- Each channel host periodically writes its player count to Redis: `HSET fieldpop:{shard}:{field} {channel} {player_count}` (one command, document 03, section 9.1). The component admitting the player reads it with `HGETALL` and chooses a channel.
- The count can lag by a few seconds. If several players choose the same channel at once, it can go slightly over. The **host enforces the actual limit using `hard_cap`**. If it refuses admission, choose the next candidate.
- Claim a new channel using the `MmoField(FieldId, Channel)` key (document 01, section 6.1). If two servers try to open "channel 3" at the same time, only one claim succeeds.

### 3.4 Manual Channel Switching

- Players can switch channels from a list showing channel numbers and crowding. The target channel must be below `hard_cap`.
- Use a **cooldown** (configurable, for example 30 seconds) to prevent switches during combat and repeated switches in quick succession.

### 3.5 Closing Channels

- Channel 1 never closes.
- Channels 2 and higher close after staying empty for the **channel grace period** (configurable, 5 minutes by default). Fields have no state to save, so simply unload them.
- Do not force low population channels to merge. As players leave, the channels empty and close on their own.

## 4. Guild Dungeons (L4, L5)

### 4.1 Opening a Dungeon

```
Guild member ──"Open guild dungeon"──> home shard player actor ──> guild actor (document 04, section 5.1)
   Guild actor checks: Is the player the guild leader or authorized to "open guild dungeons"?
                       Is the number of open guild dungeons below the limit (L5)?
                       Can the guild cover the opening cost (guild currency, etc.)?
   On success → Write the guild dungeon row to Common DB (with the expiration time, in the same transaction as the guild currency debit)
              → Send a creation request to the InGameSpaceManager on the Game server with the lowest load (document 03, section 5)
```

- The guild actor checks permission against its **guild ranks and permission list**. Make "open guild dungeons" one permission the guild leader can grant by rank.
- Debit guild currency and write the dungeon row in the same Common DB transaction (one DB per step, document 03, section 3).

### 4.2 Schedule and Expiration

| Type | When open | Examples |
|---|---|---|
| Day-specific dungeon | Only on its assigned day | Monday dungeon, Tuesday dungeon ... |
| Always-open dungeon | Always | |
| Weekend special dungeon | Two days: Saturday and Sunday | |

- Design data determines which dungeons open on which days and what each dungeon contains. When opening a dungeon, the server uses design data to calculate the expiration time and writes it to the database (for example, a day-specific dungeon expires at midnight that day).
- A guild can have at most 5 dungeons open at once, regardless of type.

Follow document 01, section 6.5. The database value is authoritative for expiration and survives server restarts. If the dungeon expires while unloaded, reject the next entry attempt. A batch job handles cleanup.

## 5. Settings

Keep all of these in the server configuration file ("General Settings" in document 08, section 8). They are not secrets, so they can be stored in the repository.

| Setting | Default | Section |
|---|---|---|
| `space.unload_grace` | 600 seconds | 2 |
| `space.memory_limit` | Required (size for server memory) | 2.3 |
| `space.lease_ttl` / `space.lease_renew` | 15 seconds / 5 seconds | Document 01, section 6.2 |
| `field.{id}.soft_cap` | Per field | 3.2 |
| `field.{id}.hard_cap` | `soft_cap` × 1.2 | 3.2 |
| `field.channel_grace` | 300 seconds | 3.5 |
| `field.channel_change_cooldown` | 30 seconds | 3.4 |
| `guild_dungeon.max_open_per_guild` | 5 | 4 |

**Metrics** (add to document 08, section 3): count and memory usage of InGameSpace instances in the grace period, count of early unloads due to memory pressure, channel count per field and player count per channel, and count of `hard_cap` rejections.

## 6. Remaining Decisions

| # | Question | Notes |
|---|---|---|
| L7 | Capacity limits per field | After load testing |
