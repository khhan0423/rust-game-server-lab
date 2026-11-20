# Pilot Message Catalog

**English** | [한국어](message-catalog.ko.md)

- Status: **Draft** (2026-11-20)

## Client frame

Follow [Session and Network Design](../design/02-session-network.md), sections 7 and 8. The 16-byte header is `len:u32`, `msg:u16`, `flags:u16`, `seq:u32`, `ack:u32`, in that order and all little-endian. `len` includes the header. Bit 0 is FRAGMENT, bit 1 is RELIABLE, and all other bits must be zero. An out-of-range length, reserved flag, or client-sent FRAGMENT frame closes the connection without a Close notice. Server fragments carry `transfer_id:u32`, `index:u16`, and `count:u16` after the common header.

Reliable frame sequences start at 1 per session and per direction. Volatile frames use `seq = 0`. `ack` is cumulative and acknowledges the last peer Reliable message processed. The Ack payload must match its header.

## Message IDs

The Gateway classifies packets by ID range alone. System (`0x0000` to `0x0FFF`) goes to the Gateway, Content (`0x1000` to `0x7FFF`) to the home shard, and Zone (`0x8000` to `0xFFFF`) to the current space host. EnterGame is a System request; the Gateway handles it and asks the home shard to admit the player.

Allocate 32 IDs per feature block and leave gaps. Only unassigned IDs may be used for new messages. Never renumber or reuse an assigned ID. C means client and S means server side. Numbers in parentheses are field tags.

## System

| ID | Message | Direction | Delivery | Fields |
|---|---|---|---|---|
| `0x0001` | `Auth` | C→S | Volatile | `login_token` bytes (1), `protocol_version` uint32 (2), `client_version` string (3), `platform` string (4) |
| `0x0002` | `AuthOk` | S→C | Volatile | `session_key` bytes (1), `player_tag` string (2), `nickname` string (3), `server_time` uint64 (4), `timers` Timers (5) |
| `0x0003` | `Close` | S→C | Volatile | `reason` CloseReason (1), `reconnect_allowed` bool (2), `message` optional string (3) |
| `0x0004` | `Resume` | C→S | Volatile | `session_key` bytes (1), `last_server_seq` Sequence (2), `protocol_version` uint32 (3), `client_version` string (4) |
| `0x0005` | `ResumeOk` | S→C | Volatile | `mode` ResumeMode (1), `last_client_seq` Sequence (2), `server_time` uint64 (3), `timers` Timers (4) |
| `0x0006` | `ResumeFail` | S→C | Volatile | `reason` ResumeFailReason (1) |
| `0x0020` | `Ping` | C→S | Volatile | `client_time` uint64 (1) |
| `0x0021` | `Pong` | S→C | Volatile | `client_time` uint64 (1), `server_time` uint64 (2) |
| `0x0022` | `Ack` | C↔S | Volatile | `ack` Sequence (1) |
| `0x0023` | `Suspend` | C→S | Volatile | None |
| `0x0024` | `Logout` | C→S | Volatile | None |
| `0x0040` | `EnterGame` | C→S | Reliable | None |
| `0x0041` | `EnterGameResult` | S→C | Reliable | `status` ActionStatus (1) |

## Content

| ID | Message | Direction | Delivery | Fields |
|---|---|---|---|---|
| `0x1000` | `BuyItem` | C→S | Reliable | `request_id` RequestId (1), `item_code` uint32 (2), `count` uint32 (3) |
| `0x1001` | `BuyItemResult` | S→C | Reliable | `request_id` RequestId (1), `status` ActionStatus (2) |
| `0x1020` | `InventoryChanged` | S→C | Reliable | `upserted` repeated ItemStack (1), `removed` repeated ItemInstanceId (2), `balance` Money (3) |
| `0x1021` | `InventoryList` | C→S | Reliable | `after` ItemInstanceId (1) |
| `0x1022` | `InventoryPage` | S→C | Reliable | `items` repeated ItemStack (1), `balance` Money (2), `next` ItemInstanceId (3) |
| `0x1040` | `SendMail` | C→S | Reliable | `request_id` RequestId (1), `recipient_tag` string (2), `item_id` ItemInstanceId (3), `count` uint32 (4) |
| `0x1041` | `SendMailResult` | S→C | Reliable | `request_id` RequestId (1), `status` ActionStatus (2) |
| `0x1042` | `ListMail` | C→S | Reliable | `after` MailId (1) |
| `0x1043` | `MailPage` | S→C | Reliable | `entries` repeated MailEntry (1), `next` MailId (2) |
| `0x1044` | `ReceiveMail` | C→S | Reliable | `request_id` RequestId (1), `mail_id` MailId (2) |
| `0x1045` | `ReceiveMailResult` | S→C | Reliable | `request_id` RequestId (1), `status` ActionStatus (2) |
| `0x1046` | `MailArrived` | S→C | Reliable | `entry` MailEntry (1) |

## Zone

| ID | Message | Direction | Delivery | Fields |
|---|---|---|---|---|
| `0x8000` | `EnterSpace` | C→S | Reliable | None |
| `0x8001` | `EnterSpaceResult` | S→C | Reliable | `status` ActionStatus (1) |
| `0x8002` | `LeaveSpace` | C→S | Reliable | None |
| `0x8003` | `LeaveSpaceResult` | S→C | Reliable | `status` ActionStatus (1) |
| `0x8020` | `Move` | C→S | Volatile | `position` Position (1) |
| `0x8021` | `SpaceSnapshot` | S→C | Volatile | `self_entity` EntityNumber (1), `entities` repeated SpaceEntity (2) |
| `0x8022` | `EntityAppeared` | S→C | Volatile | `entity` SpaceEntity (1) |
| `0x8023` | `EntityDisappeared` | S→C | Volatile | `entity` EntityNumber (1) |
| `0x8024` | `EntityMoved` | S→C | Volatile | `entity` EntityNumber (1), `position` Position (2) |

## Server envelope and controls

The 15-byte header follows design 02, section 4.2: `len:u32`, `kind:u8`, `session:u64`, `msg:u16`, in order. Length includes the header and multi-byte values are little-endian. Kind 0 relays the client message ID and payload unchanged. Kind 1 carries server-only controls in a separate ID space. Any other kind is an error. G means Gateway and B means a server link peer. ServerHello uses session number 0. Client seq, ack, and replay rules do not apply to controls. Cross-shard mail delivery belongs to the separate P6 link work.

| ID | Message | Direction | Delivery | Fields |
|---|---|---|---|---|
| `0x0001` | `ServerHello` | B↔B | N/A | `server` ServerAddress (1), `protocol_version` uint32 (2) |
| `0x0020` | `SessionAttached` | G→B | N/A | `player_id` PlayerId (1) |
| `0x0021` | `SessionBound` | B→G | N/A | `player_id` PlayerId (1), `content` ServerAddress (2), `zone` ServerAddress (3) |
| `0x0022` | `SessionDetached` | G→B | N/A | `reason` DetachReason (1) |
| `0x0023` | `SessionSuspended` | G→B | N/A | None |
| `0x0024` | `SessionResumed` | G→B | N/A | `mode` ResumeMode (1), `gateway` ServerAddress (2) |
| `0x0025` | `SessionReplaced` | G→B | N/A | `gateway` ServerAddress (1), `new_session` SessionNumber (2) |
| `0x0026` | `SessionClosed` | G→B | N/A | None |
| `0x0027` | `RouteUpdate` | B→B | N/A | `zone_index` uint32 (1) |
| `0x0028` | `SwitchZone` | B→G | N/A | `zone` ServerAddress (1) |
| `0x0040` | `Kick` | G→G | N/A | `reason` CloseReason (1) |
| `0x0041` | `CleanupComplete` | G→G | N/A | None |
| `0x0042` | `Superseded` | G→G | N/A | None |

## Shared field types

| Type | Fields |
|---|---|
| `Timers` | `heartbeat_interval` uint32 (1), `heartbeat_timeout` uint32 (2), `foreground_probe` uint32 (3), `resume_grace` uint32 (4), `suspend_grace` uint32 (5), `ack_interval` uint32 (6), `handshake_timeout` uint32 (7) |
| `Position` | `x` sint32 (1), `y` sint32 (2) |
| `SpaceEntity` | `number` EntityNumber (1), `player_tag` string (2), `nickname` string (3), `position` Position (4) |
| `ItemStack` | `item_id` ItemInstanceId (1), `item_code` uint32 (2), `count` uint32 (3) |
| `MailEntry` | `mail_id` MailId (1), `sender_tag` string (2), `sender_nickname` string (3), `item_code` uint32 (4), `count` uint32 (5) |
| `ServerAddress` | `shard` uint32 (1), `role` ServerRole (2), `instance` uint32 (3) |
| `Sequence` | `value` uint32 (1) |
| `EntityNumber` | `value` uint32 (1) |
| `Money` | `value` int64 (1) |
| `RequestId` | `value` bytes (1) |
| `ItemInstanceId` | `value` bytes (1) |
| `MailId` | `value` bytes (1) |
| `PlayerId` | `value` bytes (1) |
| `SessionNumber` | `value` uint64 (1) |

Enum values start at 0 in the order shown. Unspecified denotes an unset value and is not valid.

| Enum | Values |
|---|---|
| `CloseReason` | Unspecified, Logout, DuplicateLogin, Maintenance, UpdateRequired, Banned, ServerShutdown, IdleTimeout, AuthenticationFailed |
| `ResumeMode` | Unspecified, Replay, Resync |
| `ResumeFailReason` | Unspecified, Expired, Unknown, Closed, VersionMismatch |
| `ActionStatus` | Unspecified, Ok, Rejected |
| `ServerRole` | Unspecified, Gateway, Game |
| `DetachReason` | Unspecified, PeerClosed, PeerReset, WriteFailed, KeepaliveTimeout, HeartbeatTimeout, HandshakeTimeout, SendQueueOverflow, ProtocolViolation, LocalNetwork |

## Decided in this document

- The tables fix the pilot IDs, directions, delivery classes, payload names, field tags, wire types, and enum values. The English ID rows generate the Rust registry; the build checks the Korean rows against them.
- Client frames and backend envelopes each allow at most 65,536 bytes including their respective headers. The backend envelope uses little-endian lengths and IDs.
- Handshake, heartbeat, Close, ACK, Suspend, and Logout use unsequenced Volatile frames. Close may carry a display message, and AuthenticationFailed covers refused authentication.
- Timestamps are Unix milliseconds and timers are seconds. Client version and platform are strings. The session key and login token are opaque bytes; internal IDs must not be readable from a client-held token. Token protection belongs to the issuer and Gateway.
- Player tags are sent in canonical form. Positions are two signed centimetre coordinates; entity numbers apply only to the current space visit. Movement sends an absolute position. In the pilot, EnterSpace targets the player’s own lobby and LeaveSpace keeps the session active.
- Request, item instance, mail and internal player IDs use 16-byte UUID v7 values. Counts are unsigned integers; balances use signed 64-bit minor units. Success and rejection share ActionStatus.
- Inventory changes carry absolute stack counts, removals, and balance. Inventory and mail start paging with an absent cursor, advance with the returned exclusive cursor, and end when the next cursor is absent. Mail send addresses a public tag; receiving mail claims the attachment by mail ID.
- Control IDs are separate from client IDs. Session controls use the envelope session number unless replacing it. Server addresses carry shard, role, and instance; shard and instance are 16-bit values. Cross-shard delivery is separate from the Gateway envelope.
