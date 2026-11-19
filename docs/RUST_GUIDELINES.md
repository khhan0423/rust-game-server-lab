# Rust 코딩 및 설계 가이드

이 프로젝트의 코드와 설계는 이 문서를 기준으로 맞춘다. 사람이든 AI 에이전트든 같은 규칙을 따른다.

일관성은 세 겹으로 지킨다. 위에 있는 것이 우선한다.

1. **도구가 강제하는 규칙**: `rustfmt`, `clippy`, 컴파일러 lint. 통과하지 못하면 코드가 들어가지 않는다.
2. **이 문서의 규칙**: 도구로 잡을 수 없는 설계 규칙이다.
3. **리뷰**: 문서의 마지막 체크리스트로 확인한다.

Rust를 잘 몰라도 1번이 대부분을 잡아 준다. 규칙이 헷갈리면 "도구가 통과시키는가"를 먼저 보고, 그다음 이 문서를 본다.

---

## 0. 품질 게이트 (항상 통과해야 함)

커밋하기 전에 워크스페이스 루트에서 아래 세 명령이 모두 성공해야 한다.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- 경고는 에러로 취급한다(`-D warnings`). "경고는 나중에 고치자"는 허용하지 않는다.
- lint를 꺼야 할 때는 `#[allow(...)]`를 쓰지 않는다. 대신 `#[expect(lint_name, reason = "이유")]`를 쓴다. 나중에 경고가 사라지면 `expect` 자체가 경고를 내기 때문에 죽은 예외가 남지 않는다.
- 설정 파일은 루트에 있는 `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`과 `Cargo.toml`의 `[workspace.lints]`다. 이 파일들은 합의 없이 고치지 않는다.

---

## 1. 툴체인과 환경

- 컴파일러 버전은 `rust-toolchain.toml`이 고정한다. 개인 PC마다 버전이 달라서 생기는 차이를 없애기 위해서다. 처음에는 `channel = "stable"`로 두고, 첫 빌드가 통과하면 그때의 버전 번호(예: `"1.xx.0"`)로 바꿔 고정한다.
- `clippy.toml`은 lint 동작만 조정한다(테스트에서 `unwrap` 등 허용). lint를 켜고 끄는 것은 `Cargo.toml`의 `[workspace.lints]`에서 한다.
- Edition은 `2024`로 한다.
- **IDE를 쓰지 않는다.** 이 프로젝트는 CLI만으로 개발한다(실험 목적). 기본 흐름:
  - `cargo check`: 빠른 타입 검사. 가장 자주 쓴다
  - `cargo clippy`, `cargo fmt`, `cargo test`: 0장의 품질 게이트
  - `bacon clippy`: 터미널에 띄워 두면 저장할 때마다 자동 검사 (IDE의 빨간 밑줄 역할)
  - `cargo doc --open`: 프로젝트와 의존성 문서 열람
  - `rustc --explain E0xxx`: 컴파일 에러 코드의 상세 설명
- 개발 중 실행과 테스트는 WSL2(Linux)에서 하고, 운영 대상인 Linux 동작을 그대로 확인한다.
- Windows에서는 MSVC 툴체인(`x86_64-pc-windows-msvc`)을 쓴다.

---

## 2. 워크스페이스 구조

크레이트 하나에 모든 것을 넣지 않는다. 역할별로 크레이트를 나누고 의존 방향을 한쪽으로 고정한다.

```
Server/
├─ Cargo.toml            # [workspace], [workspace.dependencies], [workspace.lints]
├─ crates/
│  ├─ protocol/          # 패킷 정의, 인코딩/디코딩. I/O 없음, async 없음
│  ├─ game/              # 게임 규칙, 상태. I/O 없음, DB 없음 (순수 로직)
│  ├─ db/                # DB 접근 전담 (sqlx). 쿼리는 여기에만 존재
│  ├─ net/               # 프레이밍 코덱, 링크 송수신 등 공유 네트워크 코드
│  ├─ infra/             # 공용 운영 코드: 설정 로드, 로그 초기화, 지표, 관리 포트. 다른 크레이트에 의존하지 않음
│  ├─ gateway/           # Gateway 바이너리 (설계 02)
│  ├─ server/            # 바이너리. Game 서버, 전투 서버, 클러스터 서비스를 역할(role)로 띄움. tokio, 조립(wiring)
│  ├─ bot/               # 창 없는 테스트/부하용 봇 바이너리. 세션 계약의 참조 구현
│  ├─ client/            # GUI 테스트 클라이언트 (Bevy + bevy_egui, 설계 10)
│  └─ launcher/          # GUI 서버 런처 (eframe, 설계 10). 게임 크레이트에 의존하지 않음
└─ migrations/           # DB 마이그레이션 SQL
```

**의존 방향 규칙**

```
server ──> game ──> protocol
   │                   ^
   ├──> db             │
   └──> net ────────────┤
gateway ──> net         │
bot ───────> net ───────┘
client ────> net        (protocol은 net을 통해)
launcher                (게임 크레이트에 의존하지 않음)
바이너리 크레이트(server, gateway, bot, client, launcher) ──> infra   (infra는 아무것에도 의존하지 않음)
```

- `protocol`은 어떤 크레이트에도 의존하지 않고, `game`은 `protocol`에만 의존한다. 둘 다 I/O가 없는 순수 로직이라 네트워크나 DB 없이도 테스트할 수 있다.
- `gateway`는 `game`을 모른다. Gateway는 게임 패킷의 페이로드를 해석하지 않기 때문이다(설계 02 3.2).
- `game`은 `db`를 모른다. 영속화가 필요하면 `server`가 `game`의 결과를 받아 `db`에 넘긴다.
- 의존성 순환은 컴파일러가 막는다. 하지만 "일단 server에 다 넣자"는 컴파일러가 막지 못하니 리뷰에서 막는다.
- 외부 크레이트 버전은 루트 `[workspace.dependencies]`에만 적는다. 각 크레이트에서는 `tokio = { workspace = true }`처럼 참조만 한다.

C++에 비유하면 `protocol`과 `game`은 헤더 의존이 없는 static lib이고, `server`는 이것들을 링크하는 exe다.

---

## 3. 이름 규칙

Rust 표준 관례(RFC 430)를 그대로 따른다. 컴파일러와 clippy가 대부분 경고해 준다.

| 대상 | 규칙 | 예 |
|---|---|---|
| 크레이트, 모듈, 함수, 변수 | `snake_case` | `fn move_player`, `mod zone` |
| 타입, 트레이트, enum variant | `UpperCamelCase` | `struct PlayerId`, `enum Packet { MoveReq }` |
| 상수, static | `SCREAMING_SNAKE_CASE` | `const MAX_PACKET_SIZE: usize` |
| 생성자 | `new`, 변형은 `with_*`, `from_*` | `Zone::new(id)`, `Config::from_file(path)` |
| getter | `get_` 접두사 없이 필드 이름 그대로 | `player.level()` |
| 변환 | `as_*`(비용 없음, 참조), `to_*`(복사), `into_*`(소유권 소비) | `id.as_u64()`, `name.to_owned()`, `buf.into_bytes()` |
| bool 반환 | `is_*`, `has_*`, `can_*` | `is_alive()` |

- 약어도 단어로 취급한다: `HttpServer`, `DbPool` (`HTTPServer`, `DBPool` 아님).
- 헝가리안 표기(`m_`, `p`, `sz`, `n`)는 쓰지 않는다. 타입은 컴파일러가 알려 준다.

---

## 4. 타입 설계

### 4.1 원시 타입을 그대로 돌리지 않는다 (newtype)

ID, 수량, 금액은 전용 타입으로 감싼다. C++의 strong typedef와 같은 개념인데, Rust에서는 비용이 0이고 쓰기도 쉽다.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlayerId(Uuid);   // 데이터 ID는 UUID v7 (설계 03 7장)

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ItemId(Uuid);

// fn give_item(player: Uuid, item: Uuid)    // X: 순서를 바꿔도 컴파일됨
fn give_item(player: PlayerId, item: ItemId) // O: 순서가 틀리면 컴파일 에러
```

### 4.2 상태는 enum으로 표현한다

bool 플래그 여러 개를 조합하지 말고, 가능한 상태를 enum으로 나열한다. 이렇게 하면 "있을 수 없는 상태"를 아예 만들 수 없다.

```rust
// X
struct Session { logged_in: bool, in_zone: bool, zone_id: Option<ZoneId> }

// O
enum SessionState {
    Connected,
    Authenticated { account: AccountId },
    InZone { account: AccountId, zone: ZoneId },
}
```

### 4.3 match는 빠짐없이

- 프로젝트에서 직접 만든 enum을 `match`할 때는 `_ =>`를 쓰지 않는다. 모든 variant를 적어 두어야 variant가 추가됐을 때 컴파일러가 처리하지 않은 곳을 모두 알려 준다. 이것이 C++ `switch`와 가장 큰 차이이고, 가장 큰 장점이다.
- 외부 크레이트의 enum이나 숫자처럼 끝이 없는 값에만 `_`를 허용한다.

### 4.4 돈, 재화, 수치

- 금액, 재화, 확률 계산에 `f32`나 `f64`를 쓰지 않는다. 정수 최소 단위(`i64`)나 `rust_decimal`을 쓴다.
- 재화와 스탯 연산은 `checked_add`, `checked_sub`를 쓰고, 오버플로가 나면 에러로 처리한다. Rust 정수 오버플로는 debug 빌드에서는 panic이 나지만 release 빌드에서는 조용히 wrap되므로 둘 다 믿으면 안 된다.

```rust
let new_gold = player.gold.checked_add(reward).ok_or(GameError::GoldOverflow)?;
```

### 4.5 null 대신 Option, 예외 대신 Result

- "없을 수 있음"은 `Option<T>`로 표현한다. 매직 값(`-1`, `0`, `INVALID_ID`)은 쓰지 않는다.
- "실패할 수 있음"은 `Result<T, E>`로 표현한다. 자세한 내용은 5장에 있다.

---

## 5. 에러 처리

| 위치 | 방식 |
|---|---|
| 라이브러리 크레이트 (`protocol`, `game`, `db`) | `thiserror`로 크레이트별 에러 enum을 정의한다 |
| 바이너리 (`server`, `bot`)의 최상위, 시작 코드 | `anyhow::Result`로 문맥(`.context("...")`)을 붙여 위로 올린다 |

```rust
// crates/game/src/error.rs
#[derive(Debug, thiserror::Error)]
pub enum GameError {
    #[error("player {0:?} not found")]
    PlayerNotFound(PlayerId),
    #[error("not enough gold: need {need}, have {have}")]
    NotEnoughGold { need: i64, have: i64 },
    #[error("gold overflow")]
    GoldOverflow,
}
```

규칙:

- `unwrap()`과 `expect()`는 테스트 코드 밖에서 쓰지 않는다(clippy가 막음). 프로세스 시작 시 설정 로드처럼 정말 죽어야 하는 곳에만 `#[expect(clippy::expect_used, reason = "...")]`를 붙여서 쓴다.
- `panic!`, `todo!`, `unimplemented!`는 커밋하지 않는다.
- 에러를 버리지 않는다. `let _ = send(...)`처럼 결과를 무시할 때는 왜 무시해도 되는지 주석을 단다.
- 에러는 `?`로 전파한다. 같은 에러를 로그로 찍고 다시 올리는 이중 처리는 하지 않는다. 로그는 처리하는 쪽에서 한 번만 찍는다.
- 클라이언트 패킷 하나가 잘못됐다고 서버가 죽으면 안 된다. 그런 경우 세션만 끊는다.

C++ 감각으로 보면 예외는 없고, 모든 실패가 반환 타입에 드러난다. `?`는 "실패하면 바로 return"을 한 글자로 쓴 것이다.

---

## 6. 소유권, 동시성, async

이 장이 Rust 학습에서 가장 어렵고, 가장 중요하다.

### 6.1 기본 원칙: 상태는 한 task가 소유한다 (액터 모델)

- 게임 상태(Zone, 플레이어 목록)는 그 Zone을 담당하는 **task 하나가 단독으로 소유**한다.
- 다른 곳에서는 상태를 직접 건드리지 않고, `tokio::sync::mpsc` 채널로 메시지를 보낸다.
- 그러면 게임 로직 안에서는 락이 필요 없다. C++ MMO에서 Zone마다 Job 큐를 두고 단일 스레드로 처리하던 구조와 같다.

```rust
enum ZoneCommand {
    Enter { player: PlayerId, reply: oneshot::Sender<Result<(), GameError>> },
    Move { player: PlayerId, to: Position },
    Leave { player: PlayerId },
}

async fn run_zone(mut zone: Zone, mut rx: mpsc::Receiver<ZoneCommand>) {
    while let Some(cmd) = rx.recv().await {
        zone.handle(cmd); // &mut zone: 이 task만 접근하므로 락 없음
    }
}
```

### 6.2 공유가 꼭 필요할 때

| 상황 | 도구 |
|---|---|
| 읽기 전용 공유 (설정, 기획 데이터 테이블) | `Arc<T>` (불변이라 락 불필요) |
| 드물게 쓰고 자주 읽음 | `Arc<RwLock<T>>` |
| 짧은 임계 구역, `.await`를 걸치지 않음 | `Arc<std::sync::Mutex<T>>` |
| 임계 구역 안에서 `.await`를 해야 함 | 설계를 다시 본다. 대개 액터로 바꾸는 게 맞다 |

- `std::sync::Mutex` 가드를 쥔 채로 `.await`하지 않는다(clippy `await_holding_lock`).
- `Rc`와 `RefCell`은 async 서버 코드에서 쓰지 않는다. `Send`가 아니라서 task 사이를 넘어갈 수 없다.

### 6.3 async 규칙

- async 함수 안에서 블로킹 호출(`std::thread::sleep`, 동기 파일 I/O, 무거운 CPU 연산)을 하지 않는다. 꼭 필요하면 `tokio::task::spawn_blocking`으로 보낸다.
- 채널은 **bounded**(`mpsc::channel(N)`)를 기본으로 한다. unbounded 채널은 느린 클라이언트 하나가 메모리를 무한히 먹게 만든다. C++에서 송신 큐 상한을 두던 것과 같은 이유다.
- `tokio::spawn`한 task의 `JoinHandle`을 그냥 버리지 않는다. 종료와 에러를 누가 처리할지 정해 둔다.
- 서버 종료는 `CancellationToken`(tokio-util)이나 `shutdown` 채널로 전파해서 graceful하게 처리한다.

### 6.4 borrow checker와 싸우지 않는 법

- 처음에는 구조체에 참조(`&'a T`)를 저장하지 않는다. 구조체는 데이터를 소유하게 한다. 라이프타임 파라미터가 있는 구조체는 익숙해진 뒤에 쓴다.
- 컴파일 에러를 없애려고 `.clone()`을 덕지덕지 붙이지 않는다. 그런 에러는 대개 "누가 소유하는가"를 다시 정하라는 신호다. 다만 `Arc::clone`이나 작은 `Copy` 타입은 마음껏 복사해도 된다.
- 객체가 서로를 가리키는 그래프 구조(플레이어 ↔ 파티 ↔ 길드)는 포인터로 연결하지 않는다. ID(`PartyId`)로 참조하고 `HashMap<PartyId, Party>`에서 찾는다. C++의 `shared_ptr`/`weak_ptr` 순환 문제가 원천적으로 사라진다.

---

## 7. 네트워크와 프로토콜

- 모든 패킷은 `protocol` 크레이트에 정의한다. 방향별로 enum을 하나씩 둔다(`ClientPacket`, `ServerPacket`).
- 프레이밍은 길이 헤더 방식으로 하고, `tokio_util::codec`의 `Decoder`와 `Encoder`로 구현한다.
- 최대 패킷 크기 상수(`MAX_PACKET_SIZE`)를 두고, 이를 넘는 길이 헤더가 오면 즉시 세션을 끊는다.
- **경계에서 검증하고, 안쪽에서는 믿는다.** 바이트를 디코딩하는 시점에 타입이 있는 구조체로 바꾸면서 모든 검증(길이, 범위, enum 값)을 끝낸다. `game` 크레이트에 들어온 값은 이미 유효하다고 가정한다.
- 세션 하나에는 읽기 task와 쓰기 task를 하나씩 둔다. 쓰기는 bounded 채널로 받는다.

---

## 8. 데이터베이스

- DB 접근 코드는 `db` 크레이트에만 둔다. 다른 크레이트에 SQL 문자열이 있으면 안 된다. Redis 호환 저장소 접근도 같다(외부 저장소 접근은 모두 `db` 크레이트).
- **Redis 호환 저장소는 "모든 서버가 같이 보는 게시판"이다**(설계 03 9장). 규칙:
  - 재화, 아이템, 거래 상태, outbox, 멱등 키 같은 **정답 데이터를 넣지 않는다.** 정답은 DB다.
  - **단일 명령만 쓴다.** Lua 스크립트, `MULTI`/`EXEC`, `WATCH`는 쓰지 않는다.
  - "읽고, 판단하고, 쓰기"가 필요하면 Redis가 아니라 **그 판정의 주인인 액터**가 한다.
  - 모든 키에는 TTL 또는 명시적인 삭제 경로가 있어야 한다.
  - 두 명령 사이의 틈(예: `GET` 확인 후 `PEXPIRE`)이 데이터를 망가뜨릴 수 있으면 **DB에서 fencing 번호로 막는다**(옛 번호의 쓰기 거부).
  - Redis 명령은 `db` 크레이트에서 키 종류별 함수로 감싸고(예: `claim_space`), 다른 크레이트에서 임의의 명령을 보내지 않는다.
- **DB는 PostgreSQL을 쓴다.** `sqlx`의 컴파일 시 검사(컬럼 타입, NULL 여부 추론)가 가장 정확하고, 네이티브 `uuid` 타입과 CTE(여러 작업을 한 문장으로)를 쓸 수 있다. 데이터 ID는 `uuid` 컬럼에 저장한다(설계 03 7장).
- **저장 프로시저(SP)와 트리거는 쓰지 않는다.** DB 로직은 모두 `db` 크레이트의 Rust 함수 안에 `sqlx::query!`/`query_as!`로 쓴다. C의 Pro*C처럼 SQL을 코드에 직접 쓰고, 빌드할 때 실제 스키마로 검사된다. Rust 함수 하나가 과거의 SP 하나 역할을 한다.
  - 대가로 받아들인 것: 앱 DB 계정에 테이블 수정 권한이 필요하고, 여러 문장을 묶으면 문장마다 왕복이 생긴다.
- CI나 DB가 없는 환경에서도 빌드되도록 `cargo sqlx prepare`로 `.sqlx/` 오프라인 데이터를 커밋한다.
- **정합성은 DB가 지킨다.** 유니크 제약, `CHECK` 제약, 조건부 쓰기(`UPDATE ... WHERE 기대한 상태`), `INSERT ... ON CONFLICT`를 쓴다. 조건부 쓰기는 **영향받은 행 수(`rows_affected`)를 반드시 확인**하고, 기대한 수가 아니면 거부로 처리한다.
- **DB 함수는 async로 한 벌만 만든다.** 부르는 방식은 세 가지다(설계 01 5.7): async 코드는 `.await`로 직접(A), 존 스레드 같은 동기 코드는 요청/응답 메시지로(B), 시작 단계와 도구, 배치, 테스트는 블로킹 래퍼로(C). **존 스레드와 async 코드 안에서는 C를 쓰지 않는다.** 동기 전용 DB 드라이버를 따로 넣지 않는다.
- **트랜잭션은 Rust 코드에서 건다**(`pool.begin()` → `tx.commit()`). 원자성과 롤백은 DB가 보장하고, 커밋하지 않고 함수를 빠져나가면 `Drop`에서 자동 롤백된다.
  - 트랜잭션 안에서 DB 문장 외의 것(다른 서버 응답, Redis, 채널)을 기다리지 않는다. 문장만 실행하고 바로 커밋한다.
  - 트랜잭션이 필요한 쿼리 함수는 `&mut Transaction`을 받아서, 트랜잭션 밖에서 실수로 부르지 못하게 한다.
  - 데드락과 직렬화 실패는 트랜잭션 **전체를** 다시 실행한다. 멱등 키가 있으므로 다시 해도 안전하다.
  - 플레이어 액터는 커밋이 확인된 **뒤에** 메모리 상태를 바꾸고 클라이언트에 알린다.

예: 아이템 판매 (아이템 삭제 + 돈 증가를 한 트랜잭션으로)

```rust
pub async fn sell_item(pool: &PgPool, player: PlayerId, item: ItemInstanceId, price: Gold)
    -> Result<SellOutcome, DbError>
{
    let mut tx = pool.begin().await?;                                  // BEGIN

    let deleted = sqlx::query!(
        "DELETE FROM inventory WHERE item_instance_id = $1 AND player_id = $2 AND locked = false",
        item.as_uuid(), player.as_uuid()
    ).execute(&mut *tx).await?.rows_affected();
    if deleted != 1 { return Ok(SellOutcome::ItemNotFound); }          // return → 자동 ROLLBACK

    let updated = sqlx::query!(
        "UPDATE wallet SET gold = gold + $2 WHERE player_id = $1 AND gold <= $3 - $2",
        player.as_uuid(), price.as_i64(), Gold::MAX.as_i64()
    ).execute(&mut *tx).await?.rows_affected();
    if updated != 1 { return Ok(SellOutcome::GoldOverflow); }          // 삭제도 함께 ROLLBACK

    tx.commit().await?;                                                // COMMIT: 두 테이블 함께 반영
    Ok(SellOutcome::Sold)
}
```

- 가격은 서버의 기획 데이터에서 가져온다. 클라이언트가 보낸 값을 쓰지 않는다.
- 재전송된 요청의 중복 처리를 막으려면 같은 트랜잭션에 멱등 키 `INSERT`를 한 줄 더 넣는다(설계 03 3장).
- 스키마 변경은 `migrations/`의 마이그레이션 파일로만 한다. 수동으로 DDL을 실행하지 않는다.
- 재화 이동, 거래, 우편처럼 여러 행이 바뀌는 작업은 반드시 하나의 트랜잭션으로 처리한다. 함수 문서 주석에 트랜잭션 범위(무엇이 함께 성공하거나 실패하는가)를 남긴다.
- 재시도할 수 있는 요청(지급, 결제)에는 멱등성 키를 둔다.
- DB 호출은 게임 로직 task(Zone 액터) 안에서 직접 await하지 않는다. Zone 틱이 DB 지연에 묶이기 때문이다. DB 작업은 별도 task로 보내고 결과는 메시지로 돌려받는다.

---

## 9. 로깅과 관측

- `println!`/`eprintln!`/`dbg!`는 쓰지 않는다(clippy가 막음). `tracing` 매크로를 쓴다.
- 로그에는 문자열을 이어 붙이지 말고 구조화된 필드를 넣는다.

```rust
tracing::info!(player = ?player_id, zone = ?zone_id, "player entered zone");
```

- 세션 단위 작업에는 `#[tracing::instrument]`나 span으로 `session_id`를 붙여서 로그를 추적할 수 있게 한다.
- 레벨 기준:
  - `error`: 운영자가 확인해야 한다.
  - `warn`: 비정상이지만 처리했다. 클라이언트가 잘못된 패킷을 보낸 경우가 여기에 해당한다.
  - `info`: 접속, 종료, 주요 상태 변화.
  - `debug`/`trace`: 개발용.

---

## 10. unsafe

- `unsafe` 코드는 금지한다(`unsafe_code = "forbid"`).
- 꼭 필요해지면(FFI 등) 별도 크레이트로 격리하고, 그 크레이트에만 예외를 둔다. 모든 `unsafe` 블록 위에는 왜 안전한지를 `// SAFETY:` 주석으로 적는다.

---

## 11. 테스트

- 단위 테스트는 같은 파일 아래쪽 `#[cfg(test)] mod tests`에 둔다. 통합 테스트는 크레이트의 `tests/` 디렉터리에 둔다.
- `protocol`과 `game`은 순수 로직이므로 테스트를 충분히 쓴다. 인코딩 후 디코딩하면 원래 값이 나오는지 확인하는 왕복 테스트와 경계값 테스트가 기본이다.
- 테스트에서 `sleep`으로 타이밍을 맞추지 않는다. 시간이 들어가는 로직은 `tokio::time::pause()`로 가상 시간을 쓰거나, 채널과 이벤트를 기다리게 한다.
- 테스트 이름은 무엇을 검증하는지 문장처럼 쓴다: `move_rejected_when_target_outside_zone`.
- 테스트 코드 안에서는 `unwrap`/`expect`를 허용한다(`clippy.toml`에 설정되어 있음).

---

## 12. 문서와 주석

- `pub` 항목에는 `///` 문서 주석을 단다. 무엇을 하는지, 언제 실패하는지를 적는다.
- 각 모듈 파일 맨 위에는 `//!` 주석으로 그 모듈의 책임을 한두 줄로 적는다.
- 주석에는 "무엇"보다 "왜"를 쓴다. 코드를 그대로 읽어 주는 주석은 달지 않는다.

---

## 13. 외부 크레이트 (의존성)

기본 허용 목록이다. 여기 없는 크레이트를 추가하려면 이유를 남기고 합의한다.

| 용도 | 크레이트 |
|---|---|
| async 런타임 | `tokio` |
| 버퍼, 코덱 | `bytes`, `tokio-util` |
| 직렬화 | `serde` (+ 형식별 `serde_json`, `toml`) |
| 패킷 직렬화 | `prost` (설계 02 8.2에서 확정) |
| 데이터 ID | `uuid` (`v7` 기능) |
| TLS | `rustls` (`tokio-rustls`) |
| 에러 | `thiserror` (라이브러리), `anyhow` (바이너리) |
| 로깅 | `tracing`, `tracing-subscriber`(JSON 기능), `tracing-appender`(파일 회전, 비동기 쓰기) |
| 지표 | `metrics`, `metrics-exporter-prometheus` |
| HTTP (관리 포트, Entry 서버) | `axum` |
| DB | **PostgreSQL** + `sqlx` (`postgres` 기능) |
| Redis 호환 저장소 | `redis` (2026-11-19 결정. 단일 명령만 쓰는 용도에 충분) |
| 금액 (필요할 때) | `rust_decimal` |
| HTTP 클라이언트 (스팀 Web A​PI 등 외부 호출) | `reqwest` (`rustls` 기능) — 2026-11-19 승인 |
| 개발용 런타임 관찰 | `console-subscriber` (tokio-console). **개발 빌드의 기능 플래그로만 켜고 release에는 넣지 않는다** — 2026-11-19 승인 |
| 지연 시간 분포 측정 (부하 테스트) | `hdrhistogram` (`bot` 크레이트 등 측정 도구에서만) — 2026-11-19 승인 |

| GUI, 그래픽 (도구 전용) | `bevy` + `bevy_egui` (`client` 크레이트), `eframe`/`egui` (`launcher` 크레이트). **서버 바이너리에는 넣지 않는다** — 2026-11-19 승인 |
| 프로토콜 코드 생성 (빌드 전용) | `prost-build` — 2026-11-19 승인 |
| 토큰 서명 | `ed25519-dalek` (Entry가 서명, Gateway는 공개 키로 확인) — 2026-11-19 승인 |
| 난수 (세션 키 등) | `rand` — 2026-11-19 승인 |
| 명령줄 인자 | `clap` (바이너리 크레이트에서만) — 2026-11-19 승인 |

- 보류: 전역 할당자 `mimalloc`은 부하 테스트로 효과를 확인한 뒤 결정한다.

- 같은 일을 하는 크레이트를 두 개 넣지 않는다(예: `log`와 `tracing`, `async-std`와 `tokio`).
- 기능 플래그는 필요한 것만 켠다(`tokio = { features = ["full"] }`는 `server`와 `bot`에서만 허용).

---

## 14. C++ 개발자가 자주 빠지는 함정

| C++ 습관 | Rust에서 |
|---|---|
| 상속으로 다형성 | 계속 늘어나고 하나하나가 큰 것(예: InGameSpace 모드)은 `trait`, 고정된 작은 집합(예: 이벤트, 종료 사유)은 `enum` + `match` |
| 베이스 클래스에 공통 필드 | 공통 부분을 구조체로 빼서 composition |
| 함수 오버로딩 | 없다. 이름을 다르게 짓거나 trait을 쓴다 |
| 기본은 복사, 필요하면 move | 기본이 move다. 복사는 `Clone`/`Copy`로 명시한다 |
| 소멸자로 RAII | `Drop`이 같은 역할을 한다. 락 가드, 소켓, 트랜잭션 모두 스코프를 벗어나면 정리된다 |
| `nullptr` 체크 | `Option`을 `match`하거나 `?`로 처리. null 포인터는 없다 |
| 예외 | `Result`와 `?` |
| 전역 싱글턴 매니저 | 생성해서 인자로 넘긴다(`Arc<Context>`). 꼭 필요하면 `std::sync::OnceLock` |
| 헤더/구현 분리 | 없다. `pub`로 공개 범위를 조절한다. 기본은 비공개 |
| 매크로로 코드 생성 | 먼저 제네릭과 trait로 해결하고, `macro_rules!`는 마지막 수단으로 쓴다 |

---

## 15. 공개 범위

- 기본은 비공개다. 외부에서 써야 하는 것만 `pub`로 연다.
- 크레이트 안에서만 공유하는 것은 `pub(crate)`로 연다.
- 구조체 필드는 비공개로 두고 생성자와 메서드로 불변 조건을 지킨다. 단, `protocol`의 단순 데이터 구조체처럼 불변 조건이 없는 타입은 필드를 공개해도 된다.

---

## 16. 리뷰 체크리스트

변경을 마칠 때마다 확인한다.

- [ ] `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`가 모두 통과한다
- [ ] 새 lint 예외는 `#[expect(..., reason = "...")]`이고 이유가 타당하다
- [ ] 테스트 밖에 `unwrap`, `expect`, `panic!`, `todo!`가 없다
- [ ] ID와 금액이 newtype이고, 재화 연산은 `checked_*`다
- [ ] 직접 만든 enum의 `match`에 `_ =>`가 없다
- [ ] 의존 방향(2장)을 지켰고, SQL과 Redis 접근은 `db` 크레이트에만 있다
- [ ] Redis에는 단일 명령만 쓰고(Lua, `MULTI`, `WATCH` 없음), 정답 데이터를 넣지 않았다
- [ ] 락 가드를 쥔 채로 `.await`하지 않고, 채널은 bounded다
- [ ] 외부 입력(패킷, 설정)은 경계에서 검증했다
- [ ] 새 외부 크레이트를 추가했다면 13장 목록에 반영했다
- [ ] `pub` 항목에 문서 주석이 있다

---

## 부록 A. 루트 Cargo.toml 템플릿

워크스페이스를 만들 때 이 내용을 기준으로 시작한다. 각 멤버 크레이트의 `Cargo.toml`에는 `[lints] workspace = true`를 반드시 넣는다.

```toml
[workspace]
resolver = "3"
members = ["crates/*"]

[workspace.package]
edition = "2024"
version = "0.1.0"
publish = false

[workspace.dependencies]
# 외부 크레이트 버전은 여기에서만 관리한다 (13장 참고)

[workspace.lints.rust]
unsafe_code = "forbid"
missing_debug_implementations = "warn"
unreachable_pub = "warn"

[workspace.lints.clippy]
# 그룹: 더 엄격한 제안을 켠다 (priority -1 = 개별 설정보다 먼저 적용)
pedantic = { level = "warn", priority = -1 }
# pedantic 중 초반 학습에 소음이 큰 것만 끈다
must_use_candidate = "allow"
missing_errors_doc = "allow"
missing_panics_doc = "allow"
# 패닉 경로 금지
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
todo = "deny"
unimplemented = "deny"
# 디버그 출력 금지 (tracing 사용)
dbg_macro = "deny"
print_stdout = "deny"
print_stderr = "deny"
# lint 예외는 #[expect(..., reason = "...")]만 허용
allow_attributes = "deny"
allow_attributes_without_reason = "deny"
# 정수/형변환 안전
cast_possible_truncation = "warn"
cast_sign_loss = "warn"
```

멤버 크레이트 예시 (`crates/game/Cargo.toml`):

```toml
[package]
name = "game"
edition.workspace = true
version.workspace = true
publish.workspace = true

[lints]
workspace = true

[dependencies]
protocol = { path = "../protocol" }
thiserror = { workspace = true }
```
