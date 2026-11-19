//! 서버 프로세스가 공통으로 쓰는 운영 코드: 설정 로드, 로그, 지표, 관리 포트.
//!
//! 게임 규칙이나 네트워크 프로토콜은 여기에 두지 않는다. 다른 워크스페이스 크레이트에 의존하지 않는다.

pub mod admin;
pub mod config;
pub mod logging;
pub mod process;
