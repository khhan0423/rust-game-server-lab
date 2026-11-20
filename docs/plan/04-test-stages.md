# Test Stages: From Development to Launch

**English** | [한국어](04-test-stages.ko.md)

- Status: **draft for discussion** (2026-11-20). Decided on 2026-11-20: guest login is used through the alpha and never in the released game; Steam registration is put off until the beta is near; the friends test server runs on AWS.
- Related: [execution plan](README.md), [pilot](01-pilot.md), `docs/design/07-login-account.md`, `docs/research/guest-account-recovery.md`

This note walks through how the game gets from the developer's PC to paying players, one stage at a time, and what the server has to provide for each stage.

## The stages

| Stage | Who plays | How they get in | Login | Player data | Payments |
|---|---|---|---|---|---|
| Development | The developer | Local PC | Guest | Wiped at any time | None |
| Friends test | Invited friends, a few to a few dozen | A client build sent directly, an invite code, and the test server's address | Guest | Wiped when the test ends | None |
| Alpha | Invited players, tens to hundreds | A client build sent directly, an invite code, and the test server's address | Guest | Wiped when the alpha ends | None |
| Beta | A larger group that signed up | Steam Playtest sign-up, opened in waves | Steam | Wiped before launch | None with real money; the payment flow is tested in Steam's sandbox |
| Launch | Anyone who owns the game | The Steam store, as Early Access or a full release | Steam | Kept | Real |

What Steam's documentation says about Steam Playtest, checked on 2026-11-20 ([source](https://partner.steamgames.com/doc/features/playtest)): it is a separate app linked to the main game, it has the same Steamworks technical features as the main game, the developer controls who gets in and can switch it off at any time, it is free, and it must not be monetized with in-game transactions or sold. Charging for a work in progress means Steam Early Access instead. A playtest build is started through Steam, so Steam login works in it.

Steam login has a cost and a lead time. A playtest app is created from the main game's app, so the game must first be registered with Steam, which costs 100 US dollars per app. The fee is not refundable, but it is paid back once the game has earned 1,000 US dollars ([source](https://partner.steamgames.com/doc/gettingstarted/appfee)). The server verifies a player's Steam ticket with a Web API key tied to that app ([source](https://partner.steamgames.com/doc/features/auth)), so there is no way to test real Steam login without the registration. **Decided (2026-11-20):** the fee is put off. This project is an experiment, and the alpha is invite-only, so the alpha runs like a larger friends test with guest login and invite codes. The game is registered with Steam once the decision to release it is made, and Steam login starts at the beta. The price of waiting is that problems in the Steam integration show up late, so the schedule leaves time between registration and the beta to test it.

## What each stage is for

**Friends test.** The first time the servers run across the internet with real people. The goal is to find out whether login, moving around together, and reconnecting hold up outside one PC, and to hear what feels wrong. It is small and informal. Accounts are guest accounts and will not be kept.

**Alpha.** The same setup as the friends test with more people and more of the game. The goal is that the whole path works with strangers on it: nickname and player tag, friends, parties across shards. Content is thin. Data is thrown away afterwards, and testers are told so before they start.

**Beta.** The first test through Steam, and the load and operations rehearsal. Steam login replaces guest login here. The goal is to run at something near the expected launch population, with the operations tool, monitoring, maintenance, and a planned server kill during play. The payment flow is exercised in Steam's sandbox, which moves no real money.

**Launch.** Data is kept from here on. If the game starts as Early Access, that is the launch as far as the server is concerned.

## What the server needs for this

- **An environment name in the configuration** (development, friends test, alpha, beta, live). Each environment has its own databases, keys, and addresses. Login tokens and tickets carry the environment name, so one issued in a test is refused by live.
- **A switch for the guest method**, on in development, the friends test, and the alpha. A server configured as beta or live refuses to start with the guest method on.
- **Invite codes for the friends test and the alpha.** Creating a guest account requires a code that the developer handed out. A code works a set number of times. Without this, anyone who learns the server address can make accounts.
- **A limit on account creation**, so a mistake or an attack cannot fill the database with accounts.
- **A version check at entry**, so a tester with an old build gets a clear "please update" instead of odd errors.
- **A wipe procedure** that is written down and rehearsed: stop entries, back up, drop the stage's data, start clean. Nicknames and player tags are released by the wipe.
- **Tester rewards that survive the wipe.** If beta testers are promised something at launch, the promise is recorded by Steam ID outside the wiped data and granted once at launch through the normal grant path. Friends test and alpha accounts are guests with no Steam identity, so nothing is promised to carry over from them.
- **A way for testers to report problems**, with the player tag as the reference they quote. Server logs can then be searched by that player.
- **Somewhere to run the friends test server.** Either the developer's PC with a port opened, or a small rented Linux server. The servers already build and pass their tests on Linux.

## Risks worth naming

- A friend loses the saved guest file and with it the test account. This is accepted for the friends test, and testers are told to keep the `client-data` folder.
- Test data leaking into live, or a test build talking to live servers. The environment name in tokens and separate keys per environment are there to stop this.
- Promising testers that progress will carry over. The plan is the opposite: every stage before launch ends in a wipe, and only named rewards carry over.

## Decisions needed

| Question | Recommendation |
|---|---|
| How testers get into the friends test and the alpha | **Decided (2026-11-20):** invite codes, handed out by the developer |
| Where the friends test server runs | **Decided (2026-11-20):** a Linux server on AWS, set up by the owner when the test is near. Until then everything runs on the development PC |
| Login in each stage | **Decided (2026-11-20):** guest with invite codes through the alpha; Steam only from the beta on |
| Whether data carries over between stages | No. Wipe at the end of every stage before launch; carry over only named rewards |
| When Steam login is built | **Decided (2026-11-20):** after the game is registered with Steam, before the beta. Registration waits for the decision to release |
