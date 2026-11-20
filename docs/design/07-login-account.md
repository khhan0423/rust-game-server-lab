# Design 07. Login and Accounts: Publisher Login and Game Entry

**English** | [한국어](07-login-account.ko.md)

- Status: **Draft** (2026-11-18)
- Related documents: `02-session-network.md` (Gateway and sessions), `04-database.md` (DB layout), `docs/protocol/session-contract.md` (flow after the game `login_token`)

We publish the game ourselves. We therefore separate the **publisher domain** (platform accounts shared across games) from the **game domain** (this game). This is the same structure used by large publisher platforms and Steam.

## 0. Decision Summary

| # | Decision | Details |
|---|---|---|
| L1 | Default login | **Start as a guest and link a platform login later.** The same structure also supports logging in with a platform from the start |
| L2 | Supported platforms | **Google, Apple, Steam** + Guest. Keep all options open and **implement Steam first** (owner decision, 2026-11-19). See document 09, section 1.1 for the Steam login procedure |
| L3 | Accounts and login methods | **Separate them.** Link multiple login methods to one publisher account (section 2) |
| L4 | Characters | **One character per account in this game.** This is a subculture game. The two gendered appearance presets are attributes of that character (section 5) |
| L5 | Linking conflicts | **Do not merge automatically.** Switch to the existing account or cancel (2.3). Final |
| L6 | Domain separation | **The Login server is a publisher service (web server) backed by LoginDB.** The game starts **at the load balancer**. The game uses the publisher API only to verify whether the user is authentic, the account is valid, and the user is a member of this game (section 1) |
| L7 | Game account DB | The game's **AccountDB** uses the publisher's **AccountID as its unique key**. It records this game's character (`PlayerId`) and home shard (section 6) |
| L6a | Game Entry server | **Use a separate Entry server** as the first point of entry on the game side (verify publisher tickets, create characters, assign home shards, issue game `login_token`). Gateway only verifies the signature and has no DB. Final on 2026-11-18 |
| L8 | Home shard assignment | The **game Entry server** assigns a home shard on first entry. Returning players use the home shard already recorded in AccountDB (section 6) |

---

## 1. Overall Structure

```
 ┌──────────────────────── Publisher domain ───────────────────────────────────┐
 │                                                                            │
 │  Client ──Login (Guest/Google/Apple/Steam)──> Login server (web server)        │
 │                                              │ LoginDB: publisher accounts, │
 │                                              │ login methods, owned games,  │
 │                                              │ membership, platform sanctions
 │  Client <── Game entry ticket + game entry address ──┘                      │
 │                                                                            │
 └────────────────────────────────────────────────────────────────────────────┘
 ┌───────────────────────────── Game domain ───────────────────────────────────┐
 │                                                                            │
 │  Client ──ticket──> Load balancer ──> Entry server (game login)               │
 │                           ① Verify ticket ──publisher API──> Login server   │
 │                           ② AccountDB: look up AccountID. If absent,       │
 │                              create character + assign home shard          │
 │  Client <── Game login_token + Gateway address ──┘                          │
 │                                                                            │
 │  Client ──TCP──> Load balancer ──> Gateway ──> Game server ... (as in doc 02) │
 └────────────────────────────────────────────────────────────────────────────┘
```

| | Publisher Login server | Game Entry server |
|---|---|---|
| Domain | Publisher. Shared across games | Dedicated to this game |
| Form | Web server (HTTPS) | Web server (HTTPS). Stateless, with multiple instances behind a load balancer |
| Responsibilities | Create publisher accounts, link login methods, track owned games and membership, enforce platform sanctions, issue game entry tickets, provide the ticket verification API | Request ticket verification, manage this game's accounts and characters, assign home shards, check game sanctions, issue game `login_token`, check maintenance and version, manage the login queue |
| DB | **LoginDB** | **AccountDB** (game) |
| Game data | None | Only character existence and home shard |

- **As before, Gateway only verifies the signature of the game `login_token` and has no DB.** The Entry server makes publisher API calls (external HTTP) and writes to the DB. The Gateway design in the session contract and document 02 therefore stays the same.
- The load balancer distributes connections, the Entry server assigns the home shard, and Gateway routes only to the destination in the token. The three roles do not overlap.

---

## 2. Publisher Login Server (Publisher Domain)

This section is outside the game server implementation scope. It records only the **contract** the game expects.

### 2.1 Accounts and Login Methods

```
Publisher account (AccountID)    ← Unique across the publisher, immutable
   ├─ Login method: Guest  (server-issued secret)
   ├─ Login method: Google (subject = Google user ID)
   ├─ Login method: Apple  (subject = Apple user ID)
   └─ Login method: Steam  (subject = SteamID64)
```

Login always follows the same rule: **"Find the account linked to this login method `(provider, subject)`. Create a new account if there is none."**

| Case | Handling |
|---|---|
| Start as a guest | Create a Guest login method → create a new account and link it |
| Guest links a platform | **Add** the platform login method to the current account |
| Log in with a platform from the start | Since the login method does not exist yet, create a new account and link it. Same path as starting as a guest |
| Log in with a platform from another device | Find the existing account using that login method and enter with the same account |

Rules: one login method per platform per account; the last login method cannot be unlinked; each `(provider, subject)` links to only one account (unique).

### 2.2 Authentication Methods

| provider | What the client receives | What the Login server verifies |
|---|---|---|
| Guest | A random secret issued by the server on first launch (stored in iOS Keychain or Android Keystore) | Compare against the secret's hash. Store only the hash in the DB |
| Google | Google ID token | Use Google's public key to verify the signature, and check the audience and expiration |
| Apple | Apple identity token | Use Apple's public key to verify the signature, and check the audience and expiration |
| Steam | Steam authentication ticket | Verify through the Steam Web API |

- Do not use device IDs (such as advertising IDs) to identify guests.
- If we offer social login on iOS, Apple's review guidelines require Sign in with Apple (or an equivalent login) as well. Check the current guidelines before release.

### 2.3 Linking and Exceptional Cases

| Situation | Handling |
|---|---|
| The platform login method is already linked to another account | **Do not merge automatically.** Choose ① switch to that existing account or ② cancel |
| A guest deletes the app or changes devices before linking | The account is lost. Encourage linking in the game (for example, offer a reward for linking) |
| Attempt to unlink the last login method | Reject |
| Account closure | Publisher policy. Notify the game through the notification mechanism (2.5) |

### 2.4 LoginDB Schema (Draft, Publisher-Owned)

```sql
CREATE TABLE accounts (
    account_id   text PRIMARY KEY,             -- Publisher AccountID (key passed to the game)
    status       smallint NOT NULL,            -- Active / platform sanction / pending closure
    created_at   timestamptz NOT NULL
);

CREATE TABLE account_identities (
    provider     smallint NOT NULL,            -- Guest / Google / Apple / Steam
    subject      text NOT NULL,
    account_id   text NOT NULL REFERENCES accounts(account_id),
    secret_hash  bytea,                        -- Guest only
    linked_at    timestamptz NOT NULL,
    PRIMARY KEY (provider, subject),
    UNIQUE (account_id, provider)
);

CREATE TABLE game_memberships (                -- Which games the account owns and whether it is a member
    account_id   text NOT NULL REFERENCES accounts(account_id),
    game_id      text NOT NULL,
    status       smallint NOT NULL,            -- Member / canceled / platform sanction specific to the game
    joined_at    timestamptz NOT NULL,
    PRIMARY KEY (account_id, game_id)
);
```

Publisher data such as terms acceptance records, personal information, and payment methods is added here.

### 2.5 Contract with the Game

| Contract | Details |
|---|---|
| **Game entry ticket** | Short-lived (for example, 1 minute), **single-use**, opaque value bound to `game_id` |
| **Ticket verification API** (game → publisher) | `VerifyTicket(ticket, game_id)` → `{ account_id, membership status, platform sanction status, required information (country, age category, etc.) }`. Verification consumes the ticket |
| **Notification** (publisher → game, webhook) | Platform sanction, account closure, membership cancellation. The game kicks the affected player and updates the status |
| Game → publisher authentication | Server-to-server authentication (API key or mTLS). Private network or allowlist |

### 2.6 Pilot Implementation: Guest Login API

The pilot builds the Login server with the Guest method only. The other methods attach to the same structure later. The server is the `publisher` crate; its SQL lives in the `login` module of the `db` crate. Game-side crates never touch LoginDB.

All requests and responses are JSON over HTTP. An error response is `{ "error": "<code>" }` with the status shown below.

| Call | Who calls it | Request | Success response |
|---|---|---|---|
| `POST /v1/guest/accounts` | Client, on first launch | `{}` | `201` `{ "guest_id", "secret" }`. The secret is returned only this once |
| `POST /v1/guest/login` | Client | `{ "guest_id", "secret", "game_id" }` | `200` `{ "ticket", "expires_in_secs", "entry_url" }` |
| `POST /v1/tickets/verify` | Game Entry server, with the `x-api-key` header | `{ "ticket", "game_id" }` | `200` `{ "account_id", "membership", "sanction" }`. Verification consumes the ticket |

| Error code | Status | When |
|---|---|---|
| `invalid_request` | 400 | The body is missing a field or is malformed |
| `invalid_credentials` | 401 | Unknown guest ID or wrong secret. The two cases are not distinguished |
| `unauthorized` | 401 | The verification call has no API key or a wrong one |
| `account_blocked` | 403 | The account is under a platform sanction or pending closure |
| `unknown_game` | 404 | The `game_id` is not configured on the Login server |
| `ticket_invalid` | 404 | The ticket is unknown, expired, already used, or was issued for another game. The cases are not distinguished |
| `internal_error` | 500 | The server failed for a reason of its own, such as the database being unreachable. Details go to the operations log, not to the caller |

- Responses to the client carry no account ID. Internal keys stay on the server side (section 5.1); only the verification call, which is server to server, returns `account_id`.
- `account_id` and `guest_id` are UUID v7 strings. `secret` and `ticket` are 32 random bytes written as 64 hexadecimal characters.
- LoginDB stores only the SHA-256 hash of a secret or a ticket. The pilot computes the hash with the database's built-in `sha256()`. Before this server handles real users, hashing moves into the server process so the plain value never reaches the database.
- In the verification response, `membership` is one of `"member"`, `"canceled"`, `"sanctioned"` (the account's state in that game) and `sanction` is one of `"none"`, `"sanctioned"`, `"closing"` (the account's platform state). Both are strings, so the database encoding is not exposed and values can be added later.
- Verification always consumes a valid ticket and reports the current states. It does not reject because of a sanction; the game's Entry server decides, and it admits a player only when `membership` is `"member"` and `sanction` is `"none"`.
- Login is where the Login server refuses: it answers `account_blocked` when the account is not active or its membership in that game is sanctioned. A canceled membership becomes a member again on login.
- A ticket lives for 60 seconds by default (a config value) and is single-use. Consuming it is one conditional `UPDATE`; the number of affected rows decides success.
- The first login for a game creates that account's membership. Checking ownership of a paid game comes with the Steam method.
- The API key for the verification call and the LoginDB connection string are secrets and come from environment variables.
- Tickets are stored in a table that the draft in 2.4 did not have:

```sql
CREATE TABLE entry_tickets (
    ticket_hash  bytea PRIMARY KEY,
    account_id   text NOT NULL REFERENCES accounts(account_id),
    game_id      text NOT NULL,
    expires_at   timestamptz NOT NULL,
    consumed_at  timestamptz
);
```

---

## 3. Game Entry Server (Game Domain)

### 3.1 Entry Flow

```
Client ──EnterRequest(ticket)──> Load balancer ──> Entry server
Entry: ① Publisher VerifyTicket(ticket, game_id)
          Failure, nonmember, or platform sanction → reject
       ② Check maintenance status and client version
       ③ AccountDB: look up by account_id
            Found → use recorded player_id and home_shard. Check game sanctions
            Absent → create character + assign home shard (section 4) → record in AccountDB (one transaction)
       ④ If concurrent users exceed the limit, use the login queue (document 03, section 9.8)
       ⑤ Issue game login_token (signed)
Client <── login_token + Gateway address ──
Client ──TCP──> Load balancer ──> Gateway ──Auth(login_token)──> ... (session contract)
```

- **Do not trust account information sent by the client.** The game must use only the `account_id` and information in the publisher API **response**.
- Call the publisher API **once on entry**. Do not call it for each packet.
- Reconnect (`Resume`) bypasses the Entry server. The client resumes directly at Gateway with `session_key` (session contract).

### 3.2 Game `login_token`

- This is a short-lived token signed by the Entry server. It contains `account_id`, `player_id`, the home shard, the expiration time, and the signing key ID.
- Gateway **only verifies the signature** and does not query the DB (document 04, section 2).
- This token is separate from the publisher's entry ticket. The Entry server consumes the publisher ticket, which ends its use.

---

## 4. Home Shard Assignment

| Decision | Duration | Basis | Owner |
|---|---|---|---|
| Which Gateway accepts the TCP connection | For one connection | Current connection count | Load balancer |
| **This player's home shard** | **Permanent** (once on first entry) | **Total players assigned** per shard, operator-designated recommendation | **Entry server** |
| Which server gets the message | Per message | Destination in the token and session | Gateway (follows the destination without making a decision) |

- **Friend code (owner decision, 2026-11-19):** If a player enters a friend code on first entry, assign them to that friend's home shard. If the shard is closed to new players, follow the default rule below and tell the player.
- Default rule (no friend code): **Use the open shard with the fewest total players assigned.** If an operator designated a "recommended shard," give it priority. Read the shard list and open status from the configuration in Common DB.
- Use the **total number of players assigned** rather than current load because the assignment is permanent. Sending too many new players to a quiet shard in the early morning will overload it in the evening.
- When a shard has multiple Game server processes, deciding "which process handles this player's content" is a separate problem covered in cluster document 06.

---

## 5. Characters

- **This game has one character per account.** On first entry, the Entry server creates a `PlayerId` (UUID v7) and records it in AccountDB. The character's initial game data is created in the home shard's Game DB (an idempotent creation request to the home shard Game server).
- Keep `AccountId` (publisher) and `PlayerId` (game) separate. This keeps game data independent of the publisher ID format and lets operations such as server transfers handle them differently.
- The **two gendered appearance presets** are attributes of one character. Store both presets' appearance data and the "active preset" in the character data in Game DB. Handle switching through a Content message on the home shard and apply the result (the appearance in `CombatProfile`) to the current InGameSpace.
- There is no character selection step. Game entry uses `EnterGame` (document 02).

### 5.1 Player Tag and Nickname (owner decision, 2026-11-20)

Players need a way to point at each other for friend requests, whispers, and invitations. The internal keys cannot serve: they can never be changed once known, `PlayerId` and `AccountId` reveal when the account was made, and nobody can type 36 characters. So each character has two public names, and the internal keys never reach a game client.

| | Player tag | Nickname |
|---|---|---|
| What it is for | Finding one exact player: friend requests, friend codes, support tickets | The name shown in the world, in chat, and in rankings |
| Who chooses it | The Entry server, at random | The player, at first entry |
| Unique | Across the whole game | Across the whole game |
| Can change | Yes, by reissue | Policy not decided yet |

**Player tag**

- Nine characters drawn at random from capital letters and digits, leaving out `0`, `O`, `1`, `I`, and `L` (31 symbols, about 26 trillion combinations). Shown in groups of three, such as `K7Q-M2X-RP4`. Stored without the hyphens. Input is accepted with or without hyphens and in either case.
- Never sequential, so a tag says nothing about how many players exist and tags cannot be walked in order.
- The Entry server issues it at first entry. AccountDB's unique constraint guards uniqueness; on a collision the server draws again.
- **The friend code is the player tag.** The friend code a new player enters to join a friend's shard (section 4) is that friend's tag.
- A player can have the tag reissued once every 30 days (a config value); an operator can reissue it at any time. The old tag stops working at once. Friend relationships are stored by `PlayerId`, so they survive a reissue.
- Looking up a tag is rate limited per player, so tags cannot be guessed in bulk.

**Nickname**

- Unique across the whole game, not per shard. Players from different shards meet in fields, dungeons, and parties, and they must be told apart by name alone.
- AccountDB keeps the name as typed for display and a normalized form (case folded and Unicode normalized, so lookalike spellings count as the same name) in a separate unique column. The unique constraint decides who gets a name when two players ask at once.
- Allowed characters, length, a blocked word list, and the renaming policy are not decided yet (section 8).

**What a game client receives**

- About another player: nickname and player tag. Inside a space, players are referred to by a number that is valid only in that space and is assigned again on every entry.
- `PlayerId` and `AccountId` are never sent to a game client, not even the player's own.

**First entry needs a nickname**

A ticket is single-use, so the Entry server cannot ask for a nickname and then verify the same ticket again. First entry therefore takes two steps:

1. The client sends the ticket. The Entry server verifies it. If AccountDB has no character for the account, the server answers "character required" with a short-lived creation token that it signed for that account.
2. The client sends the creation token, the nickname, and optionally a friend code. The Entry server inserts the account row, the player tag, and the nickname in one transaction. If the nickname is taken, it answers so, and the client tries another name with the same token.

---

## 6. AccountDB Schema (Draft, Game-Owned)

```sql
CREATE TABLE game_accounts (
    account_id   text PRIMARY KEY,             -- AccountID supplied by the publisher (unique key)
    player_id    uuid NOT NULL UNIQUE,         -- One character. Never sent to a game client
    player_tag   text NOT NULL UNIQUE,         -- Public tag, also the friend code (section 5.1)
    tag_issued_at timestamptz NOT NULL,        -- For the reissue interval
    nickname     text NOT NULL,                -- As typed, for display
    nickname_key text NOT NULL UNIQUE,         -- Normalized form; decides uniqueness
    home_shard   smallint NOT NULL,            -- Source of truth for the player directory (document 03, section 7.2)
    status       smallint NOT NULL,            -- Game sanctions and status based on publisher notifications
    created_at   timestamptz NOT NULL
);

CREATE TABLE game_sanctions (
    account_id   text NOT NULL REFERENCES game_accounts(account_id),
    kind         smallint NOT NULL,
    until_at     timestamptz,
    reason       text NOT NULL,
    created_at   timestamptz NOT NULL
);
```

- **If two first entry requests arrive at once** (first entry from two devices at the same time), only one `INSERT` succeeds because of the `account_id` primary key. The request that fails looks up the account again and uses the character created first (query after `INSERT ... ON CONFLICT DO NOTHING`).
- When the Game server looks up the player directory (`PlayerId → home shard`), it checks the Redis cache (`home:{player}`) first. If there is no entry, it asks the Entry server. The Entry server owns AccountDB.

---

## 7. Systems That Come with Being the Publisher (Outside This Document's Scope)

| System | Domain |
|---|---|
| Payment verification (store receipts, refund notifications) | Publisher or game. The game grants currency idempotently. **Since Steam was chosen as the platform (2026-11-19), follow `09-steam-commerce.md`.** Document 09, S7 decides whether to replace this document's mobile assumptions (Guest, device-stored secret) with Steam login |
| Operations tool (GM tool) | The publisher manages publisher accounts. The game handles game data queries and grants |
| Customer support queries | Transaction logs (Game DB) and analytics logs (Log DB) |
| Push notifications, terms of service, personal information | Publisher |

## 8. Remaining Decisions

| # | Question | Notes |
|---|---|---|
| L9 | Game data retention after account closure | **Provisional (2026-11-19):** Keep data for 6 months after account closure (recovery is possible), then move it to an archive without deleting it. Privacy laws may require destroying or separately storing personally identifiable information (platform account links, contact details, etc.). Confirm with legal before finalizing the plan to retain only game data in the archive |
| L10 | Behavior when publisher authentication verification is unavailable (including Steam) | **Final (2026-11-19):** If verification is unavailable, reject only new entries. Players already connected are unaffected |
| - | Nickname rules | Allowed characters, minimum and maximum length, and the blocked word list. Needed before the Entry server accepts real names |
| - | Nickname changes | Whether a player can rename, how often or at what cost, and how long the old name stays reserved |
