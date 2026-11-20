# Guest account recovery, transfer, and rerolling

**English** | [한국어](guest-account-recovery.ko.md)

Status: Research, 2026-11-20. Not binding.

## Summary

A public friend code can locate a profile. Recovery requires proof of control, such as a linked login, a private transfer credential issued while the player still had access, or evidence support can independently verify. Transfer, account deletion, and starting over are separate actions.

For this Steam-first game, make Steam linking the way to keep guest progress. Treat unlinked guests as temporary, give every new player one starter choice from a fixed pool, and skip full account reset at launch. This removes the main reason to create disposable accounts while keeping the recovery rules simple.

## How games handle recovery and transfer

**Transfer number plus password.** Fate/Grand Order (US) issues a Transfer Number from My Room after the player sets a password. The player enters both on the destination device. The pair stays valid until used, can be used once, and is superseded when a new pair is issued. This is advance preparation for a device move or loss, and a player who has lost both credentials needs a different path. [1][2]

**ID plus password.** One Piece Treasure Cruise documents a legacy flow that accepts a User ID and a previously issued transfer password; its current FAQ says new passwords can no longer be issued. Another Eden lets the player set their own password in Data Transfer Settings and pair it with a dedicated Transfer ID. The community wiki describes that ID as reusable across transfers and lists Steam among the supported clients. These examples distinguish the publicly visible ID plus secret pattern from a dedicated transfer ID plus user-set secret; a current flow that uses a public friend UID and a user-set password together was not confirmed. [3][4][5]

**Linked login and unlinked guest.** The Starseed: Asnia Trigger test-environment guide lists Google, Apple and Hive linking from a guest profile and says linked progress can be recovered after changing devices or reinstalling. It does not promise recovery for an unlinked install or document a hardware tie. NIKKE's email login was moved to LEVEL INFINITE PASS per a reproduced publisher notice. Epic Seven's Steam FAQ describes transferring an existing email or social login to a Steam login. These mechanisms restore access by authenticating a linked identity. [6][7][8]

**Device-bound guest.** Skullgirls Mobile support says guest progress is stored only on the device and cannot be recovered after uninstalling, switching devices, or clearing app data. It offers Google, Facebook, LINE, or email linking for persistent access. This describes what the player can recover, without establishing a hardware fingerprint or how other games implement guest storage. [35]

**Support investigation.** Fate/Grand Order (US) investigates loss reports and may issue a transfer number after verification; it keeps the detailed decision criteria private. The Japanese inquiry form lists the game data ID, last device model and OS version, and purchase details such as order number, quantity and date. HoYoverse offers a recovery application with a case number and a response to a verified email. A player guide for Genshin Impact reports being asked for the approximate first login date; current mandatory form fields and scoring were not verified. A receipt is useful corroboration when checked against server records. A public UID, device model, or remembered date is weak on its own. [9][10][11][12]

## Reset and reroll support

**Clear the guest and start again.** Mobile Limbus Company guides describe starting as a guest, skipping the tutorial, and using Close Account in settings before another attempt. Prydwen says the Steam client lacks that guest option. The Starseed test guide describes immediate deletion for an unlinked guest from the account menu. Epic Seven's account FAQ lists reset separately from deletion. These examples make repetition easier, but the cited sources do not establish that the deletion controls were built for rerolling. [6][13][14][15]

**Create another identity.** Player guides for NIKKE describe registering separate accounts with Gmail plus aliases that deliver to one inbox. Separate accounts or profiles on another server are other ways to get a fresh start where the game supports them. These are player practices; continued acceptance, policy approval, and alias handling should not be inferred from a guide. [16][27]

**Change the opening reward.** Brown Dust 2's Infinite Draw lets players repeat the tutorial draw as often as they want and accept one result. Epic Seven's Selective Summon offers repeat draws from a defined pool and one result to keep; the cited guide describes a cap, so treat it as a bounded example rather than assuming unlimited or identical rules on every version. Wuthering Waves added a free choice of one standard five-star character as a time-limited launch giveback. Its announcement cited feedback about the Beginner's Choice Convene. For the first two games, reducing the incentive to reset is an observable design effect; the publisher's original reason was not confirmed. [17][18][19]

## Security and abuse

Any transferable secret can be stolen through phishing, a screenshot, or access to local files. Low entropy or an unlimited redemption endpoint also makes guessing a risk. A public player tag does not add secret entropy. If a transfer feature becomes necessary, require recent guest authentication to issue a cryptographically random, short-lived code. Keep only a verifier, redeem once atomically, invalidate older codes and old guest sessions, and rate limit issuance and redemption. Keep failures generic and keep secrets out of logs. Fate/Grand Order's until-used policy is one product choice; OWASP recommends single use, expiration and brute-force controls for reset tokens. [1][2][20]

Starter accounts have resale value: a public Epic Seven trading forum advertises rerolled accounts, and Epic Seven has published bans for abnormal account creation. These sources establish listings and enforcement, not sales volume, which accounts sold, or private detection thresholds. Multi-instance emulator play is described in Limbus Company guides. Parallel human play and bot farms are different; use behavior and creation volume as signals rather than treating emulator use alone as proof. OWASP classifies bulk automated registration as an account creation threat. [13][21][22][23]

A support agent can be pressured to accept public profile details, copied receipts, or a plausible story as ownership proof. A seller or former owner may also have old evidence. Compare evidence privately against original records, prefer a previously verified channel, notify it of any approved recovery when possible, and audit overrides. Do not tell claimants which guesses matched or let a rushed ticket bypass policy. [9][20]

## Cost on the server side

Every abandoned guest can leave an identity, starter state, inventory, logs, indexes, and a reserved nickname. Asynchronous cleanup costs database work and can race with login or linking. A useful first estimate is daily abandoned creations multiplied by the retention period and all-in bytes per abandoned account; separately measure write load, backup growth, the purge backlog, and the share of names held by inactive guests. A small record can still hold a desirable name indefinitely.

There are published precedents outside gacha. Super Penguin World says it removes unlinked guest accounts after one year of inactivity. KGS documents deleting ordinary guest accounts on logout and freeing registered names after account expiry. These show different lifecycle choices. Internal per-account costs and automated expiry periods for the gacha examples were not verified. [24][25]

## What changes on Steam

Steamworks supplies a unique SteamID and a session ticket flow that lets the backend verify it. The publisher decides what game record the verified ID maps to, and whether that record can reset or have slots. With one persistent game account per SteamID, deleting local files or reinstalling leaves the account mapping intact. [26]

Konami documents one Yu-Gi-Oh! Master Duel profile per Steam account, no delete-and-restart for the same platform account, and cross-platform transfer via KONAMI ID. A reported player workaround is starting under a different Steam account; that creates a different identity rather than resetting the original. Limbus Company guides instead point Steam players who want a quick guest reroll to its mobile client. [13][28][29][30][31]

Epic Seven documents conversion of a publisher login to Steam. Its Steam community guide describes an initial temporary account tied to Steam and no in-client reroll reset; the account conversion and withdrawal guides describe how to clear a conflicting temporary profile for migration. Repeatable fresh starts on the same Steam login after withdrawal were not verified. Tower of Fantasy has a Steam release, and a general player guide describes new progress on another server while keeping the original there. That is a per-server save pattern, not multiple Steam character slots on one server; the current behavior on the Steam build was not separately verified. [8][27][32][33][34]

## Options and recommendation for this project

**Recommended: Steam linking and one starter choice.** Once Steam login is available, make it the default and use the verified SteamID as a unique login mapping to the existing account. Let guests link from an authenticated guest session after verifying Steam in the same flow. Keep character progress, nickname and public tag on the successful link. If the SteamID already has a profile, clearly offer to sign into that profile or keep the guest as is. Never silently overwrite or merge, and after linking revoke the guest credential so it is no longer an alternate way in. Offer linking early and before purchases or any durable allocation. Keep development guests separate from public production accounts.

Keep the random nine-character player tag for friend lookup and support case routing. Keep internal keys off the client. The current guest ID and secret are local credentials; without the file or a verified link, support cannot safely recreate access from a tag. Explain this when the guest starts and again before meaningful progress. Until linking ships, treat public guest progress, if offered at all, as disposable and keep purchases off it.

**Defer guest transfer codes.** A short-lived code can move an authenticated guest to another device before Steam linking exists, but it adds a second access path and helps make guest progress transferable to buyers. Issue one only if a real migration requirement cannot be met by platform linking. An advance backup code would cover device loss, but would have to remain usable long enough to survive it and would carry more theft risk. Do not offer a permanent code or a password keyed only by the public tag at launch.

**Do not ship full account reset as a reroll feature.** Keep a separate account deletion policy and flow. If a restart is later needed, require fresh authentication and explicit confirmation. Reset the eligible character progression under the existing account; preserve the same Steam link, nickname, friend tag, enforcement and purchase records, and one-time grant claims. Never mint another character or tag or renew signup rewards. Release the nickname only on finalized deletion or a normal rename, and retire the old tag on deletion. Keeping identity stable avoids breaking friend references and recycling names through reset.

**Remove the incentive to reroll.** Offer a once-per-account choice of one starter from a small, balanced pool. If surprise is central to onboarding, offer bounded redraws from that pool instead, with one server-side commit of the final choice. Keep the claim across relogin, linking, and any later restart and exclude tradable currency and limited-banner rewards. Neither option removes the incentive if other unusually valuable random rewards are front-loaded, so inspect the whole opening package.

**Expire abandoned guests and release names.** Do not reserve a global nickname for a guest until linking or a meaningful onboarding milestone; use a temporary display name or no public profile until then. Start with a published 30-day inactivity window for unlinked guests with no purchases, unresolved support or fraud hold, or state requiring retention. Treat that window as a proposal. Run a bounded purge in batches, recheck inactivity and link status at deletion, and make purge and linking mutually exclusive. Release the nickname only when deletion is final and retire the public tag instead of assigning it to someone else. Retain only the minimal audit or anti-abuse records the policy requires for a defined period.

**Limit creation and scope support.** Start with burst and sustained creation budgets by network, by verified SteamID where available, and by an installation signal if it is available; enforce one account per SteamID and cap signup globally under load. Let existing users log in when creation is throttled. Treat shared networks and installations as signals, not ownership or automatic grounds for a ban; watch creation rates, starter claims, and false blocks before tightening. Support may locate an account by tag, explain how to regain access through Steam, and investigate a verified linking dispute privately. Do not reveal internal IDs or linked account details, issue a replacement guest secret for a tag or receipt alone, transfer a profile on request, or restore a purged guest. For a provable linking error, require independent evidence of control, a second reviewer, and an audit trail before any exceptional change.

The main trade-off is that an unlinked fallback guest can lose progress permanently. Early linking and a starter choice avoid both a standing recovery secret and a recurring reason to discard accounts.

## Open questions

- Will the public be able to keep guest progress before Steam login ships, or will those builds explicitly reset it?
- What pool and reward schedule make starter choice sufficient, and what onboarding events should consume the one-time grant or allow nickname reservation?
- What deletion notice, grace period, retention exceptions, and final name release timing are appropriate? Measure actual abandoned-account volume before settling the window.
- What creation limits fit launch traffic and shared networks, and who may approve a verified linking correction?

## Sources

[1]: https://faq.fate-go.us/faq/show/32?site_domain=default
[2]: https://faq.fate-go.us/faq/show/6?site_domain=default
[3]: https://bnfaq.channel.or.jp/faq/detail/8638
[4]: https://wfs-another-eden-global.zendesk.com/hc/en-us/articles/7584248110607-How-do-I-set-up-a-Transfer-ID-for-a-data-transfer
[5]: https://www.anothereden.wiki/w/Transfer_ID
[6]: https://promotion.withhive.com/custom/board_detail/1140?language=en
[7]: https://nikke.gg/account-system-upgrade-notification/
[8]: https://page.onstove.com/epicseven/GLOBAL/view/14378309
[9]: https://faq.fate-go.us/faq/show/11?site_domain=default
[10]: https://faq.fate-go.jp/helpdesk?category_id=9&faq_id=&site_domain=default
[11]: https://support.hoyoverse.com/hc/en-us/articles/48105986812313-How-can-I-retrieve-my-account-if-I-ve-lost-access-to-it
[12]: https://www.reddit.com/r/GenshinHacked/comments/xw4d5i/a_guide_to_recovery_form/
[13]: https://www.prydwen.gg/limbus-company/guides/reroll
[14]: https://www.destructoid.com/how-to-reroll-in-limbus-company/
[15]: https://page.onstove.com/epicseven/GLOBAL/view/9979150
[16]: https://www.prydwen.gg/nikke/guides/reroll
[17]: https://progameguides.com/brown-dust-2/brown-dust-2-tier-list-reroll-guide/
[18]: https://epicsseven.wiki/guides/epic-seven-selective-summon
[19]: https://wutheringwaves.gg/regarding-gameplay-issues-and-our-future-improvement-plans/
[20]: https://cheatsheetseries.owasp.org/cheatsheets/Forgot_Password_Cheat_Sheet.html
[21]: https://www.epicnpc.com/forums/epic-seven-epic7-accounts.1670/
[22]: https://page.onstove.com/epicseven/GLOBAL/view/10804526
[23]: https://owasp.org/www-project-automated-threats-to-web-applications/assets/oats/EN/OAT-019_Account_Creation.html
[24]: https://mijogames.se/delete-your-data-in-super-penguin-world/
[25]: https://www.gokgs.com/help/Accounts.html?helpLocale=en_US
[26]: https://partner.steamgames.com/doc/features/auth
[27]: https://progameguides.com/tower-of-fantasy/do-characters-transfer-across-servers-in-tower-of-fantasy/
[28]: https://ja-support1.konami.com/hc/ja/articles/48547980459289
[29]: https://ja-support1.konami.com/hc/ja/articles/4415751050137
[30]: https://us-support.konami.com/hc/en-us/articles/4813085461655-Can-I-transfer-my-data-to-other-platforms
[31]: https://www.businessinsider.com/reference/master-duel-reset-account-steam?op=1
[32]: https://epicsseven.wiki/guides/epic-seven-steam-reroll
[33]: https://epic7tools.dev/guides/stove-account-to-steam/
[34]: https://store.steampowered.com/app/2064650/Tower_of_Fantasy/
[35]: https://autumngames.helpshift.com/hc/en/4-skullgirls/faq/37-what-is-the-difference-between-a-guest-account-and-a-linked-account/

- Transfer and recovery: [Fate/Grand Order issuance][1], [redemption][2], [One Piece Treasure Cruise FAQ][3], [Another Eden FAQ][4] and [wiki][5], [Starseed test guide][6], [NIKKE notice reproduction][7], [Epic Seven Steam FAQ][8], [Fate/Grand Order US recovery][9] and [JP form][10], [HoYoverse support][11] and [player report][12], [Skullgirls Mobile support][35].
- Reroll and starter rewards: [Prydwen on Limbus Company][13], [Destructoid][14], [Epic Seven account FAQ][15], [Prydwen on NIKKE][16], [Brown Dust 2 guide][17], [Epic Seven guide][18], [Wuthering Waves notice reproduction][19].
- Security and retention: [OWASP on reset tokens][20], [Epic Seven trade listings][21] and [ban notice][22], [OWASP on bulk creation][23], [Super Penguin World policy][24], [KGS policy][25].
- Steam behavior: [Steamworks][26], [Tower of Fantasy server guide][27] and [store listing][34], [Konami Steam FAQ][28], [profile FAQ][29] and [transfer FAQ][30], [Business Insider on separate Steam accounts][31], [Epic Seven Steam reroll guide][32] and [conversion guide][33].
