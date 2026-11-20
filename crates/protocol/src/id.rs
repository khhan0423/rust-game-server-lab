//! Identifiers and routing metadata shared by framing and packet dispatch.

macro_rules! numeric_id {
    ($(#[$doc:meta])* $name:ident, $number:ty) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name($number);

        impl $name {
            /// Wraps a number from the wire.
            pub const fn new(value: $number) -> Self { Self(value) }

            /// Returns the underlying wire value.
            pub const fn value(self) -> $number { self.0 }
        }
    };
}

numeric_id!(
    /// Client frame message ID. Unassigned values retain their routing band.
    MessageId,
    u16
);
numeric_id!(
    /// Server-only control ID in the envelope's control namespace.
    ControlId,
    u16
);
numeric_id!(
    /// Session sequence or cumulative ACK number, including unsequenced zero.
    SequenceNumber,
    u32
);
numeric_id!(
    /// Internal session number used by the server envelope.
    SessionId,
    u64
);

/// Gateway routing band.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    /// Handled by the Gateway.
    System,
    /// Routed to the home shard.
    Content,
    /// Routed to the current space host.
    Zone,
}

impl MessageId {
    /// Classifies the ID without interpreting its payload or looking up the catalog.
    pub const fn kind(self) -> MessageKind {
        match self.0 {
            0..=0x0fff => MessageKind::System,
            0x1000..=0x7fff => MessageKind::Content,
            0x8000..=0xffff => MessageKind::Zone,
        }
    }
}

/// Whether session replay preserves the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// Sequenced and kept for replay.
    Reliable,
    /// Unsequenced and not kept for replay.
    Volatile,
}

/// Valid direction for a client-facing message ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Only the client sends this message.
    ClientToServer,
    /// Only the server sends this message.
    ServerToClient,
    /// Either side sends this message.
    Both,
}

/// Catalog metadata for a client-facing message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageSpec {
    /// Wire identifier.
    pub id: MessageId,
    /// Gateway routing band.
    pub kind: MessageKind,
    /// Valid direction.
    pub direction: Direction,
    /// Session replay policy.
    pub delivery: Delivery,
}
