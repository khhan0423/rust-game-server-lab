# Server-Client Session Contract

**English** | [한국어](session-contract.ko.md)

- Status: **Draft v0.1** (2026-11-17)
- Audience: Server (Gateway) developers and client (Unity, Unreal, Godot) network layer developers
- Related design: `docs/design/02-session-network.md` (internal server structure)

This document defines a contract that **both the server and the client MUST follow**. Document 02 covers the internal server structure. This document defines only behavior visible on the wire. If either side behaves differently from what this document specifies, that's a bug.

Notation: Required (**MUST**), prohibited (**MUST NOT**), recommended (**SHOULD**). Code comments and test names refer to rule numbers such as C1 and S1.

---

## 1. Identifiers and Keys

| Name | Created by | Visible to the client? | Lifetime | Purpose |
|---|---|---|---|---|
| **ConnId** | Gateway | **No** (internal to the server) | One TCP connection | Filters out late events from an old connection |
| **session_key** | Gateway | **Yes** | From issuance in `AuthOk` until the session ends | Proves that the client is reconnecting to the same session |
| **PlayerId** | Server (DB) | Yes | Lifetime of the character | Identifies the character |
| **seq / ack** | Both sides | Yes | One session (starts at 1 when the session changes) | Recovers lost Reliable messages (document 02, section 7) |

### 1.1 session_key Rules

- **S1.** The server issues `session_key` **once** when it creates a session and **MUST NOT change it** until the session ends. A successful reconnect uses the same key.
- **S2.** `session_key` contains a SessionId and at least 128 bits of cryptographic randomness. The client MUST NOT be able to guess and construct it.
- **C1.** The client receives `session_key` **only in `AuthOk`**. It treats the value as opaque bytes and doesn't interpret it.
- **C2.** The client keeps `session_key` **only in memory**. It MUST NOT save the key to disk. If the app process restarts, the key is lost and the client follows the initial connection (`Auth`) flow.
  - Reason: When the app restarts, the client's game state is also lost. Resending just the missed messages (Replay) doesn't help. The server handles this as a duplicate connection, carries over the same character, and sends the full state again.

**Why the key doesn't change on each reconnect.** If the server issues a new key on each reconnect, the connection can drop again right after the server sends `ResumeOk` with the new key. The client then has only the old key, which the server has already discarded, so reconnecting will fail indefinitely. The risk is higher on mobile, where consecutive disconnects are common. Production uses TLS to prevent eavesdropping on the key (document 02, N5).

---

## 2. Handshake Messages

All handshake messages are non-Reliable (`seq = 0`) because each connection starts a new handshake.

### 2.1 Initial Connection

| Direction | Message | Fields |
|---|---|---|
| C → S | `Auth` | `login_token` (issued by the game Entry server, document 07, section 3.2), `protocol_version`, `client_version`, `platform` |
| S → C | `AuthOk` | `session_key`, `player_id`, `server_time`, `timers` (section 6) |
| S → C | `Close` | `reason`, `reconnect_allowed` (section 7). Authentication failure, version mismatch, and so on |

### 2.2 Reconnect (Relogin)

| Direction | Message | Fields |
|---|---|---|
| C → S | `Resume` | `session_key`, `last_server_seq`, `protocol_version`, `client_version` |
| S → C | `ResumeOk` | `mode` (`Replay` / `Resync`), `last_client_seq`, `server_time`, `timers` |
| S → C | `ResumeFail` | `reason` (`Expired`, `Unknown`, `Closed`, `VersionMismatch`) |

- `last_server_seq`: The seq of the last Reliable server message the client **finished processing**.
- `last_client_seq`: The seq of the last Reliable client request the server **received**.
- `mode` serves the same purpose as the MQTT 5 CONNACK `Session Present` flag. The server tells the client whether it can keep relying on its local state (`Replay`) or must discard it and receive it again (`Resync`).

### 2.3 While Connected

| Direction | Message | Purpose |
|---|---|---|
| C → S | `Ping { client_time }` | Heartbeat. Sent every `timers.heartbeat_interval` |
| S → C | `Pong { client_time, server_time }` | Heartbeat response. Measures latency |
| Both directions | `Ack { ack }` | Sends a standalone ACK when there's no message to send (document 02, section 7.2) |
| C → S | `Suspend` | **Advance notice** that the app is entering the background (section 4) |
| C → S | `Logout` | Normal logout |
| S → C | `Close { reason, reconnect_allowed, message }` | Reason the server sends just before closing the connection |

---

## 3. Client States

The client network layer has the states below. The states and transitions MUST be the same across engines (Unity, Unreal, Godot).

```
                       Auth                AuthOk
  Offline ──connect──> Authenticating ──────────────> Online <─────────────────┐
     ▲                     │  ▲                         │  │                   │
     │                     │  │ ResumeFail / grace      │  │ Enter background  │ ResumeOk
     │        Close(no     │  │ period elapsed          │  ▼                   │
     │        reconnect)   │  └──────────── Reconnecting <─ Suspended          │
     │                     ▼                  │  ▲          │ Enter foreground │
     └──────────────── Terminated             │  └──drop────┘ (if needed)      │
                      (no auto reconnect)     └──────── Resume ───────────────┘
```

| State | Meaning | Data held |
|---|---|---|
| `Offline` | No connection or session | - |
| `Authenticating` | Sent `Auth` or `Resume` and waiting for a response | `session_key` (for Resume) |
| `Online` | Operating normally. Sends and receives game messages | `session_key`, seq/ack, unacknowledged Reliable requests |
| `Suspended` | The app is in the background. Heartbeats are stopped | Same as above, plus the time the app entered the background |
| `Reconnecting` | The connection dropped. Retrying `Resume` with backoff | Same as above, plus the time the connection dropped |
| `Terminated` | The server closed the connection without allowing reconnect. Shows the reason to the user | Reason for closure |

---

## 4. Client Rules

### 4.1 Choosing How to Reconnect

- **C3.** When the connection drops, check these conditions in order.
  1. If `reconnect_allowed` in the last `Close` received is `false` → `Terminated`. **MUST NOT reconnect automatically.**
  2. If `session_key` exists and no more than `timers.resume_grace` has passed since the connection dropped → `Resume`.
  3. Otherwise → `Auth` (initial connection).
- **C4.** On receiving `ResumeFail`, discard `session_key` and proceed with `Auth`.
- **C15.** The client **may send `Resume` to any Gateway.** It doesn't need to return to the original Gateway. Choose the connection address the same way as at login (a load balancer or the address provided by the game Entry server).
- **C5.** Retry intervals are 0.5 seconds, 1 second, 2 seconds, 4 seconds, 8 seconds, and 10 seconds thereafter. Add ±20% jitter to each interval. This keeps thousands of devices from connecting at the same instant.
- **C6.** Make **only one connection attempt at a time**. If two attempts send `Resume` with the same `session_key`, they can displace each other (Superseded).
- **C14.** Before attempting a new connection, **close and discard the old socket first**. **Ignore all** late messages from the discarded socket, including `Close`. The client network layer tracks the current connection number and checks it on every event.
  - Reason: If the same device sends `Auth` on a new connection before cleaning up its old one, the server treats it as a duplicate login and sends `Close(DuplicateLogin, reconnect_allowed=false)` to the old connection. If the client accepts that message, it **displaces itself** and enters `Terminated`. This uses the same mechanism as the server's ConnId rule (document 02, section 2.4).

### 4.2 Background and Foreground

- **C7. On entering the background**
  - **Attempt to send** `Suspend` on a best-effort basis. Don't wait for a response. On iOS, send it within the short background execution window (on the order of tens of seconds even with `beginBackgroundTask`).
  - Stop heartbeats and enter `Suspended`.
  - **MUST NOT close the connection directly.** It may still be alive after a short time in the background.
- **C8. On returning to the foreground**
  - If the app spent longer than `timers.heartbeat_timeout` in the background → assume the connection is dead, discard it **immediately**, and send `Resume`.
  - If the app spent less time in the background → send `Ping`. If `Pong` arrives within `timers.foreground_probe` (3 seconds), return to `Online`. If it doesn't, discard the connection and send `Resume`.
  - In both cases, **check the connection before trusting it**. A connection may look alive after the app returns from the background even if the server has already closed it (half-open).
- **C9. Events by engine**

| Engine | Entering the background | Returning to the foreground | Notes |
|---|---|---|---|
| Unity | `OnApplicationPause(true)` | `OnApplicationPause(false)` | Don't use `OnApplicationFocus`. On Android, opening the onscreen keyboard alone triggers it |
| Unreal | `FCoreDelegates::ApplicationWillEnterBackgroundDelegate` | `FCoreDelegates::ApplicationHasEnteredForegroundDelegate` | The app can terminate without warning in the background. Finish the required work on entering the background |
| Godot (C#) | `NOTIFICATION_APPLICATION_PAUSED` | `NOTIFICATION_APPLICATION_RESUMED` | Only fires on mobile platforms |

### 4.3 After Reconnecting

- **C10.** If `ResumeOk.mode == Replay`, process the Reliable messages that follow (the resent messages), then apply the InGameSpace snapshot. Keep local state.
- **C11.** If `ResumeOk.mode == Resync`, **discard** local game state and rebuild it from the full snapshot that follows.
- **C12.** Resend unacknowledged Reliable requests with a seq greater than `last_client_seq` **using their original seq**. Don't assign new seq values. The server needs the same seq to filter out duplicates.
- **C13.** Discard Reliable server messages with a seq less than or equal to the last one already processed.

---

## 5. Server Rules

- **S3. Resume decision.** The server responds only as specified in the table below.

| Session identified by `session_key` | Response |
|---|---|
| None (unknown key or session already closed) | `ResumeFail(Unknown)` or `ResumeFail(Closed)` |
| Grace period expired | `ResumeFail(Expired)` |
| `Detached` (no connection) | Attach the new connection, reply with `ResumeOk` |
| `Active` (old connection still attached, half-open) | Close the old connection (Superseded), attach the new connection, reply with `ResumeOk` |
| Version mismatch | `ResumeFail(VersionMismatch)` followed by `Close(UpdateRequired)` |

- **S4. Mode selection.** Use `Replay` if the replay buffer has every message from `last_server_seq + 1` onward. Otherwise use `Resync`.
- **S5. Suspend handling.** On receiving `Suspend`, mark the session as suspended.
  - Stop sending Volatile messages. Continue buffering Reliable messages.
  - If the connection later drops, use the longer `suspend_grace` instead of `resume_grace`. The user deliberately put the app in the background and is likely to stay away longer.
  - If the client sends `Ping` while the connection is still alive, clear the flag and resume normal transmission.
  - This follows the MQTT 5 idea of letting the client change the session expiry time when it disconnects.
- **S6. Send the reason before disconnecting.** Except for protocol violations, when the server closes a connection, first send `Close` and allow up to 3 seconds to drain the send queue before disconnecting.
- **S7. The server sets timer values.** Send them in `timers` in `AuthOk` and `ResumeOk`. The client MUST NOT hardcode these values. This allows changes in production without a client update.
- **S8. Version check.** If `protocol_version` is outside the server's supported range, send `Close(UpdateRequired, reconnect_allowed = false)`.
- **S9. `Auth` on an attached connection is a violation.** `Auth` and `Resume` are valid only while the connection is in the handshake state. If `Auth` or `Resume` arrives on a connection already attached to a session, treat it as a protocol violation and disconnect without `Close`. To switch accounts, send `Logout` and start a new connection.
- **S10. Duplicate login always creates a new session.** If `Auth` arrives on a **new connection** for an account that's already online (the existing session is `Active` or `Detached`), it's a duplicate login.
  - If the existing session is `Active`, send `Close(DuplicateLogin, reconnect_allowed=false)` to the old connection and disconnect it. If it's `Detached`, clean up quietly because there's no connection to notify.
  - The new session receives a **new `session_key`** and seq **starts at 1**. Don't use the old session's replay buffer.
  - For the client, this is **the same flow as the initial login**. It receives `AuthOk` and builds its state from the full snapshot. The server hands over the character internally (document 02, section 3.1), so play continues from the same location.

---

## 6. Timers (`timers`, sent by the server)

| Name | Default | Meaning |
|---|---|---|
| `heartbeat_interval` | 10 seconds | Client `Ping` interval |
| `heartbeat_timeout` | 30 seconds | Treat the connection as dead if nothing arrives during this time (both sides) |
| `foreground_probe` | 3 seconds | Time to wait for `Pong` after returning to the foreground (C8) |
| `resume_grace` | 120 seconds | Time to keep the session after the connection drops |
| `suspend_grace` | 600 seconds | Time to keep the session if the connection drops after `Suspend` (S5) |
| `ack_interval` | 1 second | `Ack` interval when there's no message to send |
| `handshake_timeout` | 10 seconds | Time after connecting in which to send `Auth`/`Resume` |

---

## 7. Reasons for Closure (`Close.reason`)

| Reason | `reconnect_allowed` | Client behavior |
|---|---|---|
| `Logout` | false | Show the login screen |
| `DuplicateLogin` | **false** | "You signed in on another device." **If the devices reconnect automatically, they'll keep displacing each other** |
| `Maintenance` | false | Show a maintenance notice |
| `UpdateRequired` | false | Direct the user to the store |
| `Banned` | false | Show a sanction notice |
| `ServerShutdown` | true | Back off, then send `Resume` (another process can take over) |
| `IdleTimeout` | false | Show the login screen |
| (protocol violation) | No `Close` | Same as a normal disconnect. This doesn't happen with a client that follows the protocol |

---

## 8. Scenarios

### A. Initial Connection
```
C: connect → Auth(login_token, ver)
S: AuthOk(session_key=K, timers)          C: Online, keep K in memory
```

### B. Briefly in the Background (Connection Alive)
```
C: [enter background] Suspend → Suspended
S: mark as suspended, stop Volatile messages
C: [return to foreground after 5 seconds] Ping
S: Pong, clear the suspended flag            C: Online (no reconnect)
```

### C. Longer in the Background (Connection Dead, Within Grace Period)
```
C: [enter background] Suspend → Suspended
   ... OS suspends the app, connection drops. Server detects it by heartbeat → Detached (suspend_grace 600 seconds)
C: [return to foreground after 2 minutes] exceeded heartbeat_timeout → discard connection → connect → Resume(K, last_server_seq=120)
S: buffer has 121..125 → ResumeOk(Replay) → resend seq 121..125 → InGameSpace snapshot
C: process resent messages → apply snapshot → Online
```

### D. Extended Time in the Background (Grace Period Expired)
```
C: [return to foreground after 15 minutes] → Resume(K, ...)
S: ResumeFail(Expired)                       (Player was already saved and logged out)
C: discard K → Auth(login_token) → AuthOk(K2) → reload character
```

### E. Relaunch After the App Process Is Killed (Within Grace Period)
```
C: [relaunch, no K] → Auth(login_token)
S: find Detached session for the same account → duplicate connection (quiet takeover) → clean up old session
   → new session K2, hand over Player → AuthOk(K2) → full snapshot (Resync)
C: Online
```

### F. Login on Another Device
```
Device 2: Auth(login_token)
S: Device 1 session is Active → Close(DuplicateLogin, reconnect_allowed=false) to Device 1 → after cleanup,
   AuthOk(K2) + full snapshot to Device 2
Device 1: Terminated, "You signed in on another device." Don't reconnect.
```

### G. Switching from Wi-Fi to LTE
```
C: network change event → discard connection → Resume(K, last_server_seq)
S: old connection is still Active (half-open) → old connection Superseded → ResumeOk(Replay)
```

### H. Purchase Request Sent Just Before Disconnect
```
C: connection drops right after sending BuyItem(seq=7). No ack received → retain request
C: Resume(K, ...) → S: ResumeOk(last_client_seq=7)   (server already received it)
C: don't resend seq 7 (7 or lower)
   If last_client_seq=6 instead → resend BuyItem(seq=7) with the original seq → server processes it once
```

---

## 9. Precedents and What We Adopted

| Precedent | What we adopted |
|---|---|
| MQTT 5 (Clean Start, Session Expiry Interval, CONNACK Session Present) | A `mode` in the resume response tells the client whether session state is valid. The client can change how long the session is kept before disconnecting (`Suspend` → `suspend_grace`) |
| Colyseus (`reconnectionToken`, `allowReconnection`) | The client keeps a reconnect token, and the server holds its place for a set time |
| Nakama (session token, refresh token) | Separate proof of login (`login_token`) from proof of session (`session_key`) |
| ASP.NET Core SignalR stateful reconnect | Use seq/ack and a replay buffer to resend missed messages on reconnect |
| TLS 1.3 session resumption | Treat the resumption key as an opaque value whose meaning only the server knows. Use TLS in production to prevent eavesdropping on the key |

## 10. Verification

- The `bot` crate in the server repository serves as the **reference client implementation** for this contract. Turn the scenarios in section 8 into automated tests.
- Client implementations for each engine MUST pass the same scenarios. The server provides sample packet bytes for each scenario.
- When this document changes, increment `protocol_version` and update the server and all clients together.

## Sources

- MQTT 5 Clean Start and Session Expiry: https://www.emqx.com/en/blog/mqtt5-new-feature-clean-start-and-session-expiry-interval , https://www.hivemq.com/blog/mqtt5-essentials-part4-session-and-message-expiry/
- Colyseus reconnection: https://docs.colyseus.io/sdk/connection , https://docs.colyseus.io/room/reconnection
- Nakama sessions: https://heroiclabs.com/docs/nakama/concepts/session/
- SignalR stateful reconnect: https://github.com/dotnet/aspnetcore/pull/66771
- TLS 1.3: https://datatracker.ietf.org/doc/html/rfc8446 , https://www.rfc-editor.org/rfc/rfc9846.html
- iOS background execution time: https://developer.apple.com/documentation/uikit/extending-your-app-s-background-execution-time , https://developer.apple.com/documentation/uikit/uiapplication/beginbackgroundtask(expirationhandler:)
- Unity lifecycle: https://docs.unity3d.com/6000.2/Documentation/ScriptReference/MonoBehaviour.OnApplicationPause.html , https://discussions.unity.com/t/can-somebody-explain-the-onapplicationpause-focus-scenarios/76324
- Unreal lifecycle: https://dev.epicgames.com/documentation/unreal-engine/API/Runtime/Engine/UApplicationLifecycleComponent
