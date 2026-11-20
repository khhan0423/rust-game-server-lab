# 서버-클라이언트 세션 계약

[English](session-contract.md) | **한국어**

- 상태: **초안 v0.1** (2026-11-17)
- 대상: 서버(Gateway) 개발자, 클라이언트(Unity, Unreal, Godot) 네트워크 계층 개발자
- 관련 설계: `docs/design/02-session-network.md` (서버 내부 구조)

이 문서는 서버와 클라이언트가 **양쪽 모두 반드시 지켜야 하는 약속**이다. 서버 내부 구조는 02 문서가 다루고, 이 문서는 와이어 위에서 보이는 동작만 정한다. 한쪽이 이 문서와 다르게 동작하면 버그다.

표기: **반드시**(MUST), **하지 않는다**(MUST NOT), **권장**(SHOULD). 규칙 번호(C1, S1 등)는 코드 주석과 테스트 이름에서 참조한다.

---

## 1. 식별자와 키

| 이름 | 누가 만드나 | 클라가 보나 | 수명 | 용도 |
|---|---|---|---|---|
| **ConnId** | Gateway | **아니오** (서버 내부) | TCP 연결 하나 | 늦게 도착한 옛 연결의 이벤트를 걸러냄 |
| **session_key** | Gateway | **예** | `AuthOk`로 발급 ~ 세션 종료 | 재접속할 때 "같은 세션"임을 증명 |
| **PlayerId** | 서버 (DB) | 예 | 캐릭터의 수명 | 캐릭터 식별 |
| **seq / ack** | 양쪽 | 예 | 세션 하나 (세션이 바뀌면 1부터) | Reliable 메시지의 유실 복구 (02 문서 7장) |

### 1.1 session_key 규칙

- **S1.** 서버는 `session_key`를 세션을 만들 때 **한 번** 발급하고, 세션이 끝날 때까지 **바꾸지 않는다.** 재접속에 성공해도 같은 키를 쓴다.
- **S2.** `session_key`에는 SessionId와 128비트 이상의 암호학적 난수가 들어간다. 클라이언트가 추측해서 만들 수 없어야 한다.
- **C1.** 클라이언트는 `session_key`를 **`AuthOk`에서만** 받는다. 값을 해석하지 않는 불투명(opaque) 바이트로 다룬다.
- **C2.** 클라이언트는 `session_key`를 **메모리에만** 보관한다. 디스크에 저장하지 않는다. 앱 프로세스가 재시작되면 키는 사라지고, 클라이언트는 처음 접속(`Auth`) 절차로 간다.
  - 이유: 앱이 재시작되면 클라이언트의 게임 상태도 함께 사라졌으므로, 빠진 메시지만 재전송받는 방식(Replay)은 의미가 없다. 서버는 이 경우를 중복 접속으로 처리해 같은 캐릭터를 이어 주고 전체 상태를 다시 보낸다.

**키를 재접속 때마다 바꾸지 않는 이유.** 재접속할 때마다 새 키를 발급하면, 서버가 새 키를 담은 `ResumeOk`를 보낸 직후 연결이 다시 끊기는 경우가 생긴다. 그러면 클라이언트는 옛 키만 가지고 있고 서버는 옛 키를 폐기한 상태가 되어, 재접속이 영원히 실패한다. 모바일에서는 연결이 연달아 끊기는 일이 흔하므로 이 위험이 더 크다. 대신 키가 도청되지 않도록 운영 환경에서는 TLS를 쓴다(02 문서 N5).

---

## 2. 핸드셰이크 메시지

모든 핸드셰이크 메시지는 Reliable이 아니다(`seq = 0`). 핸드셰이크는 매 연결마다 새로 하기 때문이다.

### 2.1 처음 접속

| 방향 | 메시지 | 필드 |
|---|---|---|
| C → S | `Auth` | `login_token`(게임 Entry 서버가 발급, 07 문서 3.2), `protocol_version`, `client_version`, `platform` |
| S → C | `AuthOk` | `session_key`, `player_id`, `server_time`, `timers` (6장) |
| S → C | `Close` | `reason`, `reconnect_allowed` (7장). 인증 실패, 버전 불일치 등 |

### 2.2 재접속 (리로그인)

| 방향 | 메시지 | 필드 |
|---|---|---|
| C → S | `Resume` | `session_key`, `last_server_seq`, `protocol_version`, `client_version` |
| S → C | `ResumeOk` | `mode` (`Replay` / `Resync`), `last_client_seq`, `server_time`, `timers` |
| S → C | `ResumeFail` | `reason` (`Expired`, `Unknown`, `Closed`, `VersionMismatch`) |

- `last_server_seq`: 클라이언트가 **처리 완료한** 서버 Reliable 메시지의 마지막 seq.
- `last_client_seq`: 서버가 **받은** 클라이언트 Reliable 요청의 마지막 seq.
- `mode`는 MQTT 5의 CONNACK `Session Present` 플래그와 같은 역할이다. 클라이언트가 자기 로컬 상태를 계속 믿어도 되는지(`Replay`), 버리고 다시 받아야 하는지(`Resync`)를 서버가 알려 준다.

### 2.3 연결 중

| 방향 | 메시지 | 용도 |
|---|---|---|
| C → S | `Ping { client_time }` | 하트비트. `timers.heartbeat_interval`마다 |
| S → C | `Pong { client_time, server_time }` | 하트비트 응답. 지연 측정 |
| 양방향 | `Ack { ack }` | 보낼 메시지가 없을 때 ACK만 보냄 (02 문서 7.2) |
| C → S | `Suspend` | 앱이 백그라운드로 간다는 **예고** (4장) |
| C → S | `Logout` | 정상 종료 |
| S → C | `Close { reason, reconnect_allowed, message }` | 서버가 연결을 끊기 직전에 보내는 사유 |

---

## 3. 클라이언트 상태

클라이언트 네트워크 계층은 아래 상태를 가진다. 엔진(Unity, Unreal, Godot)이 달라도 상태와 전이는 같아야 한다.

```
                       Auth                AuthOk
  Offline ──connect──> Authenticating ──────────────> Online <─────────────┐
     ▲                     │  ▲                         │  │               │
     │                     │  │ ResumeFail / 유예 초과  │  │ 백그라운드    │ ResumeOk
     │        Close(재접속 │  │                         │  ▼               │
     │        불가)        │  └──────────── Reconnecting <─ Suspended      │
     │                     ▼                  │  ▲          │ 포그라운드   │
     └──────────────── Terminated             │  └─연결 끊김─┘ (필요 시)   │
                      (자동 재접속 안 함)      └──────── Resume ───────────┘
```

| 상태 | 의미 | 들고 있는 것 |
|---|---|---|
| `Offline` | 연결 없음, 세션 없음 | - |
| `Authenticating` | `Auth` 또는 `Resume`을 보내고 응답 대기 | (Resume이면) `session_key` |
| `Online` | 정상. 게임 메시지 송수신 | `session_key`, seq/ack, 미확인 Reliable 요청 |
| `Suspended` | 앱이 백그라운드. 하트비트 중지 | 위와 같음 + 백그라운드 진입 시각 |
| `Reconnecting` | 연결이 끊겨 백오프하며 `Resume` 재시도 중 | 위와 같음 + 연결이 끊긴 시각 |
| `Terminated` | 서버가 재접속 불가로 끊음. 사용자에게 사유 표시 | 종료 사유 |

---

## 4. 클라이언트 규칙

### 4.1 재접속 판단

- **C3.** 연결이 끊기면 다음 순서로 판단한다.
  1. 마지막으로 받은 `Close`의 `reconnect_allowed`가 `false`이면 → `Terminated`. **자동 재접속하지 않는다.**
  2. `session_key`가 있고, 연결이 끊긴 지 `timers.resume_grace` 이내이면 → `Resume`.
  3. 그 밖의 경우 → `Auth` (처음 접속).
- **C4.** `ResumeFail`을 받으면 `session_key`를 버리고 `Auth`로 간다.
- **C15.** `Resume`은 **어느 Gateway로 보내도 된다.** 원래 연결했던 Gateway로 돌아갈 필요가 없다. 접속 주소는 로그인 때와 같은 방식(로드밸런서 또는 게임 Entry 서버가 알려 준 주소)으로 고른다.
- **C5.** 재시도 간격은 0.5초, 1초, 2초, 4초, 8초, 그 뒤로는 10초다. 각 간격에 ±20%의 지터를 더한다. 수천 대가 같은 순간에 몰려 들어오는 것을 막기 위해서다.
- **C6.** 연결 시도는 **동시에 하나만** 한다. 연결 시도 두 개가 같은 `session_key`로 `Resume`을 보내면 서로를 밀어내기(Superseded) 때문이다.
- **C14.** 새 연결을 시도하기 전에 **옛 소켓을 먼저 닫고 버린다.** 버린 소켓에서 늦게 도착한 메시지는 `Close`를 포함해 **모두 무시한다.** 클라이언트 네트워크 계층은 "현재 연결 번호"를 들고 있고, 이벤트마다 번호를 확인한다.
  - 이유: 같은 기기가 옛 연결을 정리하지 않은 채 새 연결로 `Auth`를 보내면, 서버는 이를 중복 로그인으로 보고 옛 연결에 `Close(DuplicateLogin, reconnect_allowed=false)`를 보낸다. 이것을 받아들이면 클라이언트가 **자기 자신에게 밀려나** `Terminated`가 된다. 서버의 ConnId 규칙(02 문서 2.4)과 같은 장치다.

### 4.2 백그라운드와 포그라운드

- **C7. 백그라운드 진입 시**
  - `Suspend`를 **보내 본다**(best-effort). 응답은 기다리지 않는다. iOS에서는 백그라운드 실행 시간이 짧으므로(`beginBackgroundTask`로 늘려도 수십 초 수준) 그 안에서 보낸다.
  - 하트비트를 멈추고 `Suspended`로 간다.
  - 연결은 **직접 끊지 않는다.** 짧게 다녀오면 연결이 살아 있을 수 있기 때문이다.
- **C8. 포그라운드 복귀 시**
  - 백그라운드에 있던 시간이 `timers.heartbeat_timeout`보다 길면 → 연결이 죽었다고 보고 **바로** 연결을 버리고 `Resume`한다.
  - 짧았다면 → `Ping`을 보내고 `timers.foreground_probe`(3초) 안에 `Pong`이 오면 `Online`으로 돌아간다. 오지 않으면 연결을 버리고 `Resume`한다.
  - 두 경우 모두 연결을 **믿지 않고 확인**하는 것이 원칙이다. 백그라운드에 다녀온 연결은 겉보기에 살아 있어도 서버 쪽에서는 이미 끊겼을 수 있다(half-open).
- **C9. 엔진별 이벤트**

| 엔진 | 백그라운드 진입 | 포그라운드 복귀 | 주의 |
|---|---|---|---|
| Unity | `OnApplicationPause(true)` | `OnApplicationPause(false)` | `OnApplicationFocus`를 쓰지 않는다. Android에서 화면 키보드만 띄워도 호출된다 |
| Unreal | `FCoreDelegates::ApplicationWillEnterBackgroundDelegate` | `FCoreDelegates::ApplicationHasEnteredForegroundDelegate` | 백그라운드에서 예고 없이 종료될 수 있다. 진입 시점에 할 일을 끝낸다 |
| Godot (C#) | `NOTIFICATION_APPLICATION_PAUSED` | `NOTIFICATION_APPLICATION_RESUMED` | 모바일 플랫폼에서만 발생 |

### 4.3 재접속 후 처리

- **C10.** `ResumeOk.mode == Replay`이면, 이어서 오는 Reliable 메시지(재전송분)를 처리한 뒤 InGameSpace 스냅샷을 적용한다. 로컬 상태는 유지한다.
- **C11.** `ResumeOk.mode == Resync`이면, 로컬 게임 상태를 **버리고** 이어서 오는 전체 스냅샷으로 다시 만든다.
- **C12.** 미확인 Reliable 요청 중 `last_client_seq`보다 큰 것을 **원래 seq 그대로** 다시 보낸다. 새 seq를 매기지 않는다. 같은 seq여야 서버가 중복을 걸러낼 수 있다.
- **C13.** 이미 처리한 seq 이하의 서버 Reliable 메시지가 오면 버린다.

---

## 5. 서버 규칙

- **S3. Resume 판정.** 서버는 아래 표대로만 응답한다.

| `session_key`가 가리키는 세션 | 응답 |
|---|---|
| 없음 (모르는 키, 이미 종료) | `ResumeFail(Unknown)` 또는 `ResumeFail(Closed)` |
| 유예 만료 | `ResumeFail(Expired)` |
| `Detached` (연결 없음) | 새 연결을 결합, `ResumeOk` |
| `Active` (옛 연결이 아직 붙어 있음, half-open) | 옛 연결을 끊고(Superseded) 새 연결 결합, `ResumeOk` |
| 버전 불일치 | `ResumeFail(VersionMismatch)` 후 `Close(UpdateRequired)` |

- **S4. 모드 결정.** 재전송 버퍼에 `last_server_seq + 1`부터 빠짐없이 있으면 `Replay`, 아니면 `Resync`.
- **S5. Suspend 처리.** `Suspend`를 받으면 세션에 "일시정지" 표시를 한다.
  - Volatile 메시지 전송을 멈춘다. Reliable은 계속 버퍼에 쌓는다.
  - 이후 연결이 끊기면 유예 시간으로 `resume_grace` 대신 더 긴 `suspend_grace`를 쓴다. 사용자가 일부러 앱을 내린 것이므로 오래 돌아오지 않을 가능성이 높기 때문이다.
  - 연결이 아직 살아 있는 동안 클라이언트가 `Ping`을 보내면 표시를 해제하고 정상 전송을 재개한다.
  - MQTT 5에서 클라이언트가 연결을 끊으면서 세션 만료 시간을 바꿀 수 있는 것과 같은 발상이다.
- **S6. 끊기 전에 사유를 보낸다.** 프로토콜 위반을 제외하고, 서버가 연결을 끊을 때는 `Close`를 먼저 보내고 최대 3초 동안 송신 큐를 비운 뒤 끊는다.
- **S7. 타이머 값은 서버가 정한다.** `AuthOk`와 `ResumeOk`의 `timers`로 내려보낸다. 클라이언트는 이 값을 하드코딩하지 않는다. 운영 중에 클라이언트 업데이트 없이 조정하기 위해서다.
- **S8. 버전 검사.** `protocol_version`이 서버가 지원하는 범위를 벗어나면 `Close(UpdateRequired, reconnect_allowed = false)`.
- **S9. 결합된 연결의 `Auth`는 위반이다.** `Auth`와 `Resume`은 연결이 핸드셰이크 상태일 때만 유효하다. 이미 세션에 결합된 연결에서 `Auth`나 `Resume`이 오면 프로토콜 위반으로 보고 `Close` 없이 끊는다. 계정을 바꾸려면 `Logout` 후 새로 접속한다.
- **S10. 중복 로그인은 세션을 항상 새로 만든다.** **새 연결**로 들어온 `Auth`의 계정이 이미 온라인이면(기존 세션이 `Active` 또는 `Detached`) 중복 로그인이다.
  - 기존 세션이 `Active`이면 옛 연결에 `Close(DuplicateLogin, reconnect_allowed=false)`를 보내고 끊는다. `Detached`이면 알릴 연결이 없으므로 조용히 정리한다.
  - 새 세션은 **새 `session_key`**를 받고 seq는 **1부터** 시작한다. 옛 세션의 재전송 버퍼는 쓰지 않는다.
  - 클라이언트 입장에서는 **처음 로그인과 같은 흐름**이다. `AuthOk`를 받고 전체 스냅샷으로 상태를 만든다. 캐릭터는 서버 내부에서 그대로 인계되므로(02 문서 3.1) 있던 자리에서 이어서 플레이한다.

---

## 6. 타이머 (`timers`, 서버가 내려줌)

| 이름 | 기본값 | 의미 |
|---|---|---|
| `heartbeat_interval` | 10초 | 클라이언트 `Ping` 주기 |
| `heartbeat_timeout` | 30초 | 이 시간 동안 아무것도 못 받으면 연결이 죽은 것으로 봄 (양쪽 모두) |
| `foreground_probe` | 3초 | 포그라운드 복귀 후 `Pong`을 기다리는 시간 (C8) |
| `resume_grace` | 120초 | 연결이 끊긴 뒤 세션을 유지하는 시간 |
| `suspend_grace` | 600초 | `Suspend` 이후 끊긴 경우의 세션 유지 시간 (S5) |
| `ack_interval` | 1초 | 보낼 메시지가 없을 때 `Ack` 주기 |
| `handshake_timeout` | 10초 | 접속 후 `Auth`/`Resume`을 보내야 하는 시간 |

---

## 7. 종료 사유 (`Close.reason`)

| 사유 | `reconnect_allowed` | 클라이언트 동작 |
|---|---|---|
| `Logout` | false | 로그인 화면 |
| `DuplicateLogin` | **false** | "다른 기기에서 접속했습니다". **자동 재접속하면 두 기기가 서로를 계속 밀어낸다** |
| `Maintenance` | false | 점검 안내 |
| `UpdateRequired` | false | 스토어로 안내 |
| `Banned` | false | 제재 안내 |
| `ServerShutdown` | true | 백오프 후 `Resume` (다른 프로세스가 이어받을 수 있음) |
| `IdleTimeout` | false | 로그인 화면 |
| (프로토콜 위반) | `Close` 없음 | 일반 끊김과 같음. 정상 클라이언트에서는 생기지 않는다 |

---

## 8. 시나리오

### A. 처음 접속
```
C: connect → Auth(login_token, ver)
S: AuthOk(session_key=K, timers)          C: Online, K를 메모리에 보관
```

### B. 백그라운드에 잠깐 (연결 살아 있음)
```
C: [백그라운드] Suspend → Suspended
S: 일시정지 표시, Volatile 중지
C: [5초 후 포그라운드] Ping
S: Pong, 일시정지 해제                       C: Online (재접속 없음)
```

### C. 백그라운드에 오래 (연결 죽음, 유예 이내)
```
C: [백그라운드] Suspend → Suspended
   ... OS가 앱을 일시정지, 연결 끊김. 서버는 하트비트로 감지 → Detached (suspend_grace 600초)
C: [2분 후 포그라운드] heartbeat_timeout 초과 → 연결 버림 → connect → Resume(K, last_server_seq=120)
S: 버퍼에 121..125 있음 → ResumeOk(Replay) → seq 121..125 재전송 → InGameSpace 스냅샷
C: 재전송분 처리 → 스냅샷 적용 → Online
```

### D. 백그라운드에 아주 오래 (유예 만료)
```
C: [15분 후 포그라운드] → Resume(K, ...)
S: ResumeFail(Expired)                       (Player는 이미 저장 후 로그아웃됨)
C: K 버림 → Auth(login_token) → AuthOk(K2) → 캐릭터 다시 로드
```

### E. 앱 강제 종료 후 재실행 (유예 이내)
```
C: [재실행, K 없음] → Auth(login_token)
S: 같은 계정의 Detached 세션 발견 → 중복 접속(조용한 인수) → 옛 세션 정리
   → 새 세션 K2, Player 인계 → AuthOk(K2) → 전체 스냅샷 (Resync)
C: Online
```

### F. 다른 기기에서 로그인
```
기기2: Auth(login_token)
S: 기기1 세션이 Active → 기기1에 Close(DuplicateLogin, reconnect_allowed=false) → 정리 완료 후
   기기2에 AuthOk(K2) + 전체 스냅샷
기기1: Terminated, "다른 기기에서 접속했습니다". 재접속하지 않음
```

### G. Wi-Fi → LTE 전환
```
C: 네트워크 변경 이벤트 → 연결 버림 → Resume(K, last_server_seq)
S: 옛 연결이 아직 Active(half-open) → 옛 연결 Superseded → ResumeOk(Replay)
```

### H. 끊기기 직전에 보낸 구매 요청
```
C: BuyItem(seq=7) 전송 직후 연결 끊김. ack 못 받음 → 보관
C: Resume(K, ...) → S: ResumeOk(last_client_seq=7)   (서버는 이미 받았음)
C: seq 7은 재전송하지 않음 (7 이하)
   만약 last_client_seq=6이었다면 → BuyItem(seq=7)을 원래 seq로 재전송 → 서버가 한 번만 처리
```

---

## 9. 선례와 채택한 것

| 선례 | 가져온 것 |
|---|---|
| MQTT 5 (Clean Start, Session Expiry Interval, CONNACK Session Present) | 재개 응답에 "세션 상태가 유효한가"를 알려 주는 `mode`. 끊기기 전에 클라가 세션 유지 시간을 바꾸는 발상(`Suspend` → `suspend_grace`) |
| Colyseus (`reconnectionToken`, `allowReconnection`) | 클라가 재접속 토큰을 보관하고, 서버가 정해진 시간 동안 자리를 남겨 두는 구조 |
| Nakama (session token, refresh token) | 로그인 증명(`login_token`)과 세션 증명(`session_key`)을 분리 |
| ASP.NET Core SignalR stateful reconnect | seq/ack와 재전송 버퍼로 재접속 시 빠진 메시지를 다시 보내는 방식 |
| TLS 1.3 세션 재개 | 재개용 키는 서버만 의미를 아는 불투명 값으로 다룸. 운영에서는 TLS로 키 도청을 막음 |

## 10. 검증

- 서버 저장소의 `bot` 크레이트가 이 계약의 **참조 클라이언트 구현**이 된다. 위 8장 시나리오를 자동 테스트로 만든다.
- 엔진별 클라이언트 구현은 같은 시나리오를 통과해야 한다. 서버는 시나리오별 패킷 바이트 샘플을 제공한다.
- 이 문서를 바꾸면 `protocol_version`을 올리고, 서버와 모든 클라이언트를 함께 맞춘다.

## 출처

- MQTT 5 Clean Start와 Session Expiry: https://www.emqx.com/en/blog/mqtt5-new-feature-clean-start-and-session-expiry-interval , https://www.hivemq.com/blog/mqtt5-essentials-part4-session-and-message-expiry/
- Colyseus 재접속: https://docs.colyseus.io/sdk/connection , https://docs.colyseus.io/room/reconnection
- Nakama 세션: https://heroiclabs.com/docs/nakama/concepts/session/
- SignalR stateful reconnect: https://github.com/dotnet/aspnetcore/pull/66771
- TLS 1.3: https://datatracker.ietf.org/doc/html/rfc8446 , https://www.rfc-editor.org/rfc/rfc9846.html
- iOS 백그라운드 실행 시간: https://developer.apple.com/documentation/uikit/extending-your-app-s-background-execution-time , https://developer.apple.com/documentation/uikit/uiapplication/beginbackgroundtask(expirationhandler:)
- Unity 생명주기: https://docs.unity3d.com/6000.2/Documentation/ScriptReference/MonoBehaviour.OnApplicationPause.html , https://discussions.unity.com/t/can-somebody-explain-the-onapplicationpause-focus-scenarios/76324
- Unreal 생명주기: https://dev.epicgames.com/documentation/unreal-engine/API/Runtime/Engine/UApplicationLifecycleComponent
