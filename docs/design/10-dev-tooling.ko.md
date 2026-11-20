# 설계 10. 개발 도구: 유닛 테스트, 테스트 클라이언트, 서버 런처

- 상태: **초안** (2026-11-19). 소유자가 정한 다음 작업이다: "유닛 테스트 준비, 테스트 클라이언트 준비, 서버 런처 준비"
- 관련 문서: `docs/plan/README.md`(실행 계획: 파일럿 단계, 개발 단계, 테스트 시나리오), `docs/RUST_GUIDELINES.md` 2장(워크스페이스), 11장(테스트), 13장(크레이트), `docs/protocol/session-contract.md`(테스트 클라이언트가 지킬 약속), `08-operations-observability.md`(헬스 체크, 로그)

## 0. 결정 요약

| # | 결정 | 내용 |
|---|---|---|
| T1 | 순서 | ① 워크스페이스 뼈대와 유닛 테스트 → ② 서버 런처 → ③ 테스트 클라이언트. 테스트 클라이언트는 접속할 서버가 있어야 의미가 있으므로 마지막이다 |
| T2 | 품질 게이트 | 모든 작업은 `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`를 통과해야 끝난다 (AGENTS.md) |
| T3 | 개발용 DB와 Valkey | **Rancher Desktop**(컨테이너 엔진 dockerd/moby)으로 띄운다. Docker CLI와 `docker compose`를 그대로 쓴다. Docker Desktop에서 옮겨 온다 (소유자 결정, 2026-11-19) |
| T4 | 테스트 클라이언트 모양 | **창이 있는 Windows GUI 프로그램** (소유자 결정). 봇 여러 개를 돌리는 시나리오·부하 모드는 창 없이도 돌 수 있게 둔다(자동 테스트용) |
| T5 | 서버 런처 형태 | **Rust로 만든 GUI 프로그램** (소유자 결정). `crates/launcher` 추가 |
| T6 | GUI와 그래픽 | **테스트 클라이언트: Bevy(0.20) + bevy_egui.** Rust에서 가장 앞선 그래픽 엔진이다(wgpu 기반, 실시간 패스 트레이싱까지 지원). 필드, 플레이어, NPC, AOI 칸을 2D/3D로 그리고, 패널(로그인, 인벤토리, 패킷 로그)은 egui로 올린다. **서버 런처: eframe(egui, wgpu 렌더러).** 상태판, 로그 창, 그래프 같은 도구 화면에 가장 강하다. 두 프로그램이 같은 egui 위젯 코드를 공유한다. egui 기본 글꼴에는 한글이 없으므로 한글 글꼴을 넣는다 (소유자 결정: "가장 향상된 GUI나 그래픽으로", 2026-11-19) |
| T7 | 크레이트 구성 | `crates/launcher`(eframe GUI), `crates/client`(Bevy GUI 테스트 클라이언트), `crates/bot`(창 없는 시나리오·부하 봇). 의존 방향: `client -> net -> protocol`, `bot -> net`, `launcher`는 게임 크레이트에 의존하지 않고 프로세스와 `/health`만 다룬다. GUI 크레이트는 이 도구 크레이트에서만 쓰고 서버 바이너리에는 넣지 않는다 |

---

## 1. 지금 상태

저장소에는 설계 문서와 도구 설정(`rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`)만 있고 Rust 코드는 없다. 그래서 세 가지 모두 **워크스페이스 뼈대**부터 시작한다.

## 2. 워크스페이스 뼈대와 유닛 테스트

### 2.1 만드는 것

가이드 2장의 구조를 따르되, 크레이트는 처음 필요한 단계에서 만든다(빈 크레이트를 미리 만들지 않는다). 처음에는 `infra`, `db`, `server`, `gateway`만 만든다.

```
Server/
├─ Cargo.toml        [workspace], [workspace.dependencies], [workspace.lints] (가이드 부록 A)
├─ crates/
│  ├─ protocol/      패킷 정의 (prost)
│  ├─ game/          게임 규칙 (I/O 없음)
│  ├─ db/            sqlx 쿼리
│  ├─ net/           프레이밍 코덱, 링크
│  ├─ gateway/       Gateway 바이너리
│  ├─ server/        Game/전투/Entry 등 역할별 바이너리
│  └─ bot/           테스트 클라이언트
└─ migrations/
```

뼈대가 생기면 품질 게이트 세 줄이 처음으로 실제로 돌아간다.

### 2.2 테스트 종류

| 종류 | 대상 | 방법 | 외부 의존 |
|---|---|---|---|
| 단위 테스트 | `protocol`, `game` (순수 로직) | 같은 파일의 `#[cfg(test)] mod tests`. 인코딩 왕복, 경계값 | 없음 |
| 액터 테스트 | 플레이어 액터, 존 로직 | 채널로 명령을 넣고 결과 메시지를 확인. 시간은 `tokio::time::pause()`로 가상 시간 (가이드 11장) | 없음 |
| DB 통합 테스트 | `db` 크레이트의 함수 | `sqlx::test`가 테스트마다 빈 DB를 만들고 마이그레이션을 적용한 뒤 실행 | **PostgreSQL 필요** |
| 서버 통합 테스트 | 여러 프로세스가 함께 도는 흐름 | 런처로 띄우고 테스트 클라이언트 시나리오로 확인 (3, 4장) | PostgreSQL, Valkey |

- 테스트에서 `sleep`으로 타이밍을 맞추지 않는다. 이벤트나 채널을 기다린다.
- DB 통합 테스트는 실제 PostgreSQL을 쓴다. 가짜 DB로 바꾸면 트랜잭션, 제약 조건, 멱등 키 같은 핵심 동작을 검증할 수 없기 때문이다.

## 3. 서버 런처

### 3.1 하는 일

개발 PC에서 서버군 전체를 명령 하나로 띄우고 내린다. 운영 환경은 systemd가 맡으므로(08 문서 M8) 런처는 **개발과 테스트용**이다.

```
launcher up              topology.toml에 적힌 서버들을 순서대로 띄움
                         (Valkey/PostgreSQL 확인 → Entry → Game 서버들 → 전투 서버 → Gateway)
                         각 서버가 표준 출력에 "ready admin=<주소>" 한 줄을 쓸 때까지 기다린 뒤 다음 서버
launcher down            역순으로 종료 신호 → 정상 종료 대기 → 시간 초과면 강제 종료
launcher restart game-2  서버 하나만 재시작 (장애 시험)
launcher kill game-2     서버 하나를 강제 종료 (장애 시험: 정상 종료 없이)
launcher status          각 서버의 상태, 포트, PID
launcher logs game-1     그 서버의 로그 파일을 이어서 보기
```

### 3.2 토폴로지 파일 (예)

```toml
# topology.toml (개발용, 비밀값 없음)
[[server]]
name = "entry"
role = "entry"
port = 7000

[[server]]
name = "game-1"
role = "game"
shard = 1

[[server]]
name = "game-2"
role = "game"
shard = 2

[[server]]
name = "gateway"
role = "gateway"
port = 7100
```

- DB 접속 정보 같은 비밀값은 이 파일에 넣지 않는다. 환경 변수로 넘긴다(08 문서 8장).
- 샤드 수, 서버 수를 바꿔 가며 띄울 수 있어서 "샤드 3개, 유저 3명" 같은 시나리오를 그대로 재현할 수 있다.

## 4. 테스트 클라이언트 (`bot` 크레이트)

### 4.1 하는 일

- **세션 계약의 참조 구현이다.** 접속, 인증, 재접속, 끊김 복구를 세션 계약 문서대로 한다. 클라이언트(Unity 등) 개발자가 "서버가 정말 이렇게 동작하나"를 이것으로 확인한다.
- 두 가지 모드를 둔다.

| 모드 | 용도 | 모양 |
|---|---|---|
| 대화형 | 개발자가 패킷을 하나씩 보내 보며 확인 | 텍스트 명령창. `login`, `enter`, `buy 1001`, `move 10 20`, `disconnect` 같은 명령 |
| 시나리오 | 같은 흐름을 자동 반복, 여러 봇 동시 실행, 부하 테스트 | 시나리오를 Rust 코드로 작성하고 봇 N개로 실행. 결과는 지연 시간 분포(`hdrhistogram`)와 실패 수 |

### 4.2 첫 시나리오

승인된 샤드 시뮬레이션(04 문서 2장)을 그대로 시나리오로 만든다.

1. 봇 3개가 처음 접속 → 서로 다른 샤드에 배정되는지 확인
2. 각자 아이템 구매 → 각자의 Game DB에 저장되는지 확인
3. 봇1이 봇2에게 우편 → 봇2의 우편함에 한 번만 들어오는지 확인 (메시지를 일부러 두 번 보내 멱등성도 확인)
4. 봇2의 Game 서버를 `launcher kill` → 재시작 후 우편이 사라지지 않았는지 확인

## 5. 각 단계의 완료 기준

| 단계 | 완료 기준 |
|---|---|
| ① 뼈대와 유닛 테스트 | **완료 (2026-11-19).** 크레이트 4개(`infra`, `db`, `server`, `gateway`), 품질 게이트 통과, 실제 PostgreSQL에서 DB 테스트와 서버 기동 테스트 통과 |
| ② 서버 런처 | `launcher up/down/status/restart/kill/logs`가 동작. 서버가 아직 껍데기여도 헬스 체크까지 확인 |
| ③ 테스트 클라이언트 | 대화형으로 접속과 인증, 시나리오 모드로 봇 N개 접속과 지연 분포 출력 |

## 6. 결정할 것

| # | 질문 | 선택지 | 권장 |
|---|---|---|---|
| Q1 | 개발용 컨테이너 환경 | 확정: Rancher Desktop (T3) | - |
| Q2 | 테스트 클라이언트 모양 | 확정: Windows GUI (T4) | - |
| Q3 | 서버 런처 형태 | 확정: Rust GUI (T5) | - |
| Q4 | GUI 라이브러리 | 확정: T6 | - |
| Q5 | 테스트 클라이언트 언어 | 확정: Rust (Bevy) | - |
