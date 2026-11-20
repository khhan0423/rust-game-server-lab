# Test Scenarios by Phase

**English** | [한국어](03-test-scenarios.ko.md)

- Status: **Draft** (2026-11-19)
- Parent document: [Execution Plan](README.md). Phase descriptions: [Pilot](01-pilot.md), [Development Phases](02-development-phases.md)

## 1. How to read

A scenario number is `phase-kind-number`. Example: `P4-D-02` is DB test 2 of pilot phase 4.

| Kind | Meaning | How it runs | External dependencies |
|---|---|---|---|
| U | Unit test | `cargo test`. Pure logic | None |
| A | Actor, tick test | `cargo test`. Put commands in through channels and check result messages. Time is virtual time | None |
| D | DB test | `sqlx::test`. Creates an empty DB per test and applies migrations | PostgreSQL |
| I | Integration test | Start the server fleet with the launcher (headless mode) and verify with bot scenarios | PostgreSQL, Valkey |
| F | Failure test | During an integration test, forcibly kill a server or cut a connection via the launcher | Same |
| L | Load test | Many bots + latency distribution collection | Same |
| M | Manual check | A person checks with a checklist in the GUI | Same |

**Common rules**

- Tests do not use `sleep` to hit a timing. Subscribe to the event to wait for first, trigger the action, then wait for that event within a time limit.
- Time-based rules (auth timeout, grace period, lease) are shortened by configuration in integration tests. Unit and actor tests use virtual time.
- Consistency tests end with a **total balance check**. Whether currency and item sums at the start match the end (excluding intended changes) is counted directly in the DB.
- Do not pin sentences or log wording as tests. Check only machine-read values (fields, codes, counts).

## 2. Pilot

### P0. Foundation

| Number | Scenario | Expected result |
|---|---|---|
| P0-U-01 | Load a config file with a required key missing | Fails. The error value contains the missing key name |
| P0-U-02 | Print a secret type with `Debug` | The original value does not appear in the output |
| P0-U-03 | Load a config file with an unknown key | Fails (typos are not silently ignored) |
| P0-A-01 | Admin port: before and after ready | Before ready `/health/live` 200, `/health/ready` 503. After ready `/health/ready` 200 |
| P0-A-02 | Admin port: `/metrics` | Recorded metrics come out as Prometheus format lines |
| P0-D-01 | Check schema of an empty DB, then check after applying migrations | Rejected as "version mismatch" before applying, passes after applying |
| P0-I-01 | Start `server`, `gateway` | One line `ready admin=<address>` on stdout. `/health/live` and `/health/ready` of that address return 200. `/metrics` has `server_ready 1` |
| P0-I-02 | Start `server` with a DB that has no migrations applied | Exits with a non-zero code and no ready notification |
| P0-I-03 | Start with a wrong DB address or without the DB secret environment variable | Exits with a non-zero code and no ready notification |
| P0-I-04 | Run `server migrate` on an empty DB | Ends successfully and passes the schema check |

### P1. Protocol and frames

| Number | Scenario | Expected result |
|---|---|---|
| P1-U-01 | Encode then decode a frame header (including field boundary values) | Same as the original value |
| P1-U-02 | Frame whose `len` exceeds the maximum size | Size limit error. Does not read more of the body |
| P1-U-03 | Frame whose `len` is smaller than the header size (16) | Format error |
| P1-U-04 | Feed a frame one byte at a time | Comes out exactly once, only when the frame is fully assembled |
| P1-U-05 | Three frames in one buffer | Three frames come out in order |
| P1-U-06 | Server-to-server envelope round trip, undefined `kind` | Round trip is identical, unknown `kind` is an error |
| P1-U-07 | Message ID classification boundary values (0x0FFF, 0x1000, 0x7FFF, 0x8000) | System, Content, Content, Zone |
| P1-U-08 | Encode round trip of all message types | Same as the original value |
| P1-U-09 | Check the generated message ID table | No duplicate IDs and all within their own band |
| P1-U-10 | FRAGMENT flag on a client-to-server frame | Classified as a contract violation |

### P2. Server launcher

| Number | Scenario | Expected result |
|---|---|---|
| P2-U-01 | Topology file with duplicate names or ports | Load fails. The duplicated entry is in the error |
| P2-U-02 | Startup order computation | Entry, game server, Gateway order. A dependency cycle is an error |
| P2-I-01 | `up` | The next server starts only after the preceding server becomes `ready` (verified by the order in startup records) |
| P2-I-02 | `up` with a topology where one server dies before `ready` | `up` fails. Cleans up every server it already started |
| P2-I-03 | `down` | Termination signals in reverse order. Servers that do not end in time are forcibly killed. Zero remaining processes |
| P2-I-04 | `kill game-2` | Only game-2 is shown as dead, the rest stay as they were |
| P2-I-05 | `restart game-2` | Comes up with a new PID and returns to `ready` |
| P2-I-06 | Exit codes in headless mode | Success 0, failure non-zero |
| P2-M-01 | GUI status board, log window, buttons | State changes are reflected on screen and Korean text is visible |

### P3. Connection and session

| Number | Scenario | Expected result |
|---|---|---|
| P3-U-01 | Shard assignment: valid friend code | The friend's shard |
| P3-U-02 | Shard assignment: friend's shard is not accepting new players | The open shard with the lowest cumulative population + "friend shard unavailable" marking |
| P3-U-03 | Shard assignment: no friend code, tie | Lowest cumulative population. On a tie, always the same result by a fixed rule |
| P3-U-04 | Token verification: tampered, expired, signed with a different key | All rejected |
| P3-U-05 | Session state transition table | Only allowed transitions pass. A game packet before authentication leads to a termination reason |
| P3-D-01 | First entry | Account, character number, and home shard are recorded in one transaction |
| P3-D-02 | Two first entries of the same account simultaneously | One account. Both receive the same character number |
| P3-D-03 | Publisher: guest account creation | Account and login method are recorded in one transaction. Only a hash of the secret is stored |
| P3-U-06 | Publisher: entry ticket verification (tampered, expired, for a different game) | All rejected |
| P3-I-01 | Normal connection (publisher guest login → ticket → Entry → Gateway auth → game entry) | The bot receives the session key and timer value and connects to the home shard game server |
| P3-I-02 | Logout then login again | The home shard does not change |
| P3-I-03 | Connect only, never send auth | The server disconnects after the auth timeout |
| P3-I-04 | A second connection with the same account | The first session is disconnected with a "replaced" reason, and the second activates after cleanup finishes |
| P3-I-05 | Send a packet over the maximum size | Immediately disconnected without a reason notice |
| P3-I-06 | Keep exceeding the packets-per-second limit | Disconnected. The disconnect reason metric rises |
| P3-I-07 | Send a Content message and a Zone message | Content goes to the home shard. A Zone message before space assignment is dropped and the metric rises |
| P3-I-08 | Start Gateway with Valkey down | `ready` does not turn on |
| P3-I-09 | New entry while the publisher login server is down | New entries rejected. Bots already connected keep working |
| P3-I-10 | Guest re-login (same secret) | Same account, same character, same home shard |
| P3-I-11 | Guest login with a wrong secret | Rejected. No ticket is issued |
| P3-I-12 | Enter again with an already used ticket | Rejected |
| P3-I-13 | Client sends a different account number directly to Entry | Ignored. Only the account verified by the publisher is used |

### P4. Content actors and DB

| Number | Scenario | Expected result |
|---|---|---|
| P4-U-01 | Currency add and subtract boundary values | Overflow or a negative result is an error. The value stays unchanged |
| P4-D-01 | Purchase | Wallet deduction, item addition, transactional log, and idempotency key commit together |
| P4-D-02 | Purchase with a full inventory | Fails. Wallet, inventory, and log all unchanged |
| P4-D-03 | Run a purchase again with the same idempotency key | "Already processed" and the first result come back. The balance was deducted only once |
| P4-D-04 | Insufficient balance | Rejected. No change |
| P4-D-05 | Run two purchases of one player (different keys) simultaneously | Both are applied and the final balance is correct |
| P4-A-01 | Put three commands into an actor in sequence | Processed in the order they were put in |
| P4-A-02 | DB function returns failure | No in-memory state change. A failure response goes out |
| P4-A-03 | DB function times out | Confirms again with the same idempotency key, then settles the result |
| P4-A-04 | Send a command to an actor whose inbox is full | The sender receives failure without waiting. The fullness metric rises |
| P4-A-05 | A command takes longer than the threshold (virtual time) | One slow command warning appears. It contains `player_id`, `request_id`, and the waiting point |
| P4-A-06 | Logout during processing | Finishes the in-flight command, saves, then the actor ends |
| P4-I-01 | Start a game server without the game DB | Startup fails. `ready` does not turn on |
| P4-I-02 | A bot buys an item | Success response. The wallet and inventory in the DB are correct |
| P4-I-03 | Trace the logs of one purchase request | The same `request_id` is in the Gateway, actor, and DB call logs |
| P4-I-04 | Send a purchase request twice with the same request number | The response is the same and it is applied in the DB only once |

### P5. Zone and lobby

| Number | Scenario | Expected result |
|---|---|---|
| P5-U-01 | AOI: entry | The target list of surrounding cells is correct |
| P5-U-02 | AOI: movement across a cell boundary | Only newly visible and newly invisible targets come out |
| P5-U-03 | AOI: grid edges and corners | Does not reference out-of-range cells |
| P5-U-04 | Movement validation | Out-of-bounds coordinates and excess speed are rejected |
| P5-U-05 | Run a tick twice with the same input | The result (list of messages to send) is identical |
| P5-U-06 | Waiting state: same-kind re-request, result with a different request number, timeout | Re-request rejected. Stale result ignored. On timeout, waiting is cleared and a failure response is sent |
| P5-A-01 | More commands than the cap arrive in one tick | Processes only up to the cap and the tick runs. The rest go to the next tick |
| P5-A-02 | A tick takes longer than its period (injected clock) | Does not run backlogged ticks in a burst. The overrun metric rises |
| P5-A-03 | Player count reaches zero | That space does not tick. It ticks again when someone re-enters |
| P5-D-01 | Save with a lower fencing number | Rejected (0 rows changed) |
| P5-D-02 | A save with a lower version number arrives late | Ignored. Latest state kept |
| P5-I-01 | First lobby entry | Claim → load → activate → enter. The owning server is recorded in Valkey |
| P5-I-02 | Two players enter simultaneously during loading | Loaded once. Both enter |
| P5-I-03 | Two managers claim the same space simultaneously | Only one becomes the owner, and the other forwards the request to the owner |
| P5-I-04 | Bot A moves | Bot B within view receives it, Bot C outside view does not |
| P5-I-05 | Everyone leaves and the grace period passes | The owner record is removed only after the final save commits. On re-entry, it comes up with the saved state |
| P5-I-06 | Re-entry during the grace period | The unload is canceled. It does not load again |
| P5-I-07 | Owner leaves, visitors remain | The lobby is kept |
| P5-I-08 | Valkey blocked during lease renewal | New entries stop. Play of bots inside continues |
| P5-I-09 | Equip request | Waiting state → confirmed → combat info version rises and is reflected to surroundings |

### P6. Cross-shard messaging and recovery

| Number | Scenario | Expected result |
|---|---|---|
| P6-D-01 | Effect and outbox record | Same transaction. On rollback both are gone |
| P6-D-02 | Receive a message with the same idempotency key twice | Effect applied once. The second also replies with the same result |
| P6-I-01 | **3 users, 3 shards** (assignment → each buys → user 1 mails user 2) | Assignment to different shards. Each purchase applied in that user's game DB. 1 item in user 2's mailbox. Totals match |
| P6-I-02 | Deliberately deliver a mail message twice on the link | 1 mail |
| P6-I-03 | Mail to an offline user | The actor briefly comes up, processes it, and goes down. The mail is there at next login |
| P6-I-04 | Connect to a server-to-server link with a wrong key | Registration rejected |
| P6-I-05 | Full clean shutdown | 0 unsaved items. Undelivered messages remain in the outbox and are delivered after restart |
| P6-F-01 | Forcibly kill the sending server right after commit, before delivery | Resent from the outbox after restart. 1 mail |
| P6-F-02 | Send while the receiving server is dead | The sender retries with increasing intervals. 1 mail after the receiving server restarts |
| P6-F-03 | Forcibly kill the receiving server right after commit, before the ack | Resent but 1 mail |
| P6-F-04 | Forcibly kill a game server | That server's users receive a disconnect reason. Re-entry possible after restart. Committed currency and items preserved |
| P6-F-05 | Repeat F-01~F-04 above 20 times in random order | Totals match every time. 0 duplicates, 0 losses |

### P6.5. Minimal operations tool

| Number | Scenario | Expected result |
|---|---|---|
| P6.5-D-01 | Operations change (notice registration) and audit record | Same transaction. On rollback both are gone. Generation number increases by 1 |
| P6.5-D-02 | Run a grant twice with the same idempotency key | Granted once. The second also returns the same result |
| P6.5-I-01 | Register a notice from the tool | All game servers report applying the new generation, and the tool screen shows per-server application status |
| P6.5-I-02 | Deliberately do not deliver a notification (hint) | Game servers catch up to the new generation via periodic checks |
| P6.5-I-03 | Make a change while one game server is down | The tool shows that server as "not applied". When the server comes back up, it applies the latest generation |
| P6.5-I-04 | Schedule maintenance | New entries are blocked first, then connected bots receive the notice and are cleaned up |
| P6.5-I-05 | Kick | The target bot is disconnected with a reason, and the tool marks it complete only after it actually disconnected |
| P6.5-I-06 | Try to run a bulk grant without approval | Rejected. It runs only after a second operator approves. The requester cannot approve it themselves |
| P6.5-I-07 | Retry after some recipients fail during a grant | Sent again only to the failed recipients. 0 duplicate grants |
| P6.5-I-08 | An unauthorized operator calls the grant API directly | Rejected and recorded (the server blocks it, separate from hiding the button in the UI) |
| P6.5-I-09 | A request without login, a form submission sent from another origin | All rejected |
| P6.5-F-01 | Forcibly kill a game server during grant execution | Resumed after restart. Exactly once per recipient |
| P6.5-M-01 | Screen check: server status, notices, maintenance, kick, grant, audit record | Each screen works and the audit record keeps before and after values |

### P7. GUI test client

| Number | Scenario | Expected result |
|---|---|---|
| P7-M-01 | Login panel (account, friend code) | Connects and shows the assigned shard |
| P7-M-02 | Lobby screen | Self and other users are visible. The AOI grid overlay can be toggled on and off |
| P7-M-03 | Movement | The on-screen position matches the server-sent position |
| P7-M-04 | Shop, inventory, mail panels | Purchases and incoming mail are reflected on screen |
| P7-M-05 | Packet log window | Sent and received messages are visible and filterable by kind. Round-trip latency is shown |
| P7-M-06 | Disconnect and reconnect buttons | (After P8) Resumes and state is restored |
| P7-M-07 | Korean text | Not garbled |
| P7-I-01 | Headless smoke mode (connect → enter → exit) | Exit code 0 |

### P8. Reconnect

| Number | Scenario | Expected result |
|---|---|---|
| P8-U-01 | Receive the same-sequence Reliable frame twice | The second is not processed. The ack number keeps advancing |
| P8-U-02 | Retransmit buffer exceeds its limit | Buffer is cleared and marked "resync needed" |
| P8-U-03 | Ack received | Acked frames are removed from the buffer |
| P8-I-A | First connection (session contract A) | Per the contract |
| P8-I-B | Briefly in background, connection alive (B) | Not disconnected, continues |
| P8-I-C | Connection dies in background, returns within the grace period (C) | Resume succeeds. Receives missed Reliable responses again |
| P8-I-D | Returns after the grace period (D) | Resume fails (expired). Must log in again. Server-side player cleanup complete |
| P8-I-E | App forcibly killed then relaunched, within the grace period (E) | Resume succeeds with the stored session key |
| P8-I-F | Login from another device (F) | The earlier device ends with "replaced" and does not auto-reconnect |
| P8-I-G | Network switch (G) | Resume on a new connection. Late frames from the old connection are ignored |
| P8-I-H | Disconnected right after a purchase request (H) | After Resume, the purchase response is received once. Applied in the DB once too |
| P8-I-09 | Resume through a different Gateway | Succeeds. Session state carries over |
| P8-I-10 | Volatile messages like movement occur while disconnected | Not queued. On Resume, a single space snapshot reconciles it |
| P8-F-01 | Forcibly kill the Gateway holding a session waiting for Resume | Resume succeeds through another Gateway |

### P9. Load measurement

| Number | Scenario | Measurement and criteria |
|---|---|---|
| P9-L-A | Distributed: 2,000 bots, 3 shards, each in its own lobby, movement 2/s, purchase every 5 s, 30 minutes | Zone tick p99 under 50 ms. Request latency p99 under 150 ms. 0 errors |
| P9-L-B | Dense: one space from 100 players, increasing by 50 | Record the maximum population holding tick p99 50 ms (starting value for capacity) |
| P9-L-C | Concurrent rewards: 2,000 bots claim a reward at the same instant | DB pool wait p99 under 1 s. 0 failures. 0 duplicate grants |
| P9-L-D | Isolation: 5% of bots deliberately trigger a slow DB call (500 ms) | Request latency of the other 95% stays at the level of scenario A |
| P9-L-E | Login surge: 200 logins per second | Login success rate, Entry and Gateway latency. 0 channel-full events |
| P9-L-F | Endurance: scenario A for 4 hours | Memory does not keep growing. Handle and connection counts stable |
| P9-L-G | Config comparison: 2, 4, 8 content workers | Request latency and CPU. Decides the default |
| P9-L-H | Allocator comparison: default allocator and mimalloc | Tick time, request latency, memory. Decides adoption |
| P9-M-01 | Tracing drill: deliberately stall one bot's request | Find the user and the waiting point from logs alone within 5 minutes |

## 3. Development phases

Development phase scenarios list only the essentials. Each phase is broken down to pilot-level detail when it starts.

### D1. Design data

| Number | Scenario | Expected result |
|---|---|---|
| D1-U-01 | Reward table pointing to a nonexistent item | Validation fails. Which table and which row is in the error |
| D1-U-02 | Value range violation (negative price, capacity 0 or less) | Validation fails |
| D1-I-01 | Start the server with a broken table | Startup fails |
| D1-I-02 | Reload during operation | In-flight requests finish on the old tables, new requests use the new tables. Ticks do not stop |
| D1-I-03 | The table to reload is broken | Reload rejected. Service continues on the old tables |

### D2. Combat server and dungeons

| Number | Scenario | Expected result |
|---|---|---|
| D2-U-01 | Combat info calculation | Same gear and buffs always give the same result |
| D2-I-01 | Same-shard 3-player party enters a dungeon | A dungeon is created on the combat server and all three players' Zone messages go to the combat server |
| D2-I-02 | 3-player party across different shards enters a dungeon | Entry and combat work. Content messages keep going to each player's home shard |
| D2-I-03 | Looting | Reserve → grant request → home shard commit → drop removal. 1 item |
| D2-I-04 | Looting with a full inventory | Reservation released, the drop stays on the ground |
| D2-I-05 | Clear reward | The combat server only sends the result, the home shard computes and grants. Once each |
| D2-I-06 | Important reward but inventory full | Granted via mail |
| D2-F-01 | Forcibly kill the combat server during combat | Dungeon progress is lost. Already confirmed rewards remain. Users return to the lobby |
| D2-F-02 | Forcibly kill the home shard game server right after a grant request | Granted once after restart via resend |
| D2-F-03 | Gateway-to-combat-server connection drops during a dungeon entry transition | Treated as a failed transition and the players stay in the original space |

### D3. Fields, channels, NPCs

| Number | Scenario | Expected result |
|---|---|---|
| D3-U-01 | Channel selection: has party members, has friends, neither, all full | Party channel, friend channel, least-filled channel under soft capacity, new channel |
| D3-U-02 | NPC AI state transitions | Idle → chase → attack → return follow their conditions |
| D3-U-03 | NPC in a cell with no players | Not updated. When reactivated, the elapsed time is applied at once |
| D3-I-01 | Entry exceeding soft capacity | A new channel opens and the player is assigned there |
| D3-I-02 | Several players enter at the hard capacity simultaneously | Only up to capacity get in, the rest are reassigned to another channel |
| D3-I-03 | Manual channel move, move again within cooldown | First move succeeds, second is rejected |
| D3-I-04 | Empty channel grace period expires | Channels 2 and above close, channel 1 remains |
| D3-I-05 | Send packets while moving between lobby and field | Packets are forwarded to the new space. The player exists in only one place |
| D3-L-01 | Field density load | Decides per-field capacity |

### D4. Social

| Number | Scenario | Expected result |
|---|---|---|
| D4-D-01 | Two players send friend requests to each other simultaneously | 1 relationship |
| D4-I-01 | First entry with a friend code | Assigned to the friend's shard |
| D4-I-02 | Visiting a friend's lobby on another shard | Enters as a guest on the friend shard's server. Content goes to the home shard |
| D4-I-03 | Visit reward | Once, to the visitor's home shard |
| D4-I-04 | Online status | Friend logins and logouts are reflected |
| D4-I-05 | Whisper (different shard) | Arrives at the other player. If the other player is offline, a failure response |
| D4-F-01 | Forcibly kill the friend shard server during a visit | The visitor returns to their own lobby. No visitor data corruption |

### D5. Guild

| Number | Scenario | Expected result |
|---|---|---|
| D5-U-01 | Permission checks | Only the guild leader and members with the permission pass |
| D5-A-01 | Two players join simultaneously with one slot left | Only one joins |
| D5-D-01 | Guild save with a lower fencing number | Rejected |
| D5-I-01 | Put an item into storage (member on another shard) | Removed from inventory, 1 item in storage. Recorded |
| D5-I-02 | Take from storage: no permission, daily limit exceeded | Rejected. Storage unchanged |
| D5-I-03 | Two officers use guild currency simultaneously | Processed in order. The balance never goes negative |
| D5-I-04 | Open a guild dungeon: the 6th | Rejected (5 concurrent) |
| D5-I-05 | Open the day-of-week dungeon on another day | Rejected |
| D5-I-06 | Join request while no guild member is online | The guild actor briefly comes up and processes it |
| D5-I-07 | Run the weekly settlement batch twice | Reward once |
| D5-F-01 | Forcibly kill a game server or the guild-owning server during a storage deposit | After restart, the item is in exactly one place: the inventory or the storage |
| D5-F-02 | Forcibly kill the guild-owning server | Another server takes over. A late save from the old server is rejected |

### D6. Steam login and payments

| Number | Scenario | Expected result |
|---|---|---|
| D6-U-01 | Order state transition table | Only allowed transitions pass |
| D6-U-02 | Amount calculation (smallest unit per currency) | Integer arithmetic. Overflow is an error |
| D6-I-01 | Steam ticket login (normal, forged, expired) | Only normal passes |
| D6-I-02 | Sandbox purchase end to end | Order creation → approval → confirmation → 1 grant |
| D6-I-03 | Client falsely reports approval | Confirmation fails. No grant |
| D6-I-04 | User closes the approval dialog | Order marked failed. No grant |
| D6-F-01 | Forcibly kill the Commerce service before the confirmation request returns | After restart, continues via a status lookup, 1 grant |
| D6-F-02 | Forcibly killed after confirmation, before the grant | 1 grant after restart |
| D6-I-05 | Refund report received | Currency clawed back. If already spent, negative balance and paid shop restriction |
| D6-I-06 | Process the same refund report twice | Clawback once |
| D6-I-07 | Log in after selling a Steam item outside the game | Removed from the copy and unequipped |

### D7. Trading

| Number | Scenario | Expected result |
|---|---|---|
| D7-U-01 | Trade state transition table | Only allowed transitions pass |
| D7-I-01 | Same-shard trade | Both sides' items are swapped. Totals match |
| D7-I-02 | Cross-shard trade | Same |
| D7-I-03 | One side cancels, timeout | Everything escrowed is returned |
| D7-F-01 | Forcibly kill the trade service or one side's game server at each step (repeated per step) | 0 copies, 0 losses. The trade either completes or is fully returned |

### D8. Cluster and operations

| Number | Scenario | Expected result |
|---|---|---|
| D8-I-01 | Drain and replace a game server | Users move to the new server without disconnection |
| D8-I-02 | Two game servers on one shard | Player actors come up on only one. Both use the same game DB |
| D8-F-01 | Forcibly kill one of two game servers on a shard | The remaining server takes over those users and spaces |
| D8-F-02 | Valkey down | Connected play continues. New entries and new space creation stop. Normalizes after recovery |
| D8-F-03 | Game DB briefly down | Requests get failure responses. Normalizes after recovery through retries. No memory-DB inconsistency |
| D8-I-03 | Alert rules (tick overrun, server gone, DB latency) | An alert fires when the condition is produced |
| D8-I-04 | Restore from backup | Restored per procedure and servers start normally |
| D8-I-05 | Admin commands (notice, kick, maintenance mode) | Rejected without authentication. Ticks do not stop while running |

### D9. Launch preparation

| Number | Scenario | Expected result |
|---|---|---|
| D9-L-01 | 10,000 concurrent users per process (production spec) | Meets the acceptance criteria |
| D9-L-02 | 72 hours continuous | No leaks. Latency maintained |
| D9-F-01 | Fault injection: mixed server, DB, Valkey, network latency and disconnection, repeated | Totals match. Automatic recovery |
| D9-S-01 | Security: forged packets, speed manipulation, request replay, impersonating another user number | All rejected and recorded |
| D9-S-02 | Access the admin port from outside | Unreachable |
