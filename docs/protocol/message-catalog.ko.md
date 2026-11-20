# 파일럿 메시지 목록

[English](message-catalog.md) | **한국어**

- 상태: **초안** (2026-11-20)

## 클라이언트 프레임

[세션과 네트워크 설계](../design/02-session-network.ko.md) 7절과 8절을 따른다. 16바이트 헤더는 `len:u32`, `msg:u16`, `flags:u16`, `seq:u32`, `ack:u32` 순서이며 모두 리틀 엔디언이다. `len`에는 헤더가 포함된다. 비트 0은 FRAGMENT, 비트 1은 RELIABLE이며 다른 비트는 0이어야 한다. 이 범위를 벗어난 길이, 예약 플래그, 클라이언트가 보낸 FRAGMENT 프레임은 Close 통지 없이 연결을 닫는다. 서버가 보낸 조각은 공통 헤더 다음에 `transfer_id:u32`, `index:u16`, `count:u16`을 싣는다.

Reliable 프레임의 `seq`는 세션마다 방향별로 1부터 시작한다. Volatile 프레임의 `seq`는 0이다. `ack`는 상대가 보낸 Reliable 메시지 중 처리를 마친 마지막 번호이며 누적된다. Ack 메시지 본문의 값은 헤더와 같아야 한다.

## 메시지 ID

게이트웨이는 ID 범위만 보고 분류한다. 시스템은 `0x0000`~`0x0FFF`로 게이트웨이가 처리하고, 콘텐츠는 `0x1000`~`0x7FFF`로 홈 샤드가 처리하며, 공간은 `0x8000`~`0xFFFF`로 현재 공간의 호스트가 처리한다. EnterGame은 시스템 요청이다. 게이트웨이가 처리한 뒤 홈 샤드에 입장을 요청한다.

기능마다 32개 번호를 묶어 배정하고 빈 번호를 남긴다. 미배정 번호만 새로 쓸 수 있다. 배정한 ID는 변경하거나 재사용하지 않는다. 아래 표의 C는 클라이언트, S는 게임 서버 측을 뜻한다. 괄호 안의 숫자는 필드 번호다.

## 시스템

| ID | 메시지 | 방향 | 전달 등급 | 필드 |
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
| `0x0023` | `Suspend` | C→S | Volatile | 없음 |
| `0x0024` | `Logout` | C→S | Volatile | 없음 |
| `0x0040` | `EnterGame` | C→S | Reliable | 없음 |
| `0x0041` | `EnterGameResult` | S→C | Reliable | `status` ActionStatus (1) |

## 콘텐츠

| ID | 메시지 | 방향 | 전달 등급 | 필드 |
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

## 공간

| ID | 메시지 | 방향 | 전달 등급 | 필드 |
|---|---|---|---|---|
| `0x8000` | `EnterSpace` | C→S | Reliable | 없음 |
| `0x8001` | `EnterSpaceResult` | S→C | Reliable | `status` ActionStatus (1) |
| `0x8002` | `LeaveSpace` | C→S | Reliable | 없음 |
| `0x8003` | `LeaveSpaceResult` | S→C | Reliable | `status` ActionStatus (1) |
| `0x8020` | `Move` | C→S | Volatile | `position` Position (1) |
| `0x8021` | `SpaceSnapshot` | S→C | Volatile | `self_entity` EntityNumber (1), `entities` repeated SpaceEntity (2) |
| `0x8022` | `EntityAppeared` | S→C | Volatile | `entity` SpaceEntity (1) |
| `0x8023` | `EntityDisappeared` | S→C | Volatile | `entity` EntityNumber (1) |
| `0x8024` | `EntityMoved` | S→C | Volatile | `entity` EntityNumber (1), `position` Position (2) |

## 제어

설계 02의 4.2절에 따른 15바이트 헤더는 `len:u32`, `kind:u8`, `session:u64`, `msg:u16` 순서다. 길이는 헤더를 포함하고 여러 바이트 값은 리틀 엔디언이다. kind 0은 클라이언트 패킷 중계로, 메시지 ID와 페이로드를 그대로 싣는다. kind 1은 서버 내부 제어이며 별도의 ID 공간을 쓴다. 다른 kind는 오류다. G는 게이트웨이, B는 서버 링크의 상대다. ServerHello의 세션 번호는 0이다. 제어 메시지에는 클라이언트의 seq, ack, 재전송 규칙을 적용하지 않는다. 샤드 간 우편 배송은 별도 P6 링크 작업의 범위다.

| ID | 메시지 | 방향 | 전달 등급 | 필드 |
|---|---|---|---|---|
| `0x0001` | `ServerHello` | B↔B | N/A | `server` ServerAddress (1), `protocol_version` uint32 (2) |
| `0x0020` | `SessionAttached` | G→B | N/A | `player_id` PlayerId (1) |
| `0x0021` | `SessionBound` | B→G | N/A | `player_id` PlayerId (1), `content` ServerAddress (2), `zone` ServerAddress (3) |
| `0x0022` | `SessionDetached` | G→B | N/A | `reason` DetachReason (1) |
| `0x0023` | `SessionSuspended` | G→B | N/A | 없음 |
| `0x0024` | `SessionResumed` | G→B | N/A | `mode` ResumeMode (1), `gateway` ServerAddress (2) |
| `0x0025` | `SessionReplaced` | G→B | N/A | `gateway` ServerAddress (1), `new_session` SessionNumber (2) |
| `0x0026` | `SessionClosed` | G→B | N/A | 없음 |
| `0x0027` | `RouteUpdate` | B→B | N/A | `zone_index` uint32 (1) |
| `0x0028` | `SwitchZone` | B→G | N/A | `zone` ServerAddress (1) |
| `0x0040` | `Kick` | G→G | N/A | `reason` CloseReason (1) |
| `0x0041` | `CleanupComplete` | G→G | N/A | 없음 |
| `0x0042` | `Superseded` | G→G | N/A | 없음 |

## 공용 필드 형식

| 형식 | 필드 |
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

열거형 번호는 아래 순서대로 0부터 시작한다. Unspecified는 미지정 값이며 유효한 값이 아니다.

| 열거형 | 값 |
|---|---|
| `CloseReason` | Unspecified, Logout, DuplicateLogin, Maintenance, UpdateRequired, Banned, ServerShutdown, IdleTimeout, AuthenticationFailed |
| `ResumeMode` | Unspecified, Replay, Resync |
| `ResumeFailReason` | Unspecified, Expired, Unknown, Closed, VersionMismatch |
| `ActionStatus` | Unspecified, Ok, Rejected |
| `ServerRole` | Unspecified, Gateway, Game |
| `DetachReason` | Unspecified, PeerClosed, PeerReset, WriteFailed, KeepaliveTimeout, HeartbeatTimeout, HandshakeTimeout, SendQueueOverflow, ProtocolViolation, LocalNetwork |

## 이 문서에서 결정한 사항

- 표에서 파일럿 메시지의 ID, 방향, 전달 등급, 이름, 필드 번호, 전송 형식, 열거형 값을 확정한다. 영어 표의 ID 행으로 Rust 레지스트리를 생성하며, 빌드할 때 한국어 표의 해당 행과 비교한다.
- 클라이언트 프레임과 서버 간 봉투의 최대 크기는 각각 헤더를 포함해 65,536바이트다. 서버 간 봉투의 길이와 ID는 리틀 엔디언이다.
- 핸드셰이크, 하트비트, Close, ACK, Suspend, Logout은 순번 없는 Volatile 프레임을 쓴다. Close는 표시할 문구를 실을 수 있고 인증 거절 사유는 AuthenticationFailed다.
- 시각은 유닉스 epoch 기준 밀리초, 타이머는 초다. 클라이언트 버전과 플랫폼은 문자열이다. 세션 키와 로그인 토큰은 불투명한 바이트열이며 클라이언트가 받은 토큰에서 내부 ID를 읽을 수 없어야 한다. 토큰 보호는 발급 서버와 게이트웨이가 맡는다.
- 플레이어 태그는 표준 형식으로 보낸다. 위치는 센티미터 단위의 부호 있는 두 좌표이며, 엔티티 번호는 현재 공간에 머무는 동안에만 유효하다. 이동은 절대 위치를 보낸다. 파일럿의 EnterSpace는 자기 로비에 들어가며, LeaveSpace로 나가도 세션은 유지된다.
- 요청, 아이템 인스턴스, 우편, 내부 플레이어 ID는 16바이트 UUID v7을 쓴다. 수량은 부호 없는 정수이고 잔액은 최소 화폐 단위의 부호 있는 64비트 정수다. 성공과 거절은 공통 ActionStatus로 표시한다.
- 인벤토리 변경 통지는 스택의 현재 수량, 삭제 목록, 잔액을 보낸다. 인벤토리와 우편은 커서 없이 첫 페이지를 요청하고 응답의 커서 바로 다음 항목에서 이어서 조회한다. 다음 커서가 없으면 마지막 페이지다. 공개 태그로 우편을 보내며 우편 ID로 첨부 아이템을 받는다.
- 제어 ID는 클라이언트 ID와 별도다. 세션 교체를 제외한 세션 제어는 봉투의 세션 번호를 쓴다. 서버 주소는 샤드, 역할, 인스턴스로 구성되며 샤드와 인스턴스는 16비트 값이다. 샤드 간 배송은 게이트웨이 봉투의 범위 밖이다.
