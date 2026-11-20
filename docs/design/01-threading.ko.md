# 설계 01. 스레드 모델

[English](01-threading.md) | **한국어**

- 상태: **v2.5** (2026-11-19). v2.5: 플레이어 액터를 OS 스레드에 고정하지 않는 방식(tokio 멀티 스레드 런타임) 확정, 디버깅과 추적 규칙 추가(5.8). v2.4: AOI 칸 단위 NPC 활성화(4.6), 엔티티 단위 대기 상태(4.8), 운영 명령과 재로드의 C 금지(5.7), 시작 순서와 기동 복구(10.0) 추가. v2.3: 컨텐츠를 동기 스레드에서 **플레이어별 async 액터**(컨텐츠 전용 tokio 런타임)로 바꿈. v2.1: Directory와 서버 등록을 Redis 기반으로 바꿈(2026-11-18 확정). v2.2: Redis에는 단일 명령만 쓰고 판정은 InGameSpaceManager 액터가, 정합성은 DB fencing이 맡음. v1에서 바뀐 것: Gateway 도입에 따른 GatewayLink, 컨텐츠 스레드 추가, InGameSpace 구조(코어 + 모드), 연결 이벤트 정책, InGameSpaceManager 2층 구조
- 선행 문서: `02-session-network.md` (세션, 네트워크), `03-data-ownership.md` (데이터 소유, 배치), `04-database.md` (DB 구성)
- 관련: `docs/RUST_GUIDELINES.md` 6장 (소유권, 동시성, async)

## 0. 요구사항

### 0.1 규모

| 항목 | 값 |
|---|---|
| 프로세스당 동접 | 10,000 이상 |
| 게임 틱 | 10Hz (100ms) |
| 운영 OS | Linux (개발은 Windows) |
| 패닉 정책 | 즉시 프로세스 종료 (`panic = "abort"`) + 코어 덤프 + 감시자 재시작 |

### 0.2 InGameSpace

플레이어가 존재하는 모든 장소를 **InGameSpace**라고 부른다. 배치 규칙은 03 문서 5장을 따른다.

| 종류 | 설명 | 호스트 | 수명 | 상태 저장 |
|---|---|---|---|---|
| **로비** | 개인 또는 소수 커뮤니티 전용 MMO형 필드. NPC가 많음. 다른 샤드 친구가 방문 가능 | 주인의 샤드 | 입장 시 로드, 비면 언로드 | **영구** (주인 DB) |
| **MMO 필드** | 로비에서 이동하는 공용 필드 | 각 샤드 (채널) | 상주 | 플레이어 상태만 |
| **인스턴스 던전** | 파티 단위 일회성 던전. 섞인 샤드 파티 가능 | 전투 서버 풀 | 생성 ~ 클리어/이탈 | 결과만 (본 샤드로) |
| **길드 던전** | 자격 있는 플레이어가 생성. 기간 한정 | 부하가 낮은 Game 서버 (길드는 샤드에 속하지 않음, 04 문서 5.2) | 생성 ~ 만료 (재시작을 넘어 지속) | **기간 내 영구** (Common DB) |
| **PvP 필드** | 1:1, N:N. 여러 샤드의 플레이어 | 전투 서버 풀 | 매치 단위 | 결과만 (본 샤드로) |

### 0.3 스레드 설계에 주는 영향

1. **InGameSpace 수가 매우 많고 대부분 가볍다.** 접속자마다 로비가 생길 수 있다.
2. **InGameSpace가 수시로 생기고 사라지고, 다른 서버에 있을 수 있다.** 찾고 배치하는 일이 클러스터 단위다(6장).
3. **영구 데이터(컨텐츠)와 시뮬레이션(존)의 주인이 다르다.** 컨텐츠는 본 샤드, 존은 InGameSpace 호스트(03 문서). 그래서 처리 방식도 나눈다(존은 동기 스레드, 컨텐츠는 async 액터).
4. **클라이언트 소켓은 Gateway에만 있다.** 백엔드는 Gateway 링크와 서버 간 링크만 다룬다(02 문서).

---

## 1. 설계 원칙

1. **상태를 가진 것은 모두 "자기 Job 큐 + 단일 실행 흐름"을 가진다(액터).** 다른 곳은 그 큐에 넣는 **핸들**만 가진다. 그래서 락이 없다.
2. **게임 로직은 락 없이 단일 스레드로 돈다.** InGameSpace 하나의 상태는 정확히 한 존 스레드만, 플레이어 한 명의 컨텐츠 상태는 정확히 한 **플레이어 액터**만 만진다.
3. **기다림은 async, 시뮬레이션은 동기.** 기다림이 대부분인 일(소켓, DB, 서버 간 통신, 조정, **컨텐츠**)은 tokio에서 async로, 계산과 고정 틱이 핵심인 일(**존**)은 전용 OS 스레드에서 동기 루프로 처리한다.
4. **틱이 흔들리지 않는다.** 네트워크 부하나 DB 지연이 틱 주기에 영향을 주지 않는다.
5. **어떤 스레드도 다른 스레드를 기다리며 막히지 않는다.** 스레드 간 통신은 bounded 채널과 non-blocking 전송만 쓴다.
6. **스레드 수는 설정으로 정한다.**
7. **잠든 InGameSpace는 비용이 거의 없다.**

---

## 2. 전체 구조

### 2.1 Game 서버 (샤드)

```
                      ┌──────────────────────────────────────────────────────┐
  Gateway ──링크───>  │ I/O 런타임 (tokio multi-thread, io-worker-N)         │
  다른 서버 ─링크──>  │  GatewayLink × (Gateway 수 × 링크 수)                │
                      │  ServerLink × (다른 서버 수 × 링크 수)               │
                      │  InGameSpaceManager task (프로세스 안 배치)          │
                      │  DB task (sqlx 커넥션 풀), KV task (Redis 호환)      │
                      └──────┬──────────────────────────────┬────────────────┘
          역직렬화된 메시지  │ Content                      │ Zone
                             ▼                              ▼
          ┌───────────────────────────────┐   ┌──────────────────────────────────┐
          │ 컨텐츠 런타임 (tokio, content-N)│   │ 존 스레드 × Kz (zone-N)          │
          │  PlayerContent × (담당 플레이어)│<─>│  zone-0: [MMO 필드 A] [로비 ×1200]│
          │  인벤토리, 재화, 우편, 퀘스트  │   │  zone-1: [로비 ×1150] [길드던전]  │
          └───────────────────────────────┘   └──────────────────────────────────┘

  그 외: main 스레드, tokio blocking 풀, 로그 기록 스레드
```

### 2.2 전투 서버

Game 서버와 같은 바이너리를 다른 역할로 띄운다. **컨텐츠 런타임과 DB task가 없다.** 존 스레드에서 인스턴스 던전과 PvP 필드만 돈다. 플레이어 데이터는 각자의 본 샤드와 서버 간 메시지로 주고받는다(03 문서).

### 2.3 Gateway

존 스레드도 컨텐츠 런타임도 없다. 모든 것이 tokio 위의 task다(02 문서 3장).

### 2.4 C++ 서버와의 대응

| 이 설계 | C++ MMO 서버 |
|---|---|
| I/O 런타임 워커 | IOCP 워커 스레드 풀 |
| GatewayLink, ServerLink | 서버 간 통신 세션 |
| 존 스레드 | Zone 전용 스레드 (Job 큐 + 틱 루프) |
| 플레이어 액터 (컨텐츠 런타임) | 플레이어 DB 작업/컨텐츠 처리 전용 스레드 + DB Job 콜백. 콜백 대신 `.await` |
| InGameSpaceManager + Redis Directory | ZoneManager 싱글톤 + 월드 서버 |
| 핸들 (채널 송신 끝) | 싱글톤 `Instance()` + Job 투입 |

---

## 3. 스레드와 task 목록 (Game 서버)

### 3.1 OS 스레드

| 이름 | 개수 | 하는 일 | 하면 안 되는 일 |
|---|---|---|---|
| `main` | 1 | 설정 로드, 런타임과 스레드 생성, Redis에 서버 등록, 종료 지휘 | 게임 로직 |
| `io-worker-N` | 설정값 | 3.2의 task들을 실행. 패킷 역직렬화와 검증 | 게임 상태 접근, 블로킹 호출 |
| `zone-N` | Kz (설정값) | InGameSpace 틱, Zone 메시지 처리 | 소켓 I/O, DB 직접 호출, 채널 수신 외의 블로킹 |
| `content-N` | Kc (설정값) | **컨텐츠 런타임**(전용 tokio)의 워커. 플레이어 액터와 ContentManager를 실행 (5장) | 블로킹 호출 |
| tokio blocking 풀 | 필요할 때 | 파일 로드, 동기 라이브러리 호출 | 상주 루프 |
| 로그 기록 | 1 (`tracing-appender`) | 로그 파일 쓰기 | |

모든 스레드에는 이름을 붙인다(`thread::Builder::name`, tokio `thread_name`).

### 3.2 I/O 런타임 위의 task

| task | 개수 | 하는 일 |
|---|---|---|
| Acceptor | 1 | 서버 간 엔드포인트에서 Gateway와 다른 서버의 링크 수락 |
| GatewayLink | Gateway 수 × 링크 수 (reader/writer) | 봉투 해석, 역직렬화, 검증, 카테고리별로 플레이어 액터(Content) 또는 존 스레드(Zone)에 전달. 응답 송신 |
| ServerLink | 다른 서버 수 × 링크 수 | 서버 간 메시지 송수신 (02 문서 4.4) |
| InGameSpaceManager | 1 | 프로세스 안 InGameSpace 생성, 배치, 소멸. host인 InGameSpace의 Redis 임대 갱신과 입장 대기열 (6장) |
| DB | 1 (커넥션 풀 동시 사용) | 로드, 저장, 트랜잭션 실행. 결과를 요청한 스레드에 회신 |
| KV | 1 (커넥션 풀 동시 사용) | Redis 호환 저장소 접근 (플레이어 디렉터리 조회, 온라인 표시 등, 03 문서 9장). 결과를 메시지로 회신 |

---

## 4. 존 스레드

### 4.1 왜 tokio task가 아니라 전용 OS 스레드인가

- tokio의 work-stealing 스케줄러는 처리량에는 좋지만 특정 task의 실행 시점을 보장하지 않는다. 동접 1만의 I/O task와 섞이면 틱이 밀린다.
- InGameSpace 상태는 단일 스레드 소유이므로 work-stealing의 이점이 없다.
- InGameSpace가 특정 스레드에 고정되면 캐시 지역성이 좋고 프로파일링 결과를 해석하기 쉽다.

존 스레드 안에는 async가 없다. 평범한 동기 루프다.

### 4.2 ZoneHost

존 스레드 하나 = ZoneHost 하나. InGameSpace 여러 개를 소유하고, 틱을 돌리고, Zone 메시지를 해당 InGameSpace에 전달한다. `server` 크레이트에 속한다.

### 4.3 InGameSpace 구조: 공통 코어 + 모드

InGameSpace는 C++의 **추상 클래스 + 파생 클래스** 모델을 따른다. Rust에는 클래스 상속이 없으므로 **공통 데이터(구조체)**와 **모드별 동작(trait)**으로 나눈다. `game` 크레이트에 속하며 스레드, 채널, tokio를 모른다.

| C++ | Rust |
|---|---|
| 추상 클래스 `InGameSpace` | `struct InGameSpace { core, mode }` |
| 베이스의 protected 멤버 | `InGameSpaceCore`의 필드 |
| 순수 가상 함수 | trait의 **필수 메서드** (구현하지 않으면 컴파일 에러) |
| 기본 구현이 있는 가상 함수 | trait의 **기본 메서드** |
| 파생 클래스 | `struct LobbyMode` 등 + `impl InGameSpaceMode` |
| 베이스의 템플릿 메서드 | `InGameSpace::tick()`이 훅을 순서대로 호출 |

```rust
// 공통 데이터: "베이스 클래스의 멤버"
pub struct InGameSpaceCore {
    pub key: InGameSpaceKey,
    pub players: HashMap<PlayerId, PlayerPresence>, // CombatProfile로 만든 대리 객체 (03 문서 1.1)
    pub aoi: AoiGrid,
    pub npcs: NpcTable,
    pub dirty: DirtyTracker,
}

// 모드별 동작: "순수 가상 함수"
pub trait InGameSpaceMode: Send {
    fn kind(&self) -> InGameSpaceKind;
    fn on_enter(&mut self, core: &mut InGameSpaceCore, player: PlayerId, out: &mut Outbox)
        -> Result<(), EnterError>;
    fn on_leave(&mut self, core: &mut InGameSpaceCore, player: PlayerId, out: &mut Outbox);
    fn on_link_event(&mut self, core: &mut InGameSpaceCore, player: PlayerId,
                     event: &LinkEvent, out: &mut Outbox);
    fn on_tick(&mut self, core: &mut InGameSpaceCore, dt: Duration, out: &mut Outbox);

    fn persistence(&self) -> Persistence { Persistence::None }
    fn on_mode_command(&mut self, _core: &mut InGameSpaceCore, _cmd: ModeCommand,
                       _out: &mut Outbox) -> Handled { Handled::No }
}

// 추상 클래스 자리: 공통 + 모드를 묶고 처리 순서를 고정
pub struct InGameSpace {
    core: InGameSpaceCore,
    mode: Box<dyn InGameSpaceMode>,
}

impl InGameSpace {
    pub fn tick(&mut self, dt: Duration, out: &mut Outbox) {
        self.core.aoi.begin_tick();
        self.mode.on_tick(&mut self.core, dt, out);
        self.core.flush_dirty(out);
    }
}
```

- **protected 접근이 인자로 바뀐다.** 모드는 공통 데이터를 `core: &mut InGameSpaceCore` 인자로 받는다. 모드가 공통 데이터를 건드리는 곳이 시그니처에 드러나고, `core`와 `mode`가 서로 다른 필드라서 동시에 빌려도 borrow checker가 허용한다.
- **새 모드를 추가해도 기존 코드를 고치지 않는다.** 파일 하나를 만들고 trait을 구현한다.
- **모드가 처리를 빠뜨리면 컴파일이 안 된다.** 필수 메서드는 모든 모드가 구현해야 한다.
- 고정된 작은 집합(`LinkEvent`, `CloseReason`, 패킷 종류)은 enum으로 두고, 모드 안에서 `match`로 빠짐없이 처리한다.
- `Outbox`: 이번 처리에서 생긴 "내보낼 것"(세션으로 보낼 패킷, 컨텐츠로 보낼 메시지, InGameSpaceManager에 알릴 것)을 쌓는 버퍼다. InGameSpace는 결과를 쌓기만 하고 실제 전송은 ZoneHost가 한다. 그래서 네트워크 없이 단위 테스트할 수 있다.

파일 구성:

```
crates/game/src/space/
├─ mod.rs          # InGameSpace (템플릿 메서드), InGameSpaceMode trait
├─ core.rs         # InGameSpaceCore
├─ factory.rs      # InGameSpaceKey/종류 → Box<dyn InGameSpaceMode>
└─ modes/
   ├─ lobby.rs
   ├─ field.rs
   ├─ dungeon.rs
   ├─ guild_dungeon.rs
   └─ pvp.rs
```

### 4.4 연결 이벤트 정책 (모드별 `on_link_event`)

Gateway는 사실(`LinkEvent`, 02 문서 4.3)만 알리고, 게임적인 결과는 모드가 결정한다.

| 모드 | Detached / Suspended | Resumed / Replaced | Closed |
|---|---|---|---|
| 로비 | 아무것도 안 함 | 스냅샷 전송 | 퇴장 |
| MMO 필드 | 그 자리에 남음. N초 뒤 무적 또는 숨김 (기획) | 스냅샷 전송 | 퇴장 |
| 인스턴스 던전 | 파티에 "연결 끊김" 표시. 그대로 둘지 AI가 대신 싸울지는 기획 | 스냅샷, 파티에 복귀 알림 | 퇴장, 파티에 알림 |
| 길드 던전 | 던전과 같음 | 같음 | 같음 |
| PvP 필드 | 상대에게 알림 + **몰수패 타이머**(예: 30초) | 타이머 취소, 스냅샷 | 몰수패 확정 |

- 게임 쪽 타이머(몰수패 등)는 Gateway의 유예 시간과 **별개**로 모드가 직접 잰다.
- `Replaced`(중복 로그인 인계)는 클라이언트가 처음부터 시작한 것이므로 **항상 전체 스냅샷**을 보낸다.

### 4.5 틱 루프

```rust
// server 크레이트: zone-N 스레드 본문 (개략)
fn run_zone_host(mut host: ZoneHost, rx: Receiver<ZoneCommand>, period: Duration) {
    let mut next_tick = Instant::now() + period;
    loop {
        // 1) 다음 틱까지 남은 시간 동안 명령을 받는 즉시 처리 (상한 있음)
        let mut handled = 0;
        while handled < MAX_COMMANDS_PER_TICK {
            let now = Instant::now();
            if now >= next_tick { break; }
            match rx.recv_timeout(next_tick - now) {
                Ok(ZoneCommand::Shutdown) => return host.shutdown(),
                Ok(cmd) => { host.dispatch(cmd); handled += 1; }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return host.shutdown(),
            }
        }

        // 2) 틱: 깨어 있는 InGameSpace만
        host.tick_active_spaces(period);
        host.flush_outbox();   // GatewayLink/컨텐츠/InGameSpaceManager로 try_send

        // 3) 다음 틱 시각. 밀렸으면 따라잡지 않고 건너뜀
        next_tick += period;
        let now = Instant::now();
        if now > next_tick {
            tracing::warn!(overrun_ms = (now - next_tick).as_millis(), "tick overrun");
            next_tick = now + period;
        }
    }
}
```

- 명령은 도착 즉시 처리하고, 틱은 고정 주기로 돈다.
- 틱당 명령 처리 수에 상한(`MAX_COMMANDS_PER_TICK`)을 두어 틱이 굶지 않게 한다.
- 틱이 밀리면 몰아서 돌리지 않고 건너뛴다. 밀릴 때마다 경고 로그를 남기고 지표로 집계한다.

### 4.6 잠든 InGameSpace는 틱을 돌지 않는다

| 목록 | 조건 | 틱 |
|---|---|---|
| active | 플레이어가 1명 이상 | 매 틱 |
| dormant | 플레이어 없음, 언로드 유예 중 | 돌지 않음. 유예가 끝나면 InGameSpaceManager에 "비었음"을 알림 |

로비 NPC의 시간 경과(일정, 재생성 등)는 매 틱 시뮬레이션하지 않는다. 깨어날 때 "마지막으로 처리한 시각"부터 지금까지를 한 번에 계산한다.

**깨어 있는 InGameSpace 안에서도 AOI 칸 단위로 잠재운다.** 넓은 필드는 플레이어가 한쪽에만 있어도 깨어 있다. 그래서 NPC AI는 플레이어가 있는 AOI 칸과 그 주변 칸에서만 갱신하고, 나머지 칸의 NPC는 멈춘다. 칸이 다시 활성화되면 위와 같이 경과 시간을 한 번에 계산한다. 아무도 보지 않는 몬스터의 순찰에 CPU를 쓰지 않기 위해서다. AOI 자체는 2D 격자다. 칸이 바뀔 때 새로 보이게 된 대상과 안 보이게 된 대상만 계산해 알린다.

### 4.7 부하 기준

- 존 스레드의 부하는 InGameSpace 수가 아니라 **"종류별 가중치 × 플레이어 수"의 합**으로 잰다.
- 각 ZoneHost는 틱마다 자기 부하(가중치 합, 직전 틱 소요 시간)를 원자 변수(`AtomicU64`)에 기록한다. InGameSpaceManager는 배치할 때 이 값만 읽는다.

### 4.8 결과를 기다려야 하는 행동: 그 플레이어만 대기 상태로 둔다

존에서 일어나는 행동 중에는 다른 쪽(DB task, 플레이어 액터, 다른 서버)의 결과가 와야 다음 단계로 갈 수 있는 것이 있다. 예를 들면 장착 확정(03 문서 4.2), 다른 서버로의 이동(6.6), 인스턴스 입장 확인이다. 이때 **존 전체나 스레드를 멈추지 않고, 그 플레이어의 대리 객체만 대기 상태로 둔다.** C++ 서버에서 요청을 DB 워커에 넘기고 완료 콜백에서 다음 단계로 가던 방식과 같은 모양이다.

```rust
// game 크레이트: PlayerPresence의 필드 (개략)
pub enum Pending {
    None,
    Equip { request: RequestId, deadline: Instant },
    Transfer { request: RequestId, deadline: Instant },
}
```

| 규칙 | 내용 |
|---|---|
| 대기 중 같은 종류 요청 | 거절하고 클라이언트에 "처리 중"으로 응답한다. 두 번 누른 장착이 두 번 나가지 않게 하기 위해서다 |
| 대기 중 다른 행동 | 행동별로 정한다. 기본은 이동과 전투를 허용하고, 같은 데이터를 바꾸는 행동은 거절한다 |
| 결과 도착 | `RequestId`가 지금 대기 중인 것과 같을 때만 적용한다. 그 사이 플레이어가 나갔거나 다른 요청으로 바뀐 뒤 도착한 오래된 결과는 버린다 |
| 시간 초과 | 틱에서 `deadline`을 확인해 대기를 풀고 실패로 응답한다. DB에 반영됐는지 모르는 요청은 멱등 키로 다시 확인한다(5.5) |

오래 걸릴 수 있는 확정을 존 틱 안에서 기다리지 않는다는 점이 핵심이다. 존은 계속 돌고, 기다리는 것은 그 플레이어의 그 행동 하나다.

---

## 5. 컨텐츠: 플레이어별 async 액터

### 5.1 왜 async인가

- 컨텐츠(인벤토리, 재화, 우편, 퀘스트)는 계산이 거의 없고 **DB를 기다리는 일이 대부분**이다. 1장 원칙 3("기다림이 대부분인 일은 async")에 따라 async로 처리한다.
- 존(InGameSpace)은 계산과 고정 틱이 핵심이라 동기 스레드로 둔다(4장). **게임 시뮬레이션은 동기, 기다림은 async**로 구분이 깔끔해진다.
- C++에서는 DB Job을 던지고 콜백으로 결과를 받았다. 그 방식은 기능 하나가 요청 함수와 콜백 함수로 쪼개지고, 여러 단계 흐름에서 대기 상태가 늘어난다. async 액터에서는 **기능 하나가 함수 하나**이고, DB를 기다리는 곳에 `.await`를 쓴다.

### 5.2 구조

```
[컨텐츠 런타임: 전용 tokio multi-thread runtime, 워커 Kc개 (content-N)]

  ContentManager (액터 1개)
     - PlayerId → PlayerContent 액터 핸들
     - 액터 생성(로그인, 오프라인 플레이어 앞 메시지), 소멸(로그아웃)
     - 서버 간 링크로 들어온 메시지를 해당 플레이어 액터로 전달

  PlayerContent 액터 × 접속자 수 (+ 잠깐 깨운 오프라인 플레이어)
     - 플레이어 한 명의 컨텐츠 상태를 단독 소유
     - 자기 inbox(bounded)에서 명령을 하나씩 꺼내 처리
```

- **컨텐츠 전용 tokio 런타임**을 I/O 런타임과 따로 둔다. 네트워크 처리량이 몰려도 컨텐츠 처리가 밀리지 않고, 반대도 같다. 워커 수 `Kc`는 설정값이다.
- 1만 명이면 액터 task 1만 개다. tokio에서는 가벼운 규모다(task 하나에 수 KB).
- 전투 서버에는 컨텐츠 런타임이 없다(2.2).

### 5.3 플레이어 액터

```rust
async fn run(mut self, mut inbox: mpsc::Receiver<ContentCommand>) {
    while let Some(cmd) = inbox.recv().await {     // 명령을 하나씩 꺼낸다
        match cmd {
            ContentCommand::Sell { item, req } => self.sell(item, req).await,
            ContentCommand::Grant(grant) => self.grant(grant).await,     // 존, 전투 서버에서 온 지급
            ContentCommand::Logout => { self.drain_and_save().await; break; }
            // ... (직접 만든 enum이므로 _ => 를 쓰지 않는다)
        }
    }
}

async fn sell(&mut self, item: ItemInstanceId, req: RequestId) {
    let price = self.tables.price_of(item);
    match db::sell_item(&self.db, self.id, item, price, req).await {   // 기다리는 동안 워커는 다른 플레이어 일을 함
        Ok(SellOutcome::Sold) => { /* 메모리 반영 → 클라 응답 (Reliable) */ }
        Ok(SellOutcome::ItemNotFound) => { /* 거절 응답 */ }
        Ok(SellOutcome::GoldOverflow) => { /* 거절 응답 */ }
        Ok(SellOutcome::AlreadyProcessed) => { /* 멱등: 이전 결과로 응답 */ }
        Err(e) => { /* 5.5 규칙 */ }
    }
}
```

- **플레이어별 순서가 구조로 보장된다.** 액터는 명령 하나를 끝내야 다음 명령을 꺼낸다. 같은 플레이어의 구매 두 건이 엇갈려 재화가 음수가 되는 일이 없다. 다른 플레이어의 액터는 그동안 계속 돈다.
- 액터 안에서 락은 없다. 상태는 `&mut self`로 액터 혼자 만진다.
- 액터는 DB(`GameDb`), Redis(KV), 다른 서버로 보내는 핸들을 직접 들고 `.await`로 쓴다. 3.2의 DB task는 존 스레드와 InGameSpaceManager처럼 **동기 쪽이나 다른 액터가 DB를 쓸 때의 창구**로 남는다.

### 5.4 들어오는 길

| 보내는 쪽 | 받는 곳 | 방법 |
|---|---|---|
| 클라이언트 Content 메시지 | 그 플레이어 액터 | GatewayLink의 라우팅 테이블이 `SessionId → 액터 핸들`을 들고 바로 보낸다. 핸들은 게임 입장 때 `RouteUpdate`로 등록 |
| 같은 서버의 존 스레드 | 그 플레이어 액터 | 존은 플레이어 대리 객체에 `ContentAddr`(아래)를 들고 있어 바로 보낸다 |
| 다른 서버 (전투 서버의 `GrantItem`, 거래 서비스의 `Deliver`, 다른 샤드 친구 로비의 방문자 보상) | ContentManager → 그 플레이어 액터 | 서버 간 링크로 받은 메시지를 ContentManager가 전달 |
| **오프라인 플레이어 앞 메시지** (우편, 거래 지급, 길드 보상) | ContentManager가 액터를 **잠깐 깨움** | 액터를 만들어 DB에서 필요한 만큼 로드하고, 처리하고, 일정 시간 뒤 내린다. 같은 경로라서 멱등 처리 코드가 하나다 |

```rust
enum ContentAddr {
    Local { actor: PlayerContentHandle },   // 같은 서버: 액터 핸들
    Remote { server: ServerId },            // 다른 서버: ServerLink → 그 서버의 ContentManager
}
```

- 존 스레드(동기)에서 보낼 때는 `try_send`를 쓴다(7장). 존은 기다리지 않는다. 결과가 필요하면 회신이 존 스레드의 입력 채널로 돌아온다(예: `GrantResult`).
- inbox는 bounded다. 가득 차면 보내는 쪽 규칙(7장)에 따른다.

### 5.5 DB 처리 규칙

| 항목 | 규칙 |
|---|---|
| **쓰기 시점** | 재화와 아이템은 **커밋 후 반영**(write-through). DB가 성공해야 메모리를 바꾸고 클라이언트에 응답한다. InGameSpace 상태(로비 등)는 존 쪽의 **주기 저장**(write-behind, 6.4) |
| **트랜잭션** | `db` 크레이트의 함수 하나가 트랜잭션 하나다(가이드 8장). 액터는 그 함수를 `.await`한다 |
| **타임아웃** | 쿼리는 PostgreSQL `statement_timeout`, 함수 전체는 `tokio::time::timeout`. 타임아웃이 나면 **결과를 모르는 상태**이므로 같은 멱등 키로 다시 실행해 결과를 확인한다 |
| **재시도** | 데드락, 직렬화 실패, 연결 끊김은 트랜잭션 전체를 다시 실행한다(지수 백오프, 최대 N회). 그 밖의 에러는 재시도하지 않고 실패로 응답한다 |
| **멱등 키** | 클라이언트 요청(구매, 판매, 보상 수령)과 서버 간 메시지(`GrantItem`, `Deliver`)는 모두 멱등 키를 같은 트랜잭션에 기록한다. 재실행해도 한 번만 반영된다 |
| **동시성 상한** | 커넥션 풀 크기가 동시 DB 작업 수다(04 문서 3장). 풀이 다 차면 기다리고, 대기 한도를 넘으면 실패 + 지표 |
| **outbox** | 다른 서버로 보낼 메시지는 효과와 **같은 트랜잭션**에서 outbox 테이블에 기록하고, 커밋 후 보낸다. 확인을 받으면 지운다(03 문서 3장) |
| **로그아웃** | 진행 중인 명령을 **끝까지 마친 뒤** 남은 상태를 저장하고 액터를 내린다. 중간에 끊지 않는다 |
| **클라이언트 응답** | 커밋 후 한 번만 보낸다. 같은 요청의 재전송은 Gateway의 seq 중복 제거(02 문서 7장)와 멱등 키가 함께 막는다 |

### 5.6 액터 안에서 하면 안 되는 것

- **블로킹 호출**(동기 파일 I/O, `std::thread::sleep`, 무거운 계산). 컨텐츠 런타임의 워커를 멈춘다. 꼭 필요하면 `spawn_blocking`.
- **락을 쥔 채 `.await`**(가이드 6.2).
- **다른 플레이어 액터의 응답을 `.await`로 기다리는 것.** 두 액터가 서로를 기다리면 교착이 생긴다. 다른 플레이어와 얽히는 일(거래, 선물)은 03 문서의 멱등 메시지 흐름으로 하고, 응답은 자기 inbox로 받는다.

### 5.7 DB 호출 방식 세 가지 (구현은 하나)

DB 처리 코드는 `db` 크레이트에 **async 함수로 한 벌만** 만든다(가이드 8장). 부르는 쪽의 사정에 맞춰 **세 가지 방식**으로 쓴다. 같은 쿼리를 동기용, 비동기용으로 두 번 만들지 않는다.

| 방식 | 흐름 | 누가 쓰나 | 모양 |
|---|---|---|---|
| **A. async 직접 호출** | 결과를 기다렸다가 다음 줄로 (순차) | 플레이어 액터, Entry 서버, 거래 서비스 등 async 코드 | `db::sell_item(&db, ...).await` |
| **B. 요청/응답 메시지** | 요청만 보내고 바로 다음 일. 결과는 나중에 입력 채널로 | **존 스레드**(동기, 틱이 멈추면 안 됨), InGameSpaceManager | `db_task.try_send(DbRequest::SaveSpace { .. })` → 나중에 `ZoneCommand::DbResult` |
| **C. 블로킹 호출** | 결과가 올 때까지 **스레드가 멈춤** | 서버 시작 시 초기 로드(기획 데이터 검증, 마이그레이션 버전 확인), 운영 도구와 배치 작업(CLI), 테스트 | `blocking.run(db::load_shard_config(&db))` |

- **A**는 흐름은 순차지만 스레드는 멈추지 않는다. 기다리는 동안 워커는 다른 일을 한다.
- **B**는 C++의 "DB Job 투입 + 결과 Job 수신"과 같다. 3.2의 DB task가 받아서 A 방식으로 실행하고 결과를 돌려보낸다.
- **C**는 C++의 블로킹 DB 호출과 같다. 내부적으로는 전용 tokio 런타임의 `block_on`으로 같은 async 함수를 실행한다.

**C를 쓰면 안 되는 곳 (컴파일러가 막지 못하므로 리뷰에서 막는다)**

| 금지 위치 | 이유 |
|---|---|
| 존 스레드 | 틱이 멈춘다. B를 쓴다 |
| async 코드 안 (플레이어 액터, I/O task) | tokio 워커를 멈추고, 런타임 안에서 `block_on`을 부르면 패닉한다. A를 쓴다 |

C는 "아직 게임이 돌기 전"(시작 단계)이나 "게임 밖"(도구, 배치, 테스트)에서만 쓴다. 블로킹 래퍼 타입(`BlockingDb`)은 이 용도의 바이너리와 시작 코드에서만 만들 수 있게 한다. 시작 코드가 끝나면 `BlockingDb`를 버리고, `ServerContext`(8장)에는 넣지 않는다. 그러면 게임이 도는 동안의 코드는 C를 부를 방법 자체가 없다.

**운영 명령과 재로드도 C를 쓰지 않는다.** 운영 명령이 시작용 블로킹 함수를 재사용하면, 명령을 치는 동안 서버의 모든 존이 멈춘다. 우리는 서비스 중에 공용 데이터(시세, 이벤트 일정 등)를 다시 읽어야 하면 다음처럼 한다.

1. `/admin` 요청(08 문서 2장)을 받은 task가 A 방식으로 새 데이터를 읽는다. 그동안 게임은 옛 데이터로 계속 돈다.
2. 읽기가 끝나면 새 데이터를 읽기 전용 `Arc` 스냅샷으로 만들어, 그 데이터를 쓰는 존 스레드와 액터에 메시지로 보낸다.
3. 받은 쪽은 자기 차례에 참조를 통째로 바꾼다. 도중에 일부만 바뀐 상태는 생기지 않는다.

C++로 치면 새 테이블을 `std::shared_ptr<const T>`로 만들어 넘겨 주고, 받는 쪽이 자기 스레드에서 포인터를 바꾸는 것과 같다. 옛 테이블은 마지막 참조가 사라질 때 자동으로 해제된다.

### 5.8 액터와 스레드, 디버깅과 추적 (소유자 결정, 2026-11-19)

**결정.** 플레이어 액터는 특정 OS 스레드에 고정하지 않는다. 컨텐츠 런타임(tokio 멀티 스레드, 워커 Kc개)의 아무 워커가 실행하고, `.await`에서 멈춘 뒤 다른 워커에서 이어질 수 있다. 한 액터를 두 워커가 동시에 실행하는 일은 없으므로 락은 필요 없다. 일이 한 워커에 몰려도 다른 워커가 나눠 받는다.

**스레드 로컨 저장소와 메모리 풀.** 둘 다 쓸 수 있다. 규칙은 하나다. `thread_local!`에서 꺼낸 것을 `.await` 너머로 들고 가지 않는다. `with(|v| ...)` 안에서는 `.await`를 쓸 수 없으므로 컴파일러가 이 규칙을 지켜 준다. 유저별 데이터는 스레드 로컬이 아니라 그 액터 안에 둔다.

**추적 규칙.** 스레드 번호로는 유저를 찾을 수 없으므로, 모든 추적을 **유저번호와 요청 번호** 기준으로 한다.

| 규칙 | 내용 |
|---|---|
| 로그 문맥 | 액터가 명령 하나를 처리하는 동안 `tracing` span에 `player_id`, `request_id`(명령 번호), 명령 이름을 담는다. 그 안에서 찍히는 모든 로그(DB 호출, 메시지 송신 포함)에 이 값이 자동으로 붙는다. 서버 간 메시지에도 `request_id`를 실어 받는 쪽 로그와 이어지게 한다 |
| 기다림 지점 표시 | DB 호출, 다른 액터나 서버로의 요청처럼 `.await`로 기다리는 곳마다 하위 span을 연다. 멈춘 액터가 **무엇을 기다리는 중인지** 로그로 보인다 |
| 느린 명령 경고 | 명령 하나가 기준 시간(설정값, 예: 1초)을 넘기면 멈추지 않고 경고 로그를 남긴다. 로그에는 `player_id`, 명령 이름, 지금 기다리는 지점이 들어간다. 같은 액터에서 계속 넘기면 기준의 배수마다 다시 남긴다 |
| inbox 적체 경고 | inbox에 대기 중인 명령 수가 기준을 넘으면 경고한다. 액터가 멈췄거나 명령이 너무 많이 들어오는 상황을 잡는다 |
| 지표 | 명령 처리 시간, inbox 대기 수, 느린 명령 횟수 (08 문서 3장) |

**교착(데드락)을 막고 찾는 방법.** 액터 구조에서 락을 쓰지 않으므로 C++식 락 데드락은 생기지 않는다. 대신 생길 수 있는 것은 "서로의 응답을 기다리는 액터 둘"이다.

- **막기:** 다른 플레이어 액터의 응답을 `.await`로 기다리지 않는다(5.6). 얽히는 일은 멱등 메시지로 보내고 응답은 자기 inbox로 받는다. 채널 전송은 `try_send`라 상대 inbox가 차도 기다리지 않는다(7장).
- **찾기:** 그래도 멈춘 액터가 생기면 위 "느린 명령 경고"가 어느 플레이어의 어떤 명령이 무엇을 기다리는지 알려 준다. 서로를 기다리는 두 액터라면 두 경고가 서로의 `request_id`를 가리킨다.
- **개발 중 도구 (선택):** tokio-console로 실행 중인 모든 task의 상태(실행 중, 대기 중, 얼마나 기다렸는지)를 볼 수 있다. 외부 크레이트(`console-subscriber`)라 사용 전에 소유자 승인이 필요하다.
- **패닉:** release는 즉시 종료와 코어 덤프(10.2). 덤프의 스택과 직전 로그의 `player_id`, `request_id`로 추적한다.

---

## 6. InGameSpace 찾기와 배치: 2층 구조

InGameSpace가 다른 서버에 있을 수 있으므로("친구 로비가 어느 서버에 떠 있나") 찾고 배치하는 일이 두 층으로 나뉜다.

| 층 | 위치 | 하는 일 |
|---|---|---|
| **Directory** | **Redis 호환 저장소의 `space:{key}`** (03 문서 9.6). 별도 서비스 없음 | **누가 host인가**만 적어 두는 공용 게시판. TTL 임대로 host가 죽으면 저절로 지워짐 |
| **InGameSpaceManager** | 각 Game/전투 서버 (I/O 런타임의 task 1개, 액터) | **판정의 주인.** 이 서버가 host인 InGameSpace의 수명 상태(Loading / Active / Unloading), 입장 대기열, 임대 연장, 존 스레드 배치, 생성과 소멸 |

C++로 치면 월드 서버의 ZoneManager가 하던 일을 나눈 것이다. "누가 담당인가"는 Redis 게시판에 적어 두고 모두가 본다. 담당 서버 안에서 일어나는 판정은 그 서버의 InGameSpaceManager 액터가 Job 큐로 한 줄씩 처리한다. Redis에는 판정을 맡기지 않는다(03 문서 9.1).

### 6.1 InGameSpace 키

```rust
pub enum InGameSpaceKey {
    Lobby(LobbyOwner),                // Personal(PlayerId) | Community(CommunityId)
    MmoField(FieldId, Channel),
    Dungeon(InstanceId),
    GuildDungeon(GuildDungeonId),
    Pvp(MatchId),
}
```

같은 키의 InGameSpace는 **클러스터 전체에서** 동시에 하나만 존재한다. Redis의 **선점(`SET ... NX`)과 TTL 임대**가 이것을 보장하고, 만에 하나 생기는 좁은 틈은 **DB의 fencing 번호**가 막는다(6.4).

### 6.2 수명 상태 (host의 InGameSpaceManager가 관리)

```
Redis:  space:{key} = { host, epoch }        ← 누가 host인가 (게시판)
host 메모리: 수명 상태 + 입장 대기열          ← 판정 (액터)

            입장 요청 (키 없음 → 선점)
  [없음] ───────────────────────────> [Loading] ──로드 완료──> [Active] ──비었음──> [Unloading] ──저장 커밋──> [키 삭제]
                                          │                        ▲                    │
                                          │ 입장 요청은 host의     │ 언로드 중 입장 요청은│
                                          └─ 대기열에 쌓았다 처리   └─ host가 기다리게 했다가 삭제 후 다시 선점
```

- **선점**: `INCR space_epoch:{key}`로 fencing 번호를 받고, `SET space:{key} "{host, 번호}" NX PX 15000`. 성공한 서버가 host다. 실패하면 이미 host가 있으므로 `GET`으로 읽어 그쪽에 입장 요청을 넘긴다.
- **로딩 중**에 들어온 입장 요청은 **host의 InGameSpaceManager**가 대기열에 쌓았다가 로드가 끝나면 한꺼번에 처리한다. 같은 로비를 두 번 로드하지 않는다.
- **언로드 중**에는 최종 저장이 DB에 커밋될 때까지 키를 지우지 않는다. 그래서 그동안 **어느 서버에서도** 새로 로드하지 않는다(write-then-read 경쟁 방지). 커밋 후 `GET`으로 내가 host인지 확인하고 `DEL`한다.
- **임대 연장**: host는 5초마다 `GET`으로 내가 host인지 확인하고 `PEXPIRE`로 15초 연장한다. host가 죽으면 키가 사라지고, 다음 입장 요청이 다른 서버에서 선점해 DB로부터 다시 로드한다.
- **연장 확인에서 다른 host가 보이거나, Redis에 닿지 않아 연장에 실패하면** 그 InGameSpace의 **새 입장을 스스로 멈춘다.** 이미 안에 있는 플레이는 계속되고, 다른 host가 보였다면 플레이어를 정리한다(03 문서 9.10).
- 상태 전이(Loading → Active → Unloading)는 host 액터 한 곳에서만 일어나므로 경쟁 조건이 구조적으로 생기지 않는다.

### 6.3 입장 흐름 (예: 상태 저장 InGameSpace인 로비)

```
요청한 서버 (예: 플레이어의 본 샤드) ──GET space:{key}──> Redis
   [키 없음] → 배치할 서버 결정 (로비는 주인 샤드, 던전/PvP는 server:{id} 부하가 가장 낮은 전투 서버)
             ──CreateSpace(key, player)──> 그 서버의 InGameSpaceManager
                    InGameSpaceManager: INCR space_epoch + SET space:{key} NX PX  → 선점 성공 = host
                                        (실패하면 이미 생긴 host로 요청을 넘김)
                    InGameSpaceManager ──Load──> DB task ──> 데이터
                    가장 한가한 존 스레드 선택 (4.7) → 생성 → (메모리에서) Active
   [키 있음] ──EnterSpace(key, player)──> host의 InGameSpaceManager (Loading이면 대기열에)

본 샤드 컨텐츠 ──CombatProfile──> host 존 스레드
host ──SwitchZone(session, host)──> Gateway   (다른 서버일 때만, 02 문서 6장)
host ──RouteUpdate(session, zone)──> 해당 GatewayLink
```

- 선점은 **host가 될 서버가 직접** 한다. 그래서 선점에 성공한 서버와 실제로 InGameSpace를 띄우는 서버가 항상 같다.
- 존 스레드는 DB도 Redis도 기다리지 않는다. 로드는 DB task, 선점과 연장은 KV task가 하고, 결과가 메시지로 넘어온다.
- MMO 필드는 서버 시작 시 생성하고 `space` 키를 선점해 처음부터 Active다. 인스턴스 던전과 PvP는 DB 로드 없이 전투 서버에서 바로 생성한다.

### 6.4 저장 정책과 fencing (로비, 길드 던전)

- InGameSpace는 변경분에 dirty 표시를 하고 Outbox로 DB 저장 요청을 낸다.
  - 중요한 결과(보상 지급과 연결된 상호작용): **즉시** 저장
  - 일반 상태: **주기적**(설정값, 예: 30초)
  - 언로드 시: **최종** 저장. 완료 회신을 받아야 키를 삭제한다
- 저장 요청에는 두 번호를 붙인다.
  - **fencing 번호** (선점 때 받은 `space_epoch`): DB는 InGameSpace마다 마지막으로 받은 fencing 번호를 기록하고 **그보다 낮은 번호의 저장을 거부**한다. 임대 연장의 좁은 틈(03 문서 9.6)으로 옛 host가 잠깐 남아 있어도 그 저장은 버려진다.
  - **버전 번호** (같은 host 안에서 단조 증가): 같은 host의 저장이 순서가 바뀌어 도착해도 더 낮은 버전은 무시한다.
- 조건부 저장은 `UPDATE ... WHERE fencing <= $mine AND version < $mine` 형태의 쿼리로 구현하고, 영향받은 행이 0이면 거부된 것으로 처리한다(가이드 8장. SP는 쓰지 않는다).
- 방문자 행동의 결과는 03 문서 6.2에 따라 나눈다. 로비 상태는 주인 DB(로컬 저장), 방문자 보상은 방문자 본 샤드로 메시지.

### 6.5 길드 던전의 만료

- 만료 시각(`expires_at`)은 **DB 값이 기준**이다. 서버가 재시작되어도 기간이 유지된다.
- 길드 던전은 틱에서 만료 시각을 확인하고, 지나면 플레이어를 내보낸 뒤 InGameSpaceManager에 "만료됨"을 알린다.
- 언로드된 상태로 만료 시각이 지난 경우, 다음 입장 시도 때 선점하려는 서버가 DB의 만료 시각을 보고 거절한다. 정리는 별도 배치 작업으로 한다.

### 6.6 이동

- **같은 서버 안에서** 다른 InGameSpace로: 출발 InGameSpace가 플레이어를 제거하고 InGameSpaceManager를 거쳐 도착 존 스레드로 넘긴다. 플레이어 대리 객체는 이동하는 동안 **정확히 한 곳에만 존재한다**(Rust의 move). GatewayLink에 `RouteUpdate`를 보낸다.
- **다른 서버로**: 출발 쪽은 대리 객체를 제거하고, 도착 쪽은 본 샤드가 보낸 `CombatProfile`로 새로 만든다. Gateway의 존 목적지를 바꾼다(02 문서 6장).

---

## 7. 스레드 간 통신 규칙

| 방향 | 채널 | 보내는 쪽 동작 | 가득 찼을 때 |
|---|---|---|---|
| GatewayLink → 존 스레드 | `std::sync::mpsc::sync_channel` (bounded) | `try_send` (async에서 `send` 금지, 블로킹됨) | 해당 세션에 Kick 요청 + 경고 로그 |
| GatewayLink → 플레이어 액터 | `tokio::sync::mpsc::channel` (bounded) | `try_send` | 해당 세션에 Kick 요청 + 경고 로그 |
| 존 스레드, 플레이어 액터 → GatewayLink | `tokio::sync::mpsc::channel` (bounded) | `try_send` | **게이트웨이 링크 장애로 취급.** 느린 클라이언트 처리는 Gateway가 하므로 여기가 가득 차는 것은 정상 상황이 아니다. 에러 로그 + 지표 |
| 존 ↔ 플레이어 액터 (같은 서버) | 상대의 입력 채널 (액터 inbox, 존 입력 채널) | `try_send` | 에러 로그 + 지표. Reliable 성격의 메시지(`GrantItem` 등)는 보내는 쪽 outbox에서 재시도 |
| 존, 플레이어 액터 → ServerLink | `tokio::sync::mpsc::channel` (bounded) | `try_send` | 위와 같음 |
| 존, InGameSpaceManager → DB task (플레이어 액터는 DB를 직접 `.await`) | `tokio::sync::mpsc::channel` (bounded) | `try_send` | **치명 상황.** 용량을 넉넉히 잡고 사용률 감시. 가득 차면 에러 로그 + 재시도 대기 |
| GatewayLink 라우팅 갱신 | GatewayLink 제어 채널 | `try_send` (`RouteUpdate`) | 에러 로그. 이동 중 패킷은 출발 존에서 forward되므로 정합성은 유지 |

- 채널 용량은 모두 설정값이다.
- 존 스레드가 블로킹되는 유일한 지점은 자기 입력 채널의 `recv_timeout`이다. 플레이어 액터는 블로킹하지 않고 `.await`로 기다린다.
- 락(`Mutex`)은 스레드 간 통신에 쓰지 않는다. 읽기 전용 공유 데이터(기획 테이블)는 `Arc`로, 부하 지표는 원자 변수로만 공유한다.

---

## 8. 핸들 전달: ServerContext

매니저와 서비스의 핸들은 전역 싱글톤으로 두지 않고, **핸들을 모은 구조체 하나**를 생성자로 넘긴다. 명시적이고(의존성이 생성자에 보임), 확장이 쉽다(매니저가 늘면 필드 하나 추가).

```rust
#[derive(Clone)]
pub struct ServerContext {
    pub config: Arc<Config>,
    pub data_tables: Arc<DataTables>,          // 기획 데이터 (읽기 전용)
    pub space_manager: InGameSpaceManagerHandle,
    pub kv: KvHandle,                          // Redis 호환 저장소 (Directory, 서버 등록 등)
    pub db: DbHandle,
    pub zones: Arc<[ZoneSender]>,              // 존 스레드 입력 채널들
    pub contents: ContentManagerHandle,        // 컨텐츠 런타임의 ContentManager
    pub links: ServerLinksHandle,
}
```

`main`에서 한 번 만들고, 각 task와 스레드를 만들 때 `ctx.clone()`을 넘긴다. "하나만 존재"는 `main`이 한 번만 만드는 것으로 보장한다.

---

## 9. 규모 산정 (Game 서버, 10,000 동접 / 10Hz, 초기 가정)

| 항목 | 가정값 | 근거 |
|---|---|---|
| 서버 코어 | 16 | 가정. 실측 후 조정 |
| 존 스레드 Kz | 6 | 로비, 필드, 길드 던전 (던전과 PvP는 전투 서버로 빠짐) |
| 컨텐츠 런타임 워커 Kc | 2 | 계산이 적고 대부분 DB를 기다림. 기다리는 동안 워커는 다른 액터를 실행 |
| I/O 워커 | 8 | 역직렬화, 링크, DB |
| 틱 예산 | 100ms 중 **50ms 이하** | 나머지는 명령 처리와 급증 대비 |
| 활성 로비 수 | 최대 10,000 | 잠든 로비는 틱 비용 0 (4.6) |

전투 서버는 컨텐츠 런타임이 없으므로 코어를 존 스레드와 I/O 워커에 나눈다(예: 16코어 → Kz 10, I/O 6). 모든 값은 bot 크레이트로 부하 테스트한 뒤 조정한다.

---

## 10. 시작, 종료와 장애

### 10.0 시작 순서 (Game 서버)

게임 서버에서 스레드를 막는 DB 호출이 정당한 곳은 기동 단계다. 길드, 시세, 시즌 같은 공용 데이터를 준비하거나 복구하는 호출이다. 이 순서를 코드 흐름과 주석에만 맡기지 않는다. 단계 목록으로 드러내고, 모두 끝나야 손님을 받는다(**기동 배리어**).

1. `main`이 설정을 읽는다. 비밀값은 설정 파일이 아니라 환경 변수나 비밀 저장소에서 읽는다(08 문서 8장).
2. DB와 Redis에 연결하고 마이그레이션 버전을 확인한다(C 방식).
3. **공용 데이터를 의존 순서대로 읽는다**(C 방식). 순서는 코드에 단계 목록으로 적는다. 예: 기획 데이터 → 샤드 공용 상태 → 그 상태에 기대는 기능(시즌, 일정). 각 단계는 `Result`를 돌려주고, 실패를 무시하는 단계는 두지 않는다.
4. **기동 복구**: 서버가 꺼져 있는 동안 지났어야 할 일(끝난 이벤트 정산, 시즌 교체)을 처리한다. 복구 하나는 **한 트랜잭션**이거나, 일 단위마다 **멱등 키**(예: 이벤트 ID + 회차)를 갖는다. 그래야 복구 도중 죽고 다시 떠도 환급이 두 번 나가지 않는다. 이벤트 종료, 재화 환급, 연혁 기록을 따로따로 호출하면 이 보장이 없다.
5. 읽은 결과는 그 데이터를 소유할 존 스레드와 액터를 만들 때 초기값으로 넘긴다. 이 시점에 `BlockingDb`를 버린다(5.7).
6. 런타임, 존 스레드, ContentManager를 띄우고 Redis에 서버를 등록한다(03 문서 9.7).
7. `/health/ready`를 켠다(08 문서 2장). 이때부터 플레이어가 들어온다.

**준비 실패 정책**: 1~6단계 중 하나라도 실패하면 기동을 실패로 끝내고 프로세스를 종료한다. 감시자가 재시작하고, 실패 단계는 에러 로그로 남는다. "일부 기능만 닫고 서비스" 같은 부분 기동은 허용하지 않는다. 실패를 무시하는 단계가 있으면 준비가 안 된 채로 서비스에 들어갈 수 있기 때문이다.

### 10.1 정상 종료 순서 (Game 서버)

1. `main`이 종료 신호(SIGTERM, Ctrl+C)를 받는다.
2. Gateway에 종료 예고를 보낸다. Gateway는 이 서버의 존에 있는 플레이어에게 점검 안내를 보내고 이동 또는 정리를 진행한다.
3. Redis의 `server:{id}`를 `Draining`으로 바꾼다. 이 서버에 새 InGameSpace가 배치되지 않는다(03 문서 9.7).
4. 모든 존 스레드에 `ZoneCommand::Shutdown`을 보낸다. 각 InGameSpace는 최종 저장 요청을 내고 루프를 끝낸다.
5. ContentManager에 종료를 보낸다. 각 플레이어 액터는 진행 중인 명령을 끝까지 마치고 저장한 뒤 내려간다(5.5). outbox의 미전송 메시지는 DB에 남는다(재시작 후 재전송).
6. 존 스레드를 모두 `join`하고, 컨텐츠 런타임의 모든 액터가 끝날 때까지 기다린 뒤 런타임을 내린다.
7. DB task가 남은 요청을 모두 커밋할 때까지 기다린다(타임아웃 있음, 타임아웃 시 미저장 목록을 에러 로그로).
8. 서버 간 링크를 닫고 런타임을 종료한다.

종료 신호 전파에는 `CancellationToken`(I/O 런타임 쪽)과 `Shutdown` 명령(존 스레드, ContentManager 쪽)을 쓴다.

### 10.2 패닉

- release 프로필에 `panic = "abort"`를 설정한다. 패닉이 나면 즉시 프로세스를 종료하고 코어 덤프를 남기며, 감시자(systemd 등)가 재시작한다.
- 패닉 경로(`unwrap`, `panic!`)는 가이드상 금지이므로 패닉은 곧 버그다.
- 마지막 저장 이후의 InGameSpace 변경분은 유실될 수 있다. 재화와 보상은 플레이어 액터가 **DB 커밋 후에만** 반영하므로(5.5) 유실되지 않는다. 컨텐츠의 outbox는 DB에 있으므로 재시작 후 이어서 전송된다.
- 전투 서버가 죽으면 그 서버의 던전과 PvP 진행은 사라진다. 보상은 본 샤드의 플레이어 액터로 직접 전달되며, 확인 전에 죽은 경우의 처리는 03 문서 3.4를 따른다.

---

## 11. 후속 문서

| 문서 | 내용 |
|---|---|
| 05 공간 수명 관리 (**초안 작성됨**, `05-space-lifecycle.md`) | Redis Directory(`space:{key}`)의 선점, 임대 시간, fencing, InGameSpaceManager 상세, 언로드 유예 시간, MMO 필드 채널(정원 초과 시 복제), 길드 던전 생성 권한과 개수 제한 |
| 06 클러스터 | Redis 기반 서버 등록과 발견(03 문서 9.7), 스케일 인/아웃, 드레인, Redis 이중화와 장애 시 동작 |

---

## 12. 이 설계가 지키는 가이드 규칙

- 게임 상태는 한 스레드가 단독으로 소유한다. InGameSpaceManager도 액터이고, 판정은 액터가 한다. Redis에는 "누가 담당인가"만 적는다 (6.1장).
- 락 가드를 쥔 채로 `.await`하지 않는다. 존 스레드에는 `.await` 자체가 없고, 플레이어 액터는 락이 없다 (6.2장).
- async 코드에서 블로킹하지 않는다. GatewayLink는 `try_send`만 쓴다 (6.3장).
- 채널은 모두 bounded다 (6.3장).
- `game` 크레이트는 I/O를 모른다. InGameSpace는 Outbox에 결과만 쌓는다 (2장).
- DB 호출은 존 스레드에서 직접 하지 않는다. 플레이어 액터는 async라서 DB를 직접 `.await`해도 틱에 영향이 없다 (8장).
- 전역 싱글톤 대신 핸들을 생성자로 넘긴다 (14장).
