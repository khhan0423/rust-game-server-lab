# Design 09. Steam-Based Payments and Item Trading

**English** | [한국어](09-steam-commerce.ko.md)

- Status: **Template** (2026-11-19). Implementation comes later. For now, define only the flows, data ownership, and decisions to make
- Platform assumption: **Keep all payment methods open and build Steam first** (owner decision, 2026-11-19). The Commerce service plugs each payment integration (Steam, then other stores) into the same order states (2.2) and grant path (S5). This document covers Steam integration
- Related documents: `07-login-account.md` (login), `03-data-ownership.md` (one DB per step, idempotent messages, outbox), `04-database.md` (DB layout), `08-operations-observability.md` section 8 (secrets)
- Official documentation (checked 2026-11-19):
  - In-game purchases: https://partner.steamgames.com/doc/features/microtransactions/implementation
  - In-game purchase Web API: https://partner.steamgames.com/doc/webapi/ISteamMicroTxn
  - Inventory Service: https://partner.steamgames.com/doc/features/inventory
  - Item definitions (ItemDef): https://partner.steamgames.com/doc/features/inventory/schema
  - Inventory Web API: https://partner.steamgames.com/doc/webapi/IInventoryService
  - User authentication: https://partner.steamgames.com/doc/features/auth
  - Community Market FAQ: https://help.steampowered.com/en/faqs/view/61F0-72B7-9A18-C70B

## 0. Decision Summary

| # | Decision | Details |
|---|---|---|
| S1 | Two features | **(A) In-game purchases**: players buy our products (currency and bundles) using their Steam Wallet. **(B) Steam items**: put some items in Steam Inventory so players can buy and sell them to each other on the Community Market |
| S2 | Where Steam calls happen | Only the new cluster service, the **Commerce service**, calls the Steam Web API. Only this service holds the Steam publisher key |
| S3 | Owner of authoritative data | (A) **Our DB** is authoritative for order state and grant history. (B) **Steam Inventory is authoritative** for Steam items. Our DB holds only a copy |
| S4 | Scope of Steam items (recommended) | **Cosmetics only.** Do not make items that affect combat power or currency into Steam items (4.5) |
| S5 | Grants | Once payment is finalized, send the grant to the home shard player actor as an idempotent message (`GrantItem`), as described in document 03 |
| S6 | Refunds and revocation | The Commerce service periodically reads the Steam settlement report (`GetReport`) to find refunds and chargebacks, then sends revocation messages |

---

## 1. Overall Structure

```
                       ┌───────────── Steam (Valve) ──────────────┐
                       │ User auth    In-game purchases Inventory │
                       │ (UserAuth)   (MicroTxn)        (Inventory)│
                       │                               Market     │
                       └───▲──────────────▲────────────────▲───────┘
                           │ HTTPS + publisher key (server only)
 Client ── Steamworks SDK (overlay, callbacks)
     │
     │ (game packets)
     ▼
  Gateway ─► Home shard Game server ── Player actor ──message──► Commerce service ──► Commerce DB
                                             ▲                       │
                                             └── GrantItem / Revoke ─┘ (idempotent messages, outbox)
```

- **The client does not call the Steam Web API directly.** Calls that require the publisher key (grants, revocations, order finalization) must come from the server (see the NOTE in the Inventory Web API documentation). The client uses only the Steamworks SDK overlay (payment approval dialog) and callbacks.
- The **Commerce service** is the same kind of cluster service as those in document 03, section 8 (such as the trade service). It is an async service. Steam calls spend most of their time waiting, so handle them with an actor and `.await` (as in document 01, section 5).
- **Commerce DB** holds orders and Steam item operation records. This data is global and independent of the player's home shard, so it does not go in a shard Game DB. S9 decides whether to use a separate DB or a schema within Account DB.

### 1.1 Relationship to Login

For a Steam game, **the Steam account itself is a login method**.

1. The client gets a ticket with `ISteamUser::GetAuthTicketForWebApi`.
2. The server verifies the ticket with `ISteamUserAuth/AuthenticateUserTicket` and obtains the SteamID (64-bit).
3. If needed, use `ISteamUser/CheckAppOwnership` to check whether the user bought the game.

This flow either replaces the "Publisher Login Server" in document 07 or adds Steam there as one login method. Document 07 assumes mobile (Guest and a secret stored on the device), so **revise it for Steam** (S7, requires owner approval). This document assumes only that a SteamID is linked to the account.

---

## 2. (A) In-Game Purchases: Sell Our Products for Steam Wallet Funds

### 2.1 Flow

This adapts the sequence from the official documentation to our architecture.

```
Client                  Player actor               Commerce service                  Steam
1. Buy in shop ───────► 2. Check product, price ──► 3. Create order (Commerce DB)
                                                      order_id(u64), state Created
                                                   4. InitTxn ─────────────────────► Approval dialog in overlay
                                                      state InitSent
5. Approve in overlay
6. MicroTxnAuthorizationResponse callback
   (order_id, approval status) ──────────────────► 7. FinalizeTxn ─────────────────► Finalize payment
                                                      success: state Finalized
                                                   8. Write outbox in the same transaction:
                                                      GrantItem{order_id-based idempotency key}
                       9. Receive GrantItem ◄─────── (send after commit)
                          Grant in Game DB (same transaction as idempotency key)
                          Return result to client
                                                  10. Confirm grant → state Granted
```

- **The client callback in steps 5 and 6 is only a signal.** Confirm payment only when `FinalizeTxn` succeeds on the server. Even if the client falsely says it approved the payment, do not grant anything if `FinalizeTxn` fails.
- **The order number is 64-bit.** Steam requires a 64-bit Order ID, so UUID v7 (128-bit, document 03, section 7) cannot be used directly. Generate the ID with a sequence in Commerce DB.
- The server sets the price. Do not use the price sent by the client. Get the user's country and currency from `GetUserInfo`.
- Use the sandbox (`ISteamMicroTxnSandbox`) during development. It does not charge real money.

### 2.2 Order States

```rust
enum OrderState {
    Created,      // Recorded in DB, before calling Steam
    InitSent,     // InitTxn succeeded, waiting for user approval
    Finalized,    // FinalizeTxn succeeded = funds were charged. The grant must now be delivered
    Granted,      // Home shard confirmed the grant
    Failed,       // Declined, canceled, timed out
    Reversed,     // Refund, chargeback (2.4)
}
```

### 2.3 Interruptions and Failures (Most Important)

Money is involved, so define what happens if the flow is interrupted at each step.

| Point of interruption | Situation | Handling |
|---|---|---|
| Before 4 | Only the order exists | Set to `Failed` after a set time |
| Between 4 and 6 | User approval is unknown (client closed, callback lost) | After a set time, check the state with `QueryTxn`. If `Approved`, call `FinalizeTxn`; otherwise set to `Failed`. For web payments the official documentation says to abandon the payment if the user does not return. It does not specify what to do for in-game payments. **Confirm the behavior in the sandbox before finalizing this decision** |
| Before the response to 7 | Unknown whether `FinalizeTxn` succeeded (timeout) | Check with `QueryTxn`. If `Succeeded`, advance to `Finalized` |
| After 7 | Funds charged, grant still pending | The outbox is in the DB, so it resends the message even after a restart. The idempotency key ensures the grant is applied once |
| 9 fails (home shard unavailable) | Grant message unacknowledged | Resend from the outbox. Follow the rules in document 03, section 3 |

The design guarantees that **once the order is `Finalized`, the grant will eventually be delivered no matter what happens**.

### 2.4 Refunds, Chargebacks, and Revocation

- Steam reports user refunds, card disputes (chargebacks), and suspected fraud refunds as status changes in `GetReport`. The official documentation says to call it at least daily and permits calls as often as once per minute. The Commerce service calls it **every 5 minutes** (configurable).
- If the status becomes `Refunded`, `PartialRefund`, `Chargedback`, `RefundedSuspectedFraud`, or `RefundedFriendlyFraud`, set the order to `Reversed` and send the home shard an idempotent **revocation message** (`Revoke{order_id}`).
- The official documentation says to "revoke items when possible." **What to do about currency that has already been spent** is a policy question (S8).
- Use `RefundTxn` when we issue a refund ourselves. The operations tool (`/admin` in document 08) sends the request to the Commerce service.

---

## 3. (B) Steam Items and the Community Market

### 3.1 What Is Possible (Based on Official Sources)

- Publish item types (ItemDef) to Steamworks. Each definition can enable `marketable` (can be sold on the Market) and `tradable` (can be traded between players).
- When a player sells on the Market, **the proceeds go to the seller's Steam Wallet. They cannot withdraw the proceeds as cash** (Market FAQ). They can use Wallet funds to buy games and other products on Steam. That is what "receiving money" means here.
- Fees (Market FAQ): The buyer pays a **5% Steam fee**. A **game-specific fee** is added. The game publisher sets and receives it. Valve games (TF2, Dota 2, CS2) charge 10%. **This game-specific fee is our source of revenue.** The screen for setting this game's fee is behind the partner site login, so we could not verify it in the public documentation.
- The developer decides whether to participate in the Market.
- Player restrictions (Market FAQ): Players must have made a purchase at least 7 days ago and must have had Steam Guard enabled for at least 15 days. The Wallet holds up to about $2,000, and a single listing can be up to about $1,800.

### 3.2 The Owner of Authoritative Data Changes

This is the central issue for this feature.

- Steam items live **on Steam servers**. Players can sell, trade, and receive them outside the game (Steam web, Market) at any time. Our server does not know when that happens.
- The rule that "our DB is authoritative" (document 03, AGENTS.md) therefore does not apply to Steam items. Our DB holds the **last verified copy**.

| Category | Source of truth | Our DB | Examples |
|---|---|---|---|
| Game items and currency | Our Game DB | Authoritative | Weapons, armor, gold, paid currency |
| Steam items | Steam Inventory | Copy (`steam_item_cache`) | Outfits, weapon skins, mount skins, emotes |

### 3.3 Ways to Obtain Steam Items

| Path | Method | Notes |
|---|---|---|
| Gameplay reward (server decides) | Commerce service calls `IInventoryService/AddItem` | With a `requestid`, **the same request runs only once** (official documentation). Use our idempotency key as is |
| Purchase | Steam Item Store (web) or in-game purchase with `ISteamInventory` | Put the price in ItemDef. Steam handles payment and grants in this case |
| Playtime drop | Client calls `TriggerItemDrop`; Steam controls probability and frequency | This method does not require trusting the client |
| Market, trades | Outside the game | We find out at the next sync |

- We can also sell Steam items through in-game purchases (section 2). In that case, `AddItem` takes the place of `GrantItem` in section 2.

### 3.4 Synchronizing the Copy

- **At login**, the Commerce service reads the full inventory with `GetInventory`, updates the copy, and passes it to the player actor.
- **During play**, read it again at regular intervals (for example, 5 minutes) and when the player uses an item (such as opening the equipment screen).
- If a player has equipped an item that disappears from the copy, unequip it at the next sync. This is enough for cosmetics.
- The server consumes items (for example, as upgrade materials) by calling `ConsumeItem` with a `requestid`. If Steam says the item is already gone, treat it as a failure. The order is **consume on Steam first → apply the game effect on success**.

### 3.5 Convert Game Items to Steam Items (Optional)

To let players "list items earned in the game on the Market," we need a way to **export** game items as Steam items.

```
1. Player actor: delete the game item + record "export to Steam" in the outbox (one transaction)
2. Commerce service: AddItem(requestid = that idempotency key)
3. Finish on success. Retry on failure (requestid prevents duplicate items)
```

In the other direction (Steam item → game item), call `ConsumeItem` and then grant the game item. Enabling this feature **ties the game economy directly to real-world value (Wallet funds)**. This makes it a target for farming operations, macros, and hacking, so the recommendation is to limit the scope as in S4.

### 3.6 Why We Recommend Cosmetics Only (S4)

- Steam items move outside our control. For items that affect combat power, problems like "a weapon bought on the Market does not appear until the next sync" or "a sold weapon stays usable until the next sync" become game balance problems immediately.
- If combat items earned by hunting can turn into Wallet funds, hunting itself becomes a way to make money and attracts farming operations.
- With cosmetics only, a delayed sync just means an outfit takes longer to change.

---

## 4. Data

### 4.1 Commerce DB (Draft)

| Table | Contents |
|---|---|
| `steam_orders` | `order_id` (u64, sequence), `player_id`, `steam_id`, product, amount (integer, minor units), currency, state, Steam `transid`, timestamps |
| `steam_order_items` | Products in the order (the Steam Item ID is our product number, 32-bit) |
| `steam_inventory_ops` | Steam Inventory operation records: type (AddItem/ConsumeItem), `requestid`, target, state |
| `steam_report_cursor` | How far we have read `GetReport` |
| `outbox` | `GrantItem` and `Revoke` to send to the home shard (document 03, section 3) |

### 4.2 Game DB Additions

- Grant idempotency key (already exists, document 03)
- `steam_item_cache`: per-player copy of Steam items and the last sync time

### 4.3 Money Representation

- Per Steam's rules, store amounts as **integers in minor units** (cents). Do not use floating point (guide section 4.4). Some currencies have no decimal places (see Supported Currencies in the official documentation).
- Use a newtype for amounts (`MinorUnits(i64)` + `Currency`).

---

## 5. Security

- Only the Commerce service holds the Steam publisher key. Follow document 08, section 8 for storage (keep secrets outside the repository). If the key leaks, anyone can create items.
- The server decides all grants and revocations. Use product numbers, prices, and result callbacks from the client only as signals.
- Payment requests use the Reliable class from document 02 and an idempotency key.
- Record payment and grant history as transaction logs in Game DB (document 04, section 1.1). Use them for customer support queries and refund handling.

---

## 6. Implementation Order (Later)

1. Steam login (ticket verification), alongside the update to document 07
2. In-game purchases, end to end in the sandbox (create → approve → finalize → grant)
3. `GetReport` polling and revocation
4. Steam items: a few ItemDefs, sync at login, server grants (`AddItem`)
5. Open the Market (Steamworks economy settings and game-specific fee settings)
6. (Optional) Export game items

---

## 7. Remaining Decisions

| # | Question | Notes |
|---|---|---|
| S7 | Login methods | **Final:** Keep all methods open and implement Steam first (document 07, L2) |
| S8 | Refunded currency was already spent | **Final:** Let the balance go negative and restrict access to the paid shop until the balance is made up. State this in the terms of service |
| S9 | Commerce DB location | **Final:** Start with a separate schema in Account DB. Split it out if load grows |
| S10 | Scope of Steam items | **Final:** Cosmetics only (3.6) |
| S11 | Export game items (3.5) | **Final:** Do not enable at initial launch |
| S12 | Game-specific Market fee rate | **Final:** Around 10%. Set the exact value on the partner site |
| S13 | HTTP client crate (for example, `reqwest`) | A new external crate requires owner approval and an update to the table in guide section 13 |
