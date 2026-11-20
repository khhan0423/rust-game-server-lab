# Design 02. Sessions and Network IO

**English** | [한국어](02-session-network.ko.md)

- Status: **Draft** (2026-11-17). Payload format finalized (8.2), automatic large-message splitting (8.3), loss recovery and delivery classes (chapter 7), session state transitions (3.1) added
- Prerequisite documents: none (this document must be finalized before 01, the threading model)
- Contract with the client (handshake, reconnect rules, client state, timers, close reasons): **`docs/protocol/session-contract.md`**. Behavior visible on the wire is defined by that document
- Affected documents: `01-threading.md` (see chapter 10)

## 0. Finalized Decisions

| # | Decision | Content |
|---|---|---|
| N1 | Connection structure | **Via gateway.** Clients connect only to the gateway, and the gateway relays to Game servers (shards) and combat servers |
| N2 | Connection and player | **Separated.** If the connection drops, the character stays in the InGameSpace for a grace period, and on reconnect play continues |
| N3 | Transport | **TCP only.** But framing is split into a codec so another transport can be added later |
| N4 | Payload format | **Protobuf + `prost`.** Server and all client code are generated from one schema (8.2) |
| N4a | Large messages | **Automatic splitting.** Server messages exceeding the maximum frame are split by the net layer and reassembled by the client network layer. Game code is unaware (8.3) |
| N5 | Encryption | Player endpoint: **TLS required in production** (`rustls`). Can be disabled by config in development. Needed because `session_key` does not change during a session |
| N6 | Reconnect grace | Start with `resume_grace` **120 seconds**, `suspend_grace` **600 seconds**. The server delivers them at connection time, so they can be adjusted during operation (7.4, session contract chapter 6) |
| N8 | WebView minigames | Play solo and submit only the result. **Bridge approach** (8.4) |
| N9 | Loss recovery | **Sequence + ACK + retransmit buffer.** On reconnect the client reports the last seq it processed, and the Gateway retransmits missing Reliable frames. If not in the buffer, a full snapshot resync (chapter 7) |
| N10 | Delivery class | **Reliable / Volatile.** Only results that must not be lost, such as purchases and rewards, get sequences and retransmission. Movement and combat are overwritten by a snapshot on resume. Default is Reliable (7.1) |
| N11 | Session state | First connection, reconnect, duplicate login (later connection wins, Player is handed over to the new session), disconnect (3.1) |
| N12 | Endpoints | Two kinds: player-facing (Gateway only, public network) and server-to-server (all servers, private network) (1.2, 4.4) |
| N13 | Two destinations | Message categories (System/Content/Zone) are separated by ID range. Content goes to the home shard (fixed), Zone to the current InGameSpace host (moves) (3.2) |
| N14 | Character handling on duplicate login | Handover first (Player stays in memory, only the address changes). If handover is impossible, a full re-login (3.1) |
| N15 | Connection events | The Gateway reports only facts (`LinkEvent`); game-level outcomes are decided by InGameSpace mode policy (4.3) |
| N16 | Session management | Connection (socket) and ClientSession (actor) are separated. Late events are ignored by ConnId. Binding decisions are made by the session actor (reconnect) and by the new Gateway's session handling code (duplicate login, judged by taking the old connection with Redis `SET ... GET`) (chapter 2, chapter 3) |
| N17 | Session mirror | Session information and the Reliable retransmit buffer are mirrored in Redis. **Reconnect to any Gateway** and Replay works. Reconnects are also distributed by the load balancer (7.8, 03 document 9.5) |

---

## 1. Overall Structure

```
                          ┌───────────────┐  server-to-server link  ┌─────────────────────────────┐
 client ──TCP──┐          │  Gateway #1   │═════════════════>│ Game server (shard 1) ── DB 1  │
 client ──TCP──┼──>       │  Gateway #2   │═════════════════>│ Game server (shard 2) ── DB 2  │
 client ──TCP──┘          │  ...          │═════════════════>│ combat server pool (dungeon, PvP) │
                          └───────────────┘                  │ cluster services               │
     ① player endpoint              ② server-to-server endpoint   └─────────────────────────────┘
```

| Server | Role | Endpoint | Game state |
|---|---|---|---|
| **Gateway** | Accept, auth check, framing, packet size and rate limits, heartbeat, reconnect, session management, routing | ① + ② | None |
| **Game server** (shard) | Player's **home shard.** Content (inventory, currency, mail, etc.), lobby, guild dungeon, MMO field, guild actors | ② | Yes + Game DB, Log DB, Common DB (04 document) |
| **Combat server pool** | Instance dungeons, PvP fields. Accepts all players regardless of shard | ② | Yes (ephemeral). No DB |
| **Trade service** | Escrow intermediary for cross-shard item exchange. Own DB (03 document 6.3) | ② | Yes + trade DB |
| **Redis-compatible store** | Account connection state (duplicate login decisions), session mirror, InGameSpace Directory, server registration and discovery, party, chat, queues, player directory cache, rankings (03 document chapters 8, 9) | Own protocol | Transient state, cache, coordination |

Which InGameSpace sits on which server, and where player data lives, is covered by `03-data-ownership.md`.

In C++ server terms, this is the structure that splits a front server (gate, proxy) from zone servers.

### 1.1 Why This Structure Was Chosen

- **Moving to another server's InGameSpace does not drop the connection.** Going to a dungeon (combat server pool), a friend's lobby (another shard), or PvP only changes the Gateway's zone destination (chapter 6). The client does not reconnect.
- **Slow-client problems end at the Gateway.** Per-client send queues and their limits are managed by the Gateway. The backend does not care about individual slow clients.
- **The backend's network structure becomes simple.** The backend handles a few Gateway links, not 10,000 client sockets.
- **The attack surface shrinks.** Only the Gateway is exposed externally. Packet size and rate limits and auth checks are filtered at the Gateway first.

The cost: one more hop adds a little latency (usually under 1ms within the same data center). And one more server type.

### 1.2 Two Kinds of Endpoints

| Item | ① Player endpoint | ② Server-to-server endpoint |
|---|---|---|
| Exposure | **Public network** | **Private network (internal) only.** Not reachable from outside |
| Peers | Tens of thousands of untrusted clients | Tens of trusted servers |
| Auth | `login_token` / `session_key` (session contract document) | Server auth. mTLS recommended; alternative is cluster shared secret + `ServerId` (4.4) |
| Encryption | TLS (required in production) | Optional on a private network. With mTLS, solved together with auth |
| Header | 16 bytes (8.1) | Envelope (4.2), server-to-server messages (4.4) |
| Max frame | 64KB. Splitting on the client side is forbidden | 16MB (snapshots, player data transfer) |
| Rate limit | Yes | None. Monitored via queue limits and metrics |
| Connection count | Thousands to tens of thousands | Tens |
| Loss protection | Per-session seq/ack, retransmit buffer (chapter 7) | Per-message idempotency key + outbox (03 document) |
| On disconnect | Session Detached, grace period | Link reconnect (backoff). Sessions using that link are marked for Resync |

Endpoints by process role:

| Process | ① Player | ② Server-to-server |
|---|---|---|
| Gateway | **Yes** (listen) | Yes (connect to backends) |
| Game server, combat server | No | Yes (listen + connect to other servers) |
| Cluster services | No | Yes |
| All-in-one for development (everything in one process) | Yes | Replaced by in-process channels |

- The same binary runs with a role config. In development, run everything as all-in-one on a single PC.
- **The two endpoints are isolated inside the Gateway.** This is so that a connection flood on the player endpoint does not slow down server-to-server links. Start with a concurrent connection cap and per-IP connection rate limits; depending on load test results, split the two endpoints into separate tokio runtimes.

---

## 2. Distinguishing the Three Kinds of "Session"

In C++, a single `Session` usually represents both the socket and the player. This design separates three things with different lifetimes.

| Concept | Where it lives | Lifetime | Identifier |
|---|---|---|---|
| **Connection** | Gateway | The lifetime of one TCP socket. Ends when it drops | Internal |
| **ClientSession** | Gateway | From login to logout (or reconnect grace expiry). **Survives reconnect** | `SessionId(u64)` |
| **Player** | Inside an InGameSpace on a Game server or combat server | From game entry (`EnterGame`) to logout. One character per account (07 document). Survives for the grace period even if the connection drops | `PlayerId` (UUID v7) |

```
Connection (TCP)  ──────X  drop           ┌──── new Connection (reconnect)
ClientSession     ──────────────────────┴──────────────────────>  kept
Player            ──────────────────────────────────────────────>  stays in the InGameSpace
                        │<── grace period ──>│
```

- The Game server knows nothing about the Connection. All the Game server knows is "Player P is connected via Session S of Gateway G".
- Reconnect is attaching a new Connection to the existing ClientSession. From the Player's side, it is only a state change of "briefly there was no connection".

### 2.1 Identifiers

| Identifier | Meaning | When it changes | Format |
|---|---|---|---|
| **ConnId** | One socket | Newly issued on every connection. **Never reused** | Monotonically increasing inside the Gateway |
| **SessionId** | Owner of the exchange history (seq, retransmit buffer) | **Kept** on reconnect, **newly issued** on fresh login | `[gateway_id: 16 | sequence: 48]`. Never reused |
| **session_key** | Key proving "same session" when reconnecting | Same as SessionId (immutable during the session) | Session contract document chapter 1 |
| **PlayerId** | Character | Always the same for the same character | Cluster-wide (03 document chapter 7) |

- The Gateway ID in a SessionId is the Gateway that **issued the session**. Since a session can move to another Gateway on reconnect (7.8), the Gateway **currently holding the session** is found via Redis `sess:{session}:owner` and `online:{account}` (03 document 9.3).
- Because IDs are never reused, the problem common in C++ session pools, "the ID of a dropped session got reused for a new session, and a late-arriving packet lands in the new session" (ABA), does not occur.

| Event | Proof presented by the new connection | ConnId | SessionId | seq | Player |
|---|---|---|---|---|---|
| Reconnect success (within grace) | `session_key` | New | **Same** | **Continues** | Unchanged |
| Reconnect failure (grace expired, etc.) | `session_key` | New | `ResumeFail` | - | Client falls back to login |
| Duplicate login (app relaunch, another device) | `login_token` | New | **New** | **From 1** | Same Player is **handed over** |
| Login after grace expiry | `login_token` | New | New | From 1 | Newly loaded from DB |

### 2.2 Binding Model

The core of session management is **binding**, and it happens in two stages.

```
Connection ──(stage-1 binding)──> ClientSession ──(stage-2 binding)──> Player
```

| Binding | At one moment | Over time | When the binding changes | Who decides |
|---|---|---|---|---|
| Connection → ClientSession | 1:1 | N:1 | Reconnect (`Resume`) | Same Gateway: that session's **ClientSession actor**; different Gateway: the new Gateway takes over with Redis `SET sess:{session}:owner ... GET` and notifies the old Gateway (03 document 9.5) |
| ClientSession → Player | 1:1 | N:1 | Duplicate login (`Auth`) | **The new Gateway** (judged by taking the old connection with Redis `SET online:{account} ... GET`. 03 document 9.4) |

When a new connection claims "I am this client", handling is decided by the **proof the new connection presents** and the **existing session state**.

| New connection's proof \ Existing session | None | Detached | Active | Closing |
|---|---|---|---|---|
| **session_key** | `ResumeFail` | **Reconnect.** Bind and Replay | **Reconnect (half-open).** Close the old connection as Superseded and bind | `ResumeFail` |
| **login_token** (new connection) | **First connection** | **Duplicate login (quiet takeover).** Clean up old session, hand over Player, Resync | **Duplicate login (race).** `Close(DuplicateLogin)` to the old connection, hand over Player after cleanup completes, Resync | Handled after cleanup completes |
| **login_token** (already-bound connection) | - | - | **Protocol violation.** Drop without `Close` | - |

So that decisions for the same account are handled in a single line, the stage-1 binding is decided by the session actor. The stage-2 binding's order is fixed by a `SET ... GET` on the account key (one command that writes the new value and returns the old value), and the new Gateway handles the result based on the old value it received.

### 2.3 Connection State and Disconnect Reasons

The connection deals with "is this socket alive right now and able to carry data", and the session deals with "how far did the exchange go and who is this now".

| Item | Connection | ClientSession |
|---|---|---|
| TCP socket, send queue (limit 256) | **Owns.** On drop, the send queue is discarded | - |
| Framing, size checks, rate limiting, heartbeat, pre-auth timeout | **Owns** | - |
| seq, ACK, retransmit buffer | - | **Owns.** Survives connection changes |
| Account, destinations (`Routes`), state, grace timer | - | **Owns** |

The send queue belongs to the connection, and the retransmit buffer belongs to the session. Even if the connection dies and the send queue is discarded, Reliable messages are not lost because they are in the session's retransmit buffer.

```rust
enum ConnState {
    /// Right after TCP accept. Accepts only Auth or Resume
    Handshaking { deadline: Instant },
    /// Bound to a session
    Bound { session: SessionId },
    /// Closing. Flushing the last frame (close reason)
    Closing { reason: ConnCloseReason, flush_deadline: Instant },
}
// Closed is not a state. The connection task ending is Closed itself; the socket and send queue are cleaned up by Drop.

enum ConnCloseReason {
    PeerClosed,                    // EOF (read Ok(0))
    PeerReset,                     // ConnectionReset, ConnectionAborted
    WriteFailed,                   // BrokenPipe etc.
    KeepaliveTimeout,              // TimedOut
    HeartbeatTimeout,              // Silence (half-open)
    HandshakeTimeout,
    SendQueueOverflow,
    ProtocolViolation(ViolationKind),
    ClosedByServer(CloseReason),   // Logout processing, kick, maintenance
    Superseded,                    // Pushed out by a new connection via reconnect
    LocalNetwork(std::io::ErrorKind), // NetworkDown etc., server-side problem
}
```

| Disconnect reason | Session handling |
|---|---|
| PeerClosed (with a preceding `Logout`) | Closing (normal logout) |
| PeerClosed (no `Logout`), PeerReset, WriteFailed, Keepalive/HeartbeatTimeout, SendQueueOverflow | **Detached** (grace) |
| ProtocolViolation | Closing (no grace, no reason notification) |
| ClosedByServer | As the session has already decided |
| **Superseded** | **Do nothing** (a new connection is already attached) |
| LocalNetwork | Detached + **operations alert** (many at once means a server-side fault) |

- From socket errors alone, the server cannot reliably tell "did the app die, or was it a brief drop". So all client-side causes are handled as **Detached + grace**, and whether the client returns as the same session is proven by the next connection.
- The disconnect reason is used for **records**, not decisions. Per-session last disconnect reason and per-reason statistics are used to find mobile quality problems (specific devices, carriers).
- Terminations where the server must send a reason (`ClosedByServer`) get time to flush the send queue (up to 3 seconds). The rest are dropped immediately.

### 2.4 The ConnId Rule: Ignore Events from Late Old Connections

Right after reconnect (half-open) attaches a new connection B to the session, the old connection A may belatedly notify "disconnected". If the session trusts this, it goes Detached even though a healthy B is attached. This is the same problem as a reused IOCP socket object in C++ receiving a late completion notification from the previous socket.

```rust
struct ClientSession {
    current_conn: Option<ConnId>,
    // ...
}

// Connection → session messages always carry a ConnId
enum FromConnection {
    Frame { conn: ConnId, frame: Frame },
    Lost { conn: ConnId, reason: ConnCloseReason },
    Logout { conn: ConnId },
}

fn on_lost(&mut self, conn: ConnId, reason: ConnCloseReason) {
    if self.current_conn != Some(conn) {
        return; // Notification from an already-replaced old connection → ignore
    }
    self.current_conn = None;
    self.enter_detached(reason);
}
```

- Frames are the same. Frames arriving late from a replaced old connection are discarded.
- **Direction rule**: the connection reports only facts ("it dropped", "this frame arrived"). Judgment belongs to the session. The connection does not know session state, and the session never touches the socket directly.
- The client follows the same rule (session contract C14).


---

## 3. Gateway Internals

The Gateway has no game state, so there are **no zone threads.** Everything is a task on a tokio multi-thread runtime.

```
[Gateway process: tokio multi-thread]

  Acceptor task (player endpoint)
     │ accept
     ▼
  Connection × N  (one per TCP)                      ← socket. Disappears on drop
     ├─ reader task:  owns ConnState. socket → frame decoding → size/rate checks → to session actor
     └─ writer task:  send queue (bounded) → socket. When the queue closes, flush the rest and shutdown
          ▲ attaches and detaches (distinguished by ConnId)
          │
  ClientSession actor × N  (one task per session)    ← the session body. Survives reconnect
     - state, seq, ACK, retransmit buffer, Routes, grace timer, current ConnId
     - inputs: frames and disconnect notifications from connections, frames from backends, control like Kick, timers
         │ create/register/unregister
         ▼
  SessionRegistry (one actor)                        ← the C++ SessionManager's place
     - SessionId issuance, session_key → session actor handle (finding the session on reconnect)
     - If the session is not on this Gateway, take it over from the Redis session mirror and create a new session actor (03 document 9.5)

  BackendLink × (number of backend servers × links per server)  (server-to-server endpoint)
     ├─ reader task:  parse envelope → find the session actor handle in its own map and deliver
     └─ writer task:  send queue (bounded) → backend socket
```

| Component | What it owns | Lifetime | C++ equivalent |
|---|---|---|---|
| Connection | Socket, read/write buffers, send queue | Ends when the socket drops | Socket object |
| ClientSession actor | All session state | From login to termination. Survives reconnect | Session object (separated from the socket) |
| SessionRegistry actor | ID issuance, session lookup by key | Process lifetime | SessionManager |
| Account directory (Redis-compatible store) | Account → (Gateway, SessionId) | Cluster lifetime | Connected-user list of an account server |

- **Only the ClientSession actor touches session state.** All inputs enter this actor's Job queue in a single line, so there are no locks.
- **Reconnect is attaching a new Connection to the existing session actor.** The Registry finds the session actor by `session_key` and sends `AttachConnection(new ConnId, send handle)`. If the old Connection is alive, the session actor closes it as `Superseded`.
- **The Registry is not involved on every packet.** When a session is created, the Registry registers the session handle with the BackendLink serving that session (3.3), and afterwards packets are found directly in the BackendLink's own map.
- **Termination is done by the session actor itself.** When `Closing` processing finishes, it sends unregistration to the Registry and the BackendLink and the task ends. Owned buffers and handles are cleaned up by `Drop`.
- A session has 3 tasks (1 session actor + 2 connection reader/writer). With 10,000 concurrent users that is 30,000 tasks, which is not a problem scale for tokio. No session pool (preallocated) is created; if measurement shows a need, add one.

### 3.1 ClientSession States and Transitions

#### States

```rust
enum SessionState {
    /// Right after TCP connection. Accepts only Auth or Resume packets
    Unauthenticated,
    /// Auth complete. The stage before game entry (EnterGame)
    Authenticated { account: AccountId },
    /// Connected to backends (home shard and current InGameSpace host), in game
    Active { account: AccountId, routes: Routes },
    /// Connection dropped, waiting for reconnect (grace period)
    Detached { account: AccountId, routes: Routes, deadline: Instant },
    /// Terminating. Waiting for the backend's save and cleanup to complete
    Closing { reason: CloseReason },
}
```

Each state accepts different packets. For example, if a game packet arrives in `Unauthenticated`, the connection is dropped immediately. Because the states are an enum, the path "a game packet passes to the backend from an unauthenticated session" cannot exist in code.

#### Transition diagram

```
                 Auth(login_token)                EnterGame
 [TCP connect] ──> Unauthenticated ─────────────> Authenticated ─────────────> Active
                     │                                                    │   ▲
                     │ Resume(session_key)                  abnormal drop  │   │ Resume success
                     └──────────────────────────────────────────────┐      ▼   │
                                                                    └──> Detached
                                                                           │
     Logout / Kick / grace expiry / replaced by duplicate login            │
 Active, Detached ─────────────────────────────────> Closing ──> (destroyed) <───┘
```

#### The Four Situations

**1) First connection**

1. The client connects over TCP → `Unauthenticated`.
2. It sends `Auth { login_token }`. The Gateway validates the token, and with `SET online:{account} ... GET` (03 document 9.4) records its own session while receiving whether an existing session was present.
3. If there was no existing session, a new ClientSession is created. seq starts from 1 → `Authenticated`. The response `AuthOk { session_key }` is sent.
4. When the character is selected, it is assigned to a backend → `Active`.

**2) Reconnect (continuing the same session)**

1. The client connects over a new TCP and sends `Resume { session_key, last_server_seq }`.
2. If the session is `Detached`, attach the new connection and return to `Active` (7.5).
3. If the session is still `Active`, the client knows about the drop but the server does not yet (half-open). In this case, **close the old connection, replace it with the new connection**, then follow the same procedure.
4. If the session is not on this Gateway, take it over from the Redis session mirror (`sess`, `rbuf`) (7.8, 03 document 9.5). If it is not in Redis either (grace expired, already terminated), send `ResumeFail`, and the client goes to the first-connection procedure.

**3) Duplicate login (a new login for the same account)**

This is when `Auth` arrives for an same account that already has a session (`Active` or `Detached`). Logging in from another device, or a mobile app that was force-killed and relaunched with no resume information, falls here.

**Policy: later connection wins.** Order matters.

1. When the account directory's atomic decision returns the existing session's info, the new Gateway sends `Kick { session, reason: DuplicateLogin }` to the existing session's Gateway (server-to-server link).
2. The old Gateway notifies the old client of the close reason and closes the connection. After **flushing all requests that were in flight from the old session to the backend**, it moves the session to `Closing` and replies "cleanup complete".
3. Only then is the new session created. If the new session attached to the same Player before cleanup completed, the old session's purchase requests and the new session's requests could interleave.
4. The backend's Player is **not logged out but handed over to the new session** (handover, `LinkEvent::Replaced`, 4.3). The Player object, position, and combat state stay as they are; only the `PlayerLink` address changes. With no save and reload, it is fast. Only when handover is impossible (an InGameSpace where handover is forbidden by design, etc.) is the old Player saved and logged out, and then, **after confirming the save commit**, loaded anew (full re-login).
5. Since the new session starts seq from 1, the old session's retransmit buffer cannot be used. The new client receives a **full snapshot** (Resync).

So that two sessions of the same account never coexist, the account → session mapping is decided only in the account directory. The implementation is a `SET ... GET` command on the account key in the Redis-compatible store (03 document 9.4).

**4) Disconnect**

| Kind | Trigger | Transition | Grace |
|---|---|---|---|
| Normal termination | Client's `Logout` | `Active` → `Closing` → destroyed. Backend saves and removes the Player | None |
| Abnormal drop | Socket error, heartbeat timeout, send queue overflow | `Active` → `Detached` | **Yes** (7.4) |
| Grace expiry | No resume within the grace period | `Detached` → `Closing` → destroyed | - |
| Server-forced termination | Duplicate login, sanction, maintenance, protocol violation | Notify close reason, then `Closing` → destroyed | None |

- Close reasons are defined as a `CloseReason` enum and reported to the client as a code. The client shows different messages by reason (e.g., "You are connected from another device", "Under maintenance").
- Protocol violations (size exceeded, game packets before auth, a client `FRAGMENT` frame, etc.) drop the connection immediately without notifying the reason. This is to avoid giving information to attackers.

### 3.2 Routing: Message Categories and Two Destinations

Client messages are divided into three categories.

| Category | Examples | Handled by |
|---|---|---|
| **System** | `Auth`, `Resume`, `Ping`, `Ack`, `Suspend`, `Logout` | **The Gateway itself** |
| **Content** | Purchases, inventory management, mail, quest reward claims, guild requests | The player's **home shard** (fixed for the session). Guild requests are forwarded by the home shard to the guild actor (04 document 5.1) |
| **Zone** | Movement, skills, looting, equip requests, NPC interaction | The **host of the InGameSpace the player is currently in** (moves) |

- Categories are declared in the schema (`.proto` message options), and the code generator assigns message IDs **within each category's ID range**.

| Message ID range | Category |
|---|---|
| `0x0000` ~ `0x0FFF` | System |
| `0x1000` ~ `0x7FFF` | Content |
| `0x8000` ~ `0xFFFF` | Zone |

- The Gateway decides the destination by **only comparing the message ID's range**. It does not parse payloads, so **the Gateway does not need to know game packet formats.** When game packets are added, the Gateway needs no changes.
- The destinations live in the ClientSession's `Routes`.

```rust
struct Routes {
    /// The player's home shard. Does not change during the session
    content: BackendId,
    /// The host of the current InGameSpace. None before assignment
    zone: Option<BackendId>,
}
```

- Moving to another server's InGameSpace changes only `zone` (chapter 6). `content` stays the same, so purchases and mail go to the home shard even while in a dungeon or a friend's lobby.
- If a Zone message arrives while `zone` is `None` (loading), it is dropped with a warning log.
- **Order across categories is not guaranteed.** Content messages and Zone messages go to different servers. Within the same category, order is preserved. Dependencies spanning the two categories (e.g., purchase then equip) are handled by server-side flows (03 document). The client handles this dependency by sending the next request after receiving the response.

### 3.3 Backend Links and Ordering

- Between the Gateway and each backend server there are **multiple** TCP links (config value, default 4). This reduces head-of-line blocking on a single link.
- One ClientSession's packets always go over **the same link** (`session_id % link count`). So within one session, packet order is guaranteed.

### 3.4 Limits and Protections (Handled at the Gateway)

| Item | Default (config value) | On violation |
|---|---|---|
| Max packet size | 64KB | Immediate connection close |
| `FRAGMENT` frames sent by the client | Not allowed (8.3) | Immediate connection close |
| Packets per second (per session) | 60 | Warning, then close if it persists |
| Auth timeout | 10 seconds after connect | Connection close |
| Heartbeat | Client sends every 10 seconds | If nothing received for 30 seconds, treated as a drop (transition to Detached) |
| Client send queue | 256 packets | Judged a slow client, connection closed (transition to Detached) |
| Retransmit buffer (per session, Reliable only) | 256 frames or 64KB | Buffer cleared and marked as needing resync (7.3) |

---

## 4. Backend Structure (Game Servers, Combat Servers)

The backend has no client sockets. It has only links to the Gateway and other servers.

```
[Backend: I/O runtime]
  GatewayLink × (number of Gateways × links)
     ├─ reader task: parse envelope → payload deserialization and validation → by category
     │                 Content → player actor (actor handle from routing table)
     │                 Zone    → zone thread (routing table)
     └─ writer task: send queue (bounded) → Gateway socket
  ServerLink × (number of other servers × links): server-to-server messages (4.4)

[Content runtime (tokio)]  player actors × connected users. Player content state. Only on Game servers (home shard) (01 document chapter 5)
[Zone threads × Kz]      InGameSpace
```

- **Deserialization and validation happen on I/O threads.** Multiple I/O workers process in parallel, and zone threads and player actors receive only typed, validated messages (guide chapter 7, "validate at the boundary").
- Combat servers have no player actors. Content messages always go to the home shard.
- Thread layout details follow `01-threading.md`.

### 4.1 Routing: Which Thread Receives It

- **Content**: the owner of a player's content is the **player actor** (01 document chapter 5). The GatewayLink's routing table holds `SessionId → actor handle`, registered at game entry via `RouteUpdate`.
- **Zone**: the GatewayLink reader must know, per packet, "which zone thread is this session's Player on".
  - **Each GatewayLink reader task exclusively owns the routing table (`HashMap<SessionId, ZoneIndex>`) for the sessions arriving on its own link.** Sessions arriving on one link are served only by that link (3.3), so the table does not need to be shared. No locks.
  - The table is updated by messages. When a Player is assigned to an InGameSpace or moves to another zone thread, the zone thread or the InGameSpaceManager sends a `RouteUpdate { session, zone }` control message to that GatewayLink.
  - Packets arriving mid-move (before the table update) go to the departure zone thread. If the Player has already left, the departure zone thread forwards the packet to the arrival zone thread. This happens only during the brief window of a move.

### 4.2 Envelope Format: Gateway ↔ Backend

```
+----------+----------+---------------+----------+-------------+
| len: u32 | kind: u8 | session: u64  | msg: u16 | payload ... |
+----------+----------+---------------+----------+-------------+
  kind: 0 = client packet relay
        1 = control message (session attached, dropped, resumed, replaced, route switch, etc.)
```

- For a client packet relay (kind 0), `msg` and `payload` carry exactly what the client sent. The Gateway only copies the bytes.
- Control messages (kind 1) are used only between servers, so the Rust-side format can be defined freely. It is independent of the client engine.

### 4.3 Connection Events (LinkEvent)

The backend holds no connection and no session. The Player holds only an **address**.

```rust
struct PlayerLink {
    gateway: GatewayId,
    session: SessionId,
    state: LinkState,   // Linked / Suspended / Unlinked
}
```

The Gateway reports the **facts** that happened to the session as control messages, and the backend converts them into `LinkEvent` and delivers them.

```rust
enum LinkEvent {
    Detached { reason: ConnCloseReason },             // Dropped (grace starts)
    Suspended,                                        // Background forewarning
    Resumed { mode: ResumeMode, gateway: GatewayId }, // Returned as the same session (possibly via a different Gateway)
    Replaced { gateway: GatewayId, session: SessionId }, // A new session took over via duplicate login
    Closed,                                           // Grace expiry or normal termination
}
```

| Receiver | What it does |
|---|---|
| InGameSpace on a zone thread | Decides game-level outcomes with **mode-specific policy** (invulnerability, AI-controlled combat, PvP forfeit timer, etc.). See the InGameSpace structure in `01-threading.md` |
| Player actor on the home shard | On `Closed`, the logout procedure (save then cleanup) |

- **Separation of roles**: the Gateway reports only facts; outcomes are decided by the InGameSpace mode.
- **Game-side timers do not depend on the Gateway's grace period.** The PvP forfeit timer is measured by the PvP mode itself. This is so that changing the Gateway grace period does not change game rules.
- **Ordering guarantee**: control messages flow on the same link as that session's game packets (3.3). The order of "skill use sent right before the drop" and the "dropped event" cannot be swapped.
- When `Replaced` (a new device came in via a different Gateway) or `Resumed` (resumed from a different Gateway) changes the Gateway, the `PlayerLink` address is updated and the routing table refreshed.

### 4.4 Server-to-Server Link Rules

The rules set for client sessions repeat in the same shape for server-to-server links.

| Client session | Server-to-server link |
|---|---|
| `Auth` handshake | `ServerHello { server_id, role, protocol_version }` + server auth |
| Duplicate login | **Duplicate link.** If two servers connect to each other simultaneously, only one survives |
| Reconnect | Link reconnect (exponential backoff) |
| Ignore late events by ConnId | Ignore late events from old links with a **link generation number** |
| Connection state | `Handshaking → Active → Reconnecting` |

- **ServerId**: `{ shard: u16, role: ServerRole, instance: u16 }`. Unique across the cluster.
- **Who initiates**: Gateway → backend is always initiated by the Gateway. Between other servers, the one with the **smaller ServerId** initiates. So duplicate links do not arise. If a race creates both, the link initiated by the smaller one survives.
- **Link count**: multiple per peer server (default 2). Messages pick a link by hashing the key (PlayerId etc.), so messages with the same key keep their order.
- **Reliability**: the link itself has no seq. Server-to-server messages that must not be lost (`GrantItem`, trade deposits and grants, etc.) are guaranteed by **outbox + idempotency key** from `03-data-ownership.md`. When a link drops, unacknowledged messages are resent from the outbox after reconnecting.
- **Server list (discovery)**: which servers are up is obtained from the `server:{id}` list in Redis (03 document 9.7). When a server dies, TTL removes it from the list. Cluster configuration details are covered in document 06.

---

## 5. Connection and Login Flow

```
Client            Gateway                    Redis (online:{account})  Game server
  │──TCP connect────>│ [Unauthenticated]
  │──Auth(token)───>│──validate token──────────────>│
  │                 │<─AccountId, duplicate check────│
  │<─AuthOk(session_key)─│ [Authenticated]
  │──EnterGame───────>│──(home shard)─────────────────────────────────────────>│ InGameSpaceManager: enter lobby
  │                 │<─────────────────── SessionBound(player, routes) ─────│
  │                 │ [Active]
  │<═══════════════ game packets (Gateway relays by header only) ═══════════════>│
```

- The game `login_token` is issued by the game **Entry server**. The Entry server verifies the publisher Login server's entry ticket via the publisher API, and determines the character and home shard in the game Account DB. The home shard is inside the token. Details are in `07-login-account.md`.
- **Duplicate login**: when a new login arrives for the same account, the duplicate login procedure of 3.1 applies (later connection wins).
- The `session_key` is the key proving it is the same session on reconnect. It does not change until the session ends. Issuance, storage, and usage rules follow `docs/protocol/session-contract.md` chapter 1.

---

## 6. Zone Destination Switch (Moving to Another Server's InGameSpace)

The client connection stays, and only the **zone destination (`Routes.zone`) held by the Gateway** changes. The content destination (home shard) does not change. Entering a dungeon (combat server pool), visiting a friend's lobby (another shard), and entering PvP all follow this flow.

```
1. Departure InGameSpace: decides the move (e.g., dungeon entry)
2. Directory: find or place the arrival InGameSpace (03 document, 05 space lifetime document)
3. Home shard content → arrival host: CombatProfile transfer (03 document)
4. Arrival host: accept → control message to Gateway SwitchZone { session, to: arrival host }
5. Gateway: Routes.zone = arrival host
            From now on this session's Zone messages go to the arrival host. Content messages keep going to the home shard
6. Returning: the same way
```

- At the moment of switching, the Gateway marks "switching in progress", and Zone messages arriving during that time are briefly held and sent to the new destination once the switch completes. This prevents packets from leaking to the old server.
- Moving to a different zone thread **within the same server** does not involve the Gateway. Only the routing table inside that server changes (4.1).
- How player data is handled across servers and how results return to the home shard is covered by `03-data-ownership.md`.

---

## 7. Drops, Reconnects, and Packet Loss Recovery (N2, N9, N10)

### 7.0 Where Loss Happens

- **While a TCP connection is alive, there is no loss and no reordering.** TCP guarantees it. So during the connection, no retransmission requests are needed.
- **Loss happens at the moment the connection drops.**
  - Data the server sent but still sitting in the network or kernel buffers disappears with the socket. The server thinks it "sent" it, the client never received it, and the two sides diverge.
  - When a mobile app is **suspended** in the background, the connection stops working. On iOS the issue is not "is it in the background" but "is it suspended". Android's Doze mode also restricts background network access.
  - Switching between Wi-Fi and LTE changes the IP, which breaks the TCP connection itself.
- **Solution: only messages that must not be lost get sequence numbers, ACK, and a retransmit buffer.** Everything else is overwritten with the latest state on reconnect.
  - Precedents using the same approach include ASP.NET Core SignalR's stateful reconnect (.NET 8) and MQTT's persistent session (QoS 1).

### 7.1 Delivery Classes: Reliable and Volatile (N10)

Not every message needs a sequence. The criterion is **"if it is lost, can it be recreated?"**

| Class | Criterion | Examples | seq and retransmission | On drop and resume |
|---|---|---|---|---|
| **Reliable** | **Events and transaction results** that cannot be recreated if lost | Shop purchase results, reward grants, currency changes, mail arrival, trade completion, quest completion | **Yes** | Missing items retransmitted in original order |
| **Volatile** | Things that can be **overwritten** with the latest state | Movement, combat effects, HP changes, appearance/disappearance of nearby objects | None | Discarded. On resume, the latest state comes from the InGameSpace snapshot |

Client → server requests follow the same criterion.
- **Reliable requests**: purchases, reward claims, trade confirmations. Retransmitted, and the Gateway filters duplicates so they are processed **exactly once**.
- **Volatile requests**: movement input, skill use. Input from the moment of the drop is discarded. The player can just input again.

Note that Volatile messages still keep their order, because they are on the same TCP connection. What the class separates is not "order" but **"whether it is worth resending when the connection drops"**.

**The class is declared per message in the schema.** Marked with a `.proto` message option (e.g., `option (delivery) = VOLATILE;`), and the code generator automatically sets the `RELIABLE` flag in the header at send time. Game code does not care about classes.

**The default is Reliable.** Volatile must be explicit. If a marking is forgotten, handling it as Reliable only costs a bit of bandwidth and memory, but handling it as Volatile can lose rewards.

### 7.2 Sequence and ACK

| Item | Rule |
|---|---|
| Scope | **Reliable frames only.** Volatile and control frames (auth, resume, heartbeat, ACK-only) use `seq = 0` |
| Number space | Separate per direction (server→client, client→server). `u32`, starting at 1 when the ClientSession is created. Only Reliable frames are counted, so numbers are contiguous |
| Who numbers server→client seq | **The Gateway.** The Gateway numbers frames whose `RELIABLE` flag the backend set. Whether sent by the Game server or the Gateway, it is one number space, so a zone destination switch (chapter 6) does not break the numbering |
| Who numbers client→server seq | The client network layer |
| Fragment frames (8.3) | When a Reliable message is split, each fragment gets its own seq |
| ACK | The `ack` field in every frame header = "the last seq of the peer's Reliable frames that I have **finished processing**" (cumulative ACK). Also carried on Volatile frames. If there is no frame to send, an ACK-only frame is sent every second |
| Processed criterion (client) | ACK after reflecting it in game logic. If the app process dies after receiving but before processing, all memory is gone anyway, so it goes through the duplicate login path (3.1) |
| seq skips mid-connection | Cannot happen on TCP, so treated as a bug or tampering. Drop the connection and follow the resume procedure (7.5) |

### 7.3 Retransmit Buffer

| Location | Contents | Cleared when |
|---|---|---|
| **Gateway** (per ClientSession) | **Reliable** frames sent to the client but not yet ACKed (with their original seq) | On the client's ACK, remove from the front up to that seq |
| **Client** | **Reliable** request frames sent to the server but not yet ACKed | Removed on the server's ACK |

- Limit (config value): per session, **256 frames or 64KB**, whichever comes first. Since Volatile is excluded, what enters the buffer is only rare messages like purchases and rewards, so it can be small. Worst case memory is 10,000 concurrent users × 64KB ≈ 640MB, and in practice far less.
- **The Gateway's retransmit buffer is mirrored to Redis (`rbuf:{session}`).** Reliable frames are written to Redis before being sent to the client, and removed from Redis on ACK. Used when resuming from a different Gateway (7.8).
- **While Detached**, the Gateway buffers only **Reliable** frames from those the backend sends, and discards Volatile ones.
- **If the limit is exceeded**, the buffer is cleared and the session is marked "resync needed". On resume it goes through the full snapshot path (Resync in 7.5).
- If the link between the Gateway and a backend drops, frames that were flowing on that link may be lost. So all sessions belonging to that link are marked "resync needed".

### 7.4 Drop

1. The Gateway detects the drop (socket error, heartbeat timeout, send queue overflow).
2. It moves the ClientSession to `Detached` and records the grace deadline (`resume_grace` default 120 seconds; if a `Suspend` was received just before, `suspend_grace` default 600 seconds. Config values). seq and the retransmit buffer are kept.
3. It sends the control message `SessionDetached { session }` to the backend.
4. The backend's Player **stays** in the InGameSpace. The backend sends packets as usual, and the Gateway buffers only Reliable ones (7.3). The backend can skip Volatile sends for Detached sessions to save link bandwidth. In-combat handling (invulnerability, auto combat, staying exposed) depends on game design.

### 7.5 Resume Handshake

```
Client  ── Resume { token, last_server_seq }  ──────────────>  Gateway
            (last_server_seq: the last seq of server Reliable frames the client finished processing)

Gateway: validate token → attach the new Connection to the existing ClientSession (reconnect in 3.1)

  [Buffer has everything from last_server_seq + 1]  → Replay mode
Gateway ── ResumeOk { mode: Replay, last_client_seq } ─────>  Client
Gateway ── retransmit the buffered Reliable frames with their original seqs ──>  Client
Gateway ── SessionResumed { resync: false } ───────────────>  Backend
Backend ── InGameSpace snapshot (Volatile state: position, HP, nearby objects) ──>  Client

  [Not in the buffer (limit exceeded, link failure, not in the Redis mirror either)]  → Resync mode
Gateway ── ResumeOk { mode: Resync, last_client_seq } ─────>  Client
Gateway ── SessionResumed { resync: true } ────────────────>  Backend
Backend ── full snapshot (InGameSpace state + player state: inventory, currency, mail, etc.) ──>  Client

Client  ── retransmit unacknowledged Reliable requests after last_client_seq with their original seqs ──>  Gateway
Gateway: discard seqs at or below the last received seq (prevents duplicate processing)
```

- **In both modes, the InGameSpace snapshot is always sent.** Volatile messages during the drop were discarded. The difference is whether missing Reliable results are filled by retransmission (Replay) or the entire player state is sent again (Resync).
- In Replay mode, **Reliable retransmission comes first, the InGameSpace snapshot later**. This is so the client first applies results like reward grants, then brings the screen state up to date.
- **Duplicate handling of client requests**: the Gateway remembers "the last client seq received" and does not forward seqs at or below it to the backend. So even if a purchase request sent right before the drop is retransmitted, it is processed **exactly once within the same session**.
- **Duplicates across sessions cannot be blocked by this layer.** That is the case when the session changes due to duplicate login (3.1) or re-login after grace expiry. So requests like purchases, trades, and reward claims carry a separate application-level **request ID (idempotency key)**, and duplicates are checked in the DB. This part is covered in the DB design and `03-data-ownership.md`.
- The client also discards server frames at or below seqs it has already processed, among retransmitted frames. Normally this cannot happen, but it is kept as a safety net.

### 7.6 When the Client Starts Reconnecting

| Trigger | Action |
|---|---|
| Socket error, connection close detected | Try resume immediately |
| No heartbeat response (15 seconds) | Consider the connection dead and try resume |
| **App returns to foreground** (Unity `OnApplicationPause(false)`, Unreal `FCoreDelegates::ApplicationHasEnteredForegroundDelegate`, etc.) | Do not trust the connection state; confirm immediately with a ping. If no response, try resume right away |
| Network change (Wi-Fi ↔ LTE) | Consider the existing connection dead and try resume right away |

- Retry intervals use exponential backoff with jitter (0.5s, 1s, 2s, ... up to 10s).
- If the grace period (`resume_grace` 120 seconds, or `suspend_grace` 600 seconds if it was in the background) expires, or `ResumeFail` is received, give up on resuming and go to the first-connection procedure. If the existing Player remains, it is taken over via the duplicate login path (3.1).

### 7.7 Grace Expiry

- If there is no resume within the grace period, the Gateway cleans up the ClientSession and retransmit buffer and sends `SessionClosed { session }`.
- The backend follows the normal logout procedure (save, then remove from the InGameSpace).

### 7.8 You Can Reconnect to Any Gateway

- Session information and the Reliable retransmit buffer are **mirrored** to Redis (03 document 9.5). Reliable frames are written to Redis **before** being sent to the client, so every frame the client could have seen is in Redis.
- So the client may reconnect to **any Gateway** (whichever the load balancer picks). The new Gateway takes over the session with `SET sess:{session}:owner ... GET`, learns the old Gateway, sends a **Superseded** message to the old Gateway (server-to-server link), and Replays from `rbuf` (03 document 9.5).
- Replay also works when the original Gateway died or scaled in. The backend sees the Gateway link drop, keeps all its Players in grace, and when one resumes from another Gateway, updates the address with `LinkEvent::Resumed { gateway }`.
- If the client comes back to the original Gateway, it is handled directly from the in-memory buffer. This path works even if Redis is down (03 document 9.10).

---

## 8. Packet Format

### 8.1 Framing (Finalized): Client ↔ Gateway

```
+----------+----------+------------+----------+----------+-------------+
| len: u32 | msg: u16 | flags: u16 | seq: u32 | ack: u32 | payload ... |
+----------+----------+------------+----------+----------+-------------+
  len   : total length including the header (16 bytes). Exceeding the maximum closes the connection immediately
  msg   : message ID. The Gateway routes by this value alone
  flags : bit0 = FRAGMENT (8.3), bit1 = RELIABLE (7.1). Remaining bits reserved (compression, encryption markers, etc.)
  seq   : sequence number of Reliable frames (7.2). 0 if not Reliable
  ack   : the last seq of the peer's Reliable frames that has been processed (7.2)
  byte order: little-endian, fixed
```

The header is simple enough that Unity (C#), Godot (GDScript/C#), or Unreal (C++) can each implement it in a few dozen lines.

### 8.2 Payload Format (Finalized: Protobuf + `prost`)

- Research basis: `docs/research/packet-format.md`
- The schema source lives in a single `proto/` directory. The server (Rust, `prost`) and the clients (Unity C#, Godot C#, Unreal C++) all generate code from it.
- The mapping table between message IDs (`msg u16`) and message types is also generated from the schema. Not maintained by hand. The generation method (message options or a separate mapping file) will be decided at implementation.
- Godot does not do web export, so it uses C# (`Google.Protobuf`). No GDScript code generation is needed.
- Schema authoring rules follow chapter 5 of the research document (no field number reuse, `reserved`, coordinate quantization, do not pull generated types into the `game` crate).
- This decision is contained inside the `protocol` crate. Changing the format later does not change the Gateway, the envelope, or game logic.

### 8.3 Automatic Large-Message Splitting (Fragmentation)

Example: `ReqPlayerItem` → `PlayerItemAck` with thousands of items, whose serialized size exceeds the maximum frame size (64KB).

**Principle: game code does not know about splitting.** Server game code sends one `PlayerItemAck`, and client game code receives one `PlayerItemAck`. Splitting and joining happen only in the server's `net` layer and the client's network layer.

#### Fragment frame format

```
+----------+----------+--------------+------------------+------------+------------+---------+
| len: u32 | msg: u16 | flags: u16   | transfer_id: u32 | index: u16 | count: u16 | chunk.. |
|          |          | (FRAGMENT=1) |                  |            |            |         |
+----------+----------+--------------+------------------+------------+------------+---------+
  msg         : the original message ID. All fragments share the same value
  transfer_id : a monotonically increasing transfer number within the session
  index/count : fragment ordinal (from 0) and total fragment count. index == count - 1 means the last fragment
```

`seq` and `ack` are omitted in the diagram. In reality the fragment header follows the common 16-byte header of 8.1. If the message is Reliable, each fragment gets its own seq.

#### Sending (server)

1. The `protocol` encoder serializes the whole message.
2. If the size is within the maximum payload, send it as one normal frame.
3. If it exceeds it, cut it into fragment sizes and create `count` fragment frames.
4. Enqueue all fragments as **one send unit**. So no other game message slips in between fragments.

Splitting is done by the backend that created the message (Game server or combat server). The Gateway relays fragments like normal frames, looking only at the header, so **the Gateway needs no changes.**

#### Receiving (client)

1. On receiving a `FRAGMENT` frame, store it at position `index` in the buffer for that `transfer_id`.
2. When all `count` fragments arrive, concatenate, deserialize **once**, and deliver one `PlayerItemAck` to game code.

We do not deserialize each fragment separately. Fragment boundaries are arbitrary byte cuts, so a single fragment is not a Protobuf message by itself. Cases that must be interpreted per piece should be designed with the paging below.

#### Order guarantee

- Game message order is preserved. No other game message slips between `PlayerItemAck`'s fragments.
- Gateway's own messages (heartbeat replies, etc.) can slip in between fragments. So the client does not discard the reassembly buffer when it receives a non-`FRAGMENT` frame.

#### Limits

| Item | Default (config value) | On violation |
|---|---|---|
| Max total size of a server → client message | 4MB | Error log + refusal to send on the server. Treated as a design bug |
| Client reassembly timeout | 30 seconds | Buffer discarded, then re-request or reconnect |
| Client → server splitting | **Not allowed** | If the Gateway receives a client frame with the `FRAGMENT` flag, it closes the connection immediately. This blocks attacks that exhaust server memory. Design client requests to be small |

#### Division of Roles Between Automatic Splitting and Paging

| | Automatic splitting (this section) | Paging (application design) |
|---|---|---|
| Shape | One message cut at byte boundaries | Several independent messages like `PlayerItemPage { page, total, items }` |
| Game code | Unaware of splitting | Knows about pages and handles them |
| Client handling | All at once after everything arrives | Can display as pages arrive, completion on the last page |
| Use | **Safety net.** Delivers messages that grew beyond expectations safely | **The design choice for inherently large data.** Low peak memory and progress display possible |

Guideline: responses whose normal size can exceed a few hundred KB (full inventory, mailbox, etc.) are designed as paged messages from the start. Automatic splitting stays always on as a safety net for all other messages.

While a large message is being sent, movement and combat packets on the same connection wait behind it (head-of-line). So the client guide says to make bulk data requests at non-combat moments like loading screens or menus.

#### Engine Implementation Verification

The reassembly logic is implemented once per engine's network layer (C#, C++). The server generates and provides **byte samples (test vectors)**, and each engine implementation is verified to produce the same result with them.

### 8.4 In-Client WebView Minigames

A minigame can run in a WebView (JavaScript) inside the client.

**Finalized: bridge approach.** The minigame is played solo and only the result (score, reward request) is submitted to the server. The WebView does not connect to the server directly. Messages pass through the host client's JS bridge, and the host relays over the existing TCP connection.

- Auth and session are reused as-is. No separate login for the WebView side.
- No new transport like WebSocket is needed on the server. N3 (TCP only) is preserved.
- The host already handles reconnect, splitting, and other network processing, so the minigame deals only with messages.
- Minigame messages are also defined in the same `proto/` schema. The JS side uses Protobuf JS code generation (e.g., protobuf-es), or the host converts between JSON and Protobuf.

If a real-time competitive minigame appears, a dedicated backend (HTTP/WebSocket) will be designed separately at that time. In that case it does not go through the Gateway.

Minigame results are values computed by the client and are **not trusted as-is.** Reward grants are handled by the home shard as Content messages, after server-side checks such as score caps and play time.

---

## 9. Crate Layout Changes

```
crates/
├─ protocol/   # client packets (header + payload codec), envelope, control messages
├─ net/        # (new) framing codec, link send/receive tasks, network code shared by Gateway and backends
├─ game/
├─ db/
├─ gateway/    # (new) Gateway binary
├─ server/     # server binary. Runs Game server, combat server, cluster services, or all-in-one by role config
└─ bot/
```

Added dependency directions: `gateway → net → protocol`, `server → net`. `game` still knows nothing about networking.

---

## 10. Impact on the 01 Threading Model

The decisions in this document (GatewayLink, content/zone routing, `RouteUpdate`, `LinkEvent`, moving slow-client handling to the Gateway, termination order) are reflected in `01-threading.md` v2.

---

## 11. Remaining Decisions

| # | Question | Note |
|---|---|---|
| N7 | Gateway placement (shared across shards, or per-shard) | Effectively decided as **shared across shards** due to cross-shard play |
