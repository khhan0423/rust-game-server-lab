//! Dispatch by direction and message ID, independent of transport frames.

use prost::Message as _;

use crate::{
    ControlId, Delivery, Direction, MessageId, MessageKind, MessageSpec, ProtocolError, wire,
};

macro_rules! packets {
    ($packet:ident, $id:ident; $($name:ident = $value:literal,)*) => {
        #[doc = concat!("Typed payloads carried by ", stringify!($packet), ".")]
        #[derive(Debug, Clone, PartialEq)]
        pub enum $packet {
            $(#[doc = concat!(stringify!($name), " payload.")]
            $name(wire::$name),)*
        }

        impl $packet {
            /// Returns the ID to put in the frame or envelope header.
            pub const fn id(&self) -> $id {
                match self { $(Self::$name(_) => $id::new($value),)* }
            }

            /// Serializes the payload, without a frame or envelope header.
            pub fn encode(&self) -> Vec<u8> {
                match self { $(Self::$name(payload) => payload.encode_to_vec(),)* }
            }

            /// Parses a payload in this packet's direction and namespace.
            ///
            /// # Errors
            /// Returns an error if the ID is unknown here or the payload does not decode.
            pub fn decode(id: $id, bytes: &[u8]) -> Result<Self, ProtocolError> {
                match id.value() {
                    $($value => Ok(Self::$name(wire::$name::decode(bytes)?)),)*
                    value => Err(ProtocolError::$id(value)),
                }
            }
        }
    };
}

macro_rules! message_ids {
    ($($name:ident = $id:literal, $kind:ident, $dir:ident, $class:ident;)*) => {
        impl MessageId {
            $(#[doc = concat!("Wire ID for ", stringify!($name), ".")]
            pub const $name: Self = Self::new($id);)*

            /// Returns the declared delivery class for an assigned ID.
            pub const fn delivery(self) -> Option<Delivery> {
                match self.value() {
                    $($id => Some(Delivery::$class),)*
                    _ => None,
                }
            }
        }

        /// All assigned client-facing IDs, with their catalog metadata.
        pub const MESSAGE_SPECS: &[MessageSpec] = &[
            $(MessageSpec { id: MessageId::$name, kind: MessageKind::$kind, direction: Direction::$dir, delivery: Delivery::$class },)*
        ];
    };
}

include!(concat!(env!("OUT_DIR"), "/registry.rs"));

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn id_ranges_classify_boundaries() {
        for (raw, expected) in [
            (0x0fff, MessageKind::System),
            (0x1000, MessageKind::Content),
            (0x7fff, MessageKind::Content),
            (0x8000, MessageKind::Zone),
        ] {
            assert_eq!(MessageId::new(raw).kind(), expected, "{raw:#06x}");
        }
    }

    #[test]
    fn catalog_assigns_unique_ids_in_the_declared_bands() {
        let mut seen = HashSet::new();
        for spec in MESSAGE_SPECS {
            assert!(seen.insert(spec.id), "duplicate {:?}", spec.id);
            assert_eq!(spec.id.kind(), spec.kind, "{:?}", spec.id);
        }
    }

    #[test]
    fn each_packet_round_trips_in_its_direction() {
        macro_rules! check {
            ($ty:ident, $id:ident; $($message:expr),+ $(,)?) => {
                for packet in [$($message),+] {
                    assert_eq!($ty::decode(packet.id(), &packet.encode()).unwrap(), packet);
                }
            };
        }
        check!(ClientPacket, MessageId;
            ClientPacket::Auth(wire::Auth { login_token: vec![1, 2, 3, 4, 5, 6, 0x70, 8, 0x80, 10, 11, 12, 13, 14, 15, 16], protocol_version: 13, client_version: String::from("K7QM2XRP4"), platform: String::from("K7QM2XRP4") }),
            ClientPacket::Resume(wire::Resume { session_key: vec![1, 2, 3, 4, 5, 6, 0x70, 8, 0x80, 10, 11, 12, 13, 14, 15, 16], last_server_seq: Some(wire::Sequence { value: 13 }), protocol_version: 13, client_version: String::from("K7QM2XRP4") }),
            ClientPacket::Ping(wire::Ping { client_time: 95 }),
            ClientPacket::Ack(wire::Ack { ack: Some(wire::Sequence { value: 13 }) }),
            ClientPacket::Suspend(wire::Suspend {  }),
            ClientPacket::Logout(wire::Logout {  }),
            ClientPacket::EnterGame(wire::EnterGame {  }),
            ClientPacket::BuyItem(wire::BuyItem { request_id: Some(wire::RequestId { value: vec![1, 2, 3, 4, 5, 6, 0x70, 8, 0x80, 10, 11, 12, 13, 14, 15, 16] }), item_code: 13, count: 13 }),
            ClientPacket::InventoryList(wire::InventoryList { after: Some(wire::ItemInstanceId { value: vec![1, 2, 3, 4, 5, 6, 0x70, 8, 0x80, 10, 11, 12, 13, 14, 15, 16] }) }),
            ClientPacket::SendMail(wire::SendMail { request_id: Some(wire::RequestId { value: vec![1, 2, 3, 4, 5, 6, 0x70, 8, 0x80, 10, 11, 12, 13, 14, 15, 16] }), recipient_tag: String::from("K7QM2XRP4"), item_id: Some(wire::ItemInstanceId { value: vec![1, 2, 3, 4, 5, 6, 0x70, 8, 0x80, 10, 11, 12, 13, 14, 15, 16] }), count: 13 }),
            ClientPacket::ListMail(wire::ListMail { after: Some(wire::MailId { value: vec![1, 2, 3, 4, 5, 6, 0x70, 8, 0x80, 10, 11, 12, 13, 14, 15, 16] }) }),
            ClientPacket::ReceiveMail(wire::ReceiveMail { request_id: Some(wire::RequestId { value: vec![1, 2, 3, 4, 5, 6, 0x70, 8, 0x80, 10, 11, 12, 13, 14, 15, 16] }), mail_id: Some(wire::MailId { value: vec![1, 2, 3, 4, 5, 6, 0x70, 8, 0x80, 10, 11, 12, 13, 14, 15, 16] }) }),
            ClientPacket::EnterSpace(wire::EnterSpace {  }),
            ClientPacket::LeaveSpace(wire::LeaveSpace {  }),
            ClientPacket::Move(wire::Move { position: Some(wire::Position { x: -32, y: 62 }) }),
        );
        check!(ServerPacket, MessageId;
            ServerPacket::AuthOk(wire::AuthOk { session_key: vec![1, 2, 3, 4, 5, 6, 0x70, 8, 0x80, 10, 11, 12, 13, 14, 15, 16], player_tag: String::from("K7QM2XRP4"), nickname: String::from("K7QM2XRP4"), server_time: 95, timers: Some(wire::Timers { heartbeat_interval: 13, heartbeat_timeout: 13, foreground_probe: 13, resume_grace: 13, suspend_grace: 13, ack_interval: 13, handshake_timeout: 13 }) }),
            ServerPacket::Close(wire::Close { reason: wire::CloseReason::Logout.into(), reconnect_allowed: true, message: Some(String::from("notice")) }),
            ServerPacket::ResumeOk(wire::ResumeOk { mode: wire::ResumeMode::Replay.into(), last_client_seq: Some(wire::Sequence { value: 13 }), server_time: 95, timers: Some(wire::Timers { heartbeat_interval: 13, heartbeat_timeout: 13, foreground_probe: 13, resume_grace: 13, suspend_grace: 13, ack_interval: 13, handshake_timeout: 13 }) }),
            ServerPacket::ResumeFail(wire::ResumeFail { reason: wire::ResumeFailReason::Expired.into() }),
            ServerPacket::Pong(wire::Pong { client_time: 95, server_time: 95 }),
            ServerPacket::Ack(wire::Ack { ack: Some(wire::Sequence { value: 13 }) }),
            ServerPacket::EnterGameResult(wire::EnterGameResult { status: wire::ActionStatus::Ok.into() }),
            ServerPacket::BuyItemResult(wire::BuyItemResult { request_id: Some(wire::RequestId { value: vec![1, 2, 3, 4, 5, 6, 0x70, 8, 0x80, 10, 11, 12, 13, 14, 15, 16] }), status: wire::ActionStatus::Ok.into() }),
            ServerPacket::InventoryChanged(wire::InventoryChanged { upserted: vec![wire::ItemStack { item_id: Some(wire::ItemInstanceId { value: vec![1, 2, 3, 4, 5, 6, 0x70, 8, 0x80, 10, 11, 12, 13, 14, 15, 16] }), item_code: 13, count: 13 }], removed: vec![wire::ItemInstanceId { value: vec![1, 2, 3, 4, 5, 6, 0x70, 8, 0x80, 10, 11, 12, 13, 14, 15, 16] }], balance: Some(wire::Money { value: 17 }) }),
            ServerPacket::InventoryPage(wire::InventoryPage { items: vec![wire::ItemStack { item_id: Some(wire::ItemInstanceId { value: vec![1, 2, 3, 4, 5, 6, 0x70, 8, 0x80, 10, 11, 12, 13, 14, 15, 16] }), item_code: 13, count: 13 }], balance: Some(wire::Money { value: 17 }), next: Some(wire::ItemInstanceId { value: vec![1, 2, 3, 4, 5, 6, 0x70, 8, 0x80, 10, 11, 12, 13, 14, 15, 16] }) }),
            ServerPacket::SendMailResult(wire::SendMailResult { request_id: Some(wire::RequestId { value: vec![1, 2, 3, 4, 5, 6, 0x70, 8, 0x80, 10, 11, 12, 13, 14, 15, 16] }), status: wire::ActionStatus::Ok.into() }),
            ServerPacket::MailPage(wire::MailPage { entries: vec![wire::MailEntry { mail_id: Some(wire::MailId { value: vec![1, 2, 3, 4, 5, 6, 0x70, 8, 0x80, 10, 11, 12, 13, 14, 15, 16] }), sender_tag: String::from("K7QM2XRP4"), sender_nickname: String::from("K7QM2XRP4"), item_code: 13, count: 13 }], next: Some(wire::MailId { value: vec![1, 2, 3, 4, 5, 6, 0x70, 8, 0x80, 10, 11, 12, 13, 14, 15, 16] }) }),
            ServerPacket::ReceiveMailResult(wire::ReceiveMailResult { request_id: Some(wire::RequestId { value: vec![1, 2, 3, 4, 5, 6, 0x70, 8, 0x80, 10, 11, 12, 13, 14, 15, 16] }), status: wire::ActionStatus::Ok.into() }),
            ServerPacket::MailArrived(wire::MailArrived { entry: Some(wire::MailEntry { mail_id: Some(wire::MailId { value: vec![1, 2, 3, 4, 5, 6, 0x70, 8, 0x80, 10, 11, 12, 13, 14, 15, 16] }), sender_tag: String::from("K7QM2XRP4"), sender_nickname: String::from("K7QM2XRP4"), item_code: 13, count: 13 }) }),
            ServerPacket::EnterSpaceResult(wire::EnterSpaceResult { status: wire::ActionStatus::Ok.into() }),
            ServerPacket::LeaveSpaceResult(wire::LeaveSpaceResult { status: wire::ActionStatus::Ok.into() }),
            ServerPacket::SpaceSnapshot(wire::SpaceSnapshot { self_entity: Some(wire::EntityNumber { value: 13 }), entities: vec![wire::SpaceEntity { number: Some(wire::EntityNumber { value: 13 }), player_tag: String::from("K7QM2XRP4"), nickname: String::from("K7QM2XRP4"), position: Some(wire::Position { x: -32, y: 62 }) }] }),
            ServerPacket::EntityAppeared(wire::EntityAppeared { entity: Some(wire::SpaceEntity { number: Some(wire::EntityNumber { value: 13 }), player_tag: String::from("K7QM2XRP4"), nickname: String::from("K7QM2XRP4"), position: Some(wire::Position { x: -32, y: 62 }) }) }),
            ServerPacket::EntityDisappeared(wire::EntityDisappeared { entity: Some(wire::EntityNumber { value: 13 }) }),
            ServerPacket::EntityMoved(wire::EntityMoved { entity: Some(wire::EntityNumber { value: 13 }), position: Some(wire::Position { x: -32, y: 62 }) }),
        );
        check!(ControlPacket, ControlId;
            ControlPacket::ServerHello(wire::ServerHello { server: Some(wire::ServerAddress { shard: 13, role: wire::ServerRole::Gateway.into(), instance: 13 }), protocol_version: 13 }),
            ControlPacket::SessionAttached(wire::SessionAttached { player_id: Some(wire::PlayerId { value: vec![1, 2, 3, 4, 5, 6, 0x70, 8, 0x80, 10, 11, 12, 13, 14, 15, 16] }) }),
            ControlPacket::SessionBound(wire::SessionBound { player_id: Some(wire::PlayerId { value: vec![1, 2, 3, 4, 5, 6, 0x70, 8, 0x80, 10, 11, 12, 13, 14, 15, 16] }), content: Some(wire::ServerAddress { shard: 13, role: wire::ServerRole::Gateway.into(), instance: 13 }), zone: Some(wire::ServerAddress { shard: 13, role: wire::ServerRole::Gateway.into(), instance: 13 }) }),
            ControlPacket::SessionDetached(wire::SessionDetached { reason: wire::DetachReason::PeerClosed.into() }),
            ControlPacket::SessionSuspended(wire::SessionSuspended {  }),
            ControlPacket::SessionResumed(wire::SessionResumed { mode: wire::ResumeMode::Replay.into(), gateway: Some(wire::ServerAddress { shard: 13, role: wire::ServerRole::Gateway.into(), instance: 13 }) }),
            ControlPacket::SessionReplaced(wire::SessionReplaced { gateway: Some(wire::ServerAddress { shard: 13, role: wire::ServerRole::Gateway.into(), instance: 13 }), new_session: Some(wire::SessionNumber { value: 95 }) }),
            ControlPacket::SessionClosed(wire::SessionClosed {  }),
            ControlPacket::RouteUpdate(wire::RouteUpdate { zone_index: 13 }),
            ControlPacket::SwitchZone(wire::SwitchZone { zone: Some(wire::ServerAddress { shard: 13, role: wire::ServerRole::Gateway.into(), instance: 13 }) }),
            ControlPacket::Kick(wire::Kick { reason: wire::CloseReason::Logout.into() }),
            ControlPacket::CleanupComplete(wire::CleanupComplete {  }),
            ControlPacket::Superseded(wire::Superseded {  }),
        );
    }

    #[test]
    fn malformed_payload_and_wrong_direction_are_rejected() {
        assert!(matches!(
            ClientPacket::decode(MessageId::AUTH, &[0x0a, 0x04, 0x03]),
            Err(ProtocolError::Decode(_))
        ));
        assert!(matches!(
            ClientPacket::decode(MessageId::AUTH_OK, &[]),
            Err(ProtocolError::MessageId(2))
        ));
    }
}
