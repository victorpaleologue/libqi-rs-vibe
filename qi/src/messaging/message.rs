use crate::messaging::{
    format,
    value::{object, service, KeyDynValueMap},
};
use crate::value::Dynamic;
use bytes::Bytes;

#[derive(
    Default,
    Debug,
    Hash,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Clone,
    Copy,
    derive_more::From,
    derive_more::Into,
    derive_more::Display,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(transparent)]
pub struct Id(pub u32);

#[derive(
    Default,
    Debug,
    Hash,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Clone,
    Copy,
    derive_more::Display,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct Version(pub u16);

#[derive(
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Debug,
    Hash,
    derive_more::Display,
    serde::Serialize,
    serde::Deserialize,
)]
pub enum Type {
    #[default]
    #[display("call")]
    Call,
    #[display("reply")]
    Reply,
    #[display("error")]
    Error,
    #[display("post")]
    Post,
    #[display("event")]
    Event,
    #[display("capabilities")]
    Capabilities,
    #[display("cancel")]
    Cancel,
    #[display("canceled")]
    Canceled,
}

impl Type {
    pub const DEFAULT: Self = Self::Call;
}

/// Flags of a message.
///
/// Flags qualify the payload of a message. They are represented as one byte in the message header.
#[derive(
    Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct Flags(u8);

impl Flags {
    /// No flags.
    pub const NONE: Self = Self(0);

    /// The payload is a dynamic value (of signature `m`) instead of the value of the expected
    /// type. For calls, posts and events, the dynamic value contains a tuple of the arguments.
    pub const DYNAMIC_PAYLOAD: Self = Self(1);

    /// The payload of the call also contains a string, following the arguments, that is the
    /// signature of the type the return value must be converted to. The reply to the call is
    /// then sent with the [`DYNAMIC_PAYLOAD`](Self::DYNAMIC_PAYLOAD) flag.
    pub const RETURN_TYPE: Self = Self(2);

    const ALL: Self = Self(Self::DYNAMIC_PAYLOAD.0 | Self::RETURN_TYPE.0);

    /// Constructs empty flags.
    pub const fn empty() -> Self {
        Self::NONE
    }

    /// Constructs flags from their bits representation, if all bits are known flags.
    pub const fn from_bits(bits: u8) -> Option<Self> {
        if bits & !Self::ALL.0 == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    /// The bits representation of the flags.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Returns true if no flag is set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Returns true if all the flags of `other` are set in `self`.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Returns the union of both flags.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Returns true if the `DYNAMIC_PAYLOAD` flag is set.
    pub const fn is_dynamic_payload(self) -> bool {
        self.contains(Self::DYNAMIC_PAYLOAD)
    }

    /// Returns true if the `RETURN_TYPE` flag is set.
    pub const fn has_return_type(self) -> bool {
        self.contains(Self::RETURN_TYPE)
    }
}

impl std::ops::BitOr for Flags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl std::ops::BitOrAssign for Flags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl std::ops::BitAnd for Flags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

impl std::fmt::Debug for Flags {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut set = f.debug_set();
        if self.is_dynamic_payload() {
            set.entry(&"DYNAMIC_PAYLOAD");
        }
        if self.has_return_type() {
            set.entry(&"RETURN_TYPE");
        }
        set.finish()
    }
}

impl std::fmt::Display for Flags {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#04b}", self.0)
    }
}

#[derive(
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    derive_more::Display,
    serde::Serialize,
    serde::Deserialize,
)]
#[display("{{{_0}.{_1}.{_2}}}")]
pub struct Address(pub service::Id, pub object::Id, pub object::ActionId);

impl Address {
    pub const fn service(&self) -> service::Id {
        self.0
    }

    pub const fn with_service(&self, service: service::Id) -> Self {
        Self(service, self.1, self.2)
    }

    pub const fn object(&self) -> object::Id {
        self.1
    }

    pub const fn with_object(&self, object: object::Id) -> Self {
        Self(self.0, object, self.2)
    }

    pub const fn action(&self) -> object::ActionId {
        self.2
    }

    pub const fn with_action(&self, action: object::ActionId) -> Self {
        Self(self.0, self.1, action)
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Call {
        id: Id,
        address: Address,
        payload: Bytes,
        flags: Flags,
    },
    Reply {
        id: Id,
        address: Address,
        payload: Bytes,
        flags: Flags,
    },
    Error {
        id: Id,
        address: Address,
        error: String,
    },
    Post {
        id: Id,
        address: Address,
        payload: Bytes,
        flags: Flags,
    },
    Event {
        id: Id,
        address: Address,
        payload: Bytes,
        flags: Flags,
    },
    Capabilities {
        id: Id,
        address: Address,
        capabilities: KeyDynValueMap,
    },
    Cancel {
        id: Id,
        address: Address,
        call_id: Id,
    },
    Canceled {
        id: Id,
        address: Address,
    },
}

impl Default for Message {
    fn default() -> Self {
        Self::Call {
            id: Default::default(),
            address: Default::default(),
            payload: Default::default(),
            flags: Default::default(),
        }
    }
}

impl Message {
    /// The identifier of the message.
    pub fn id(&self) -> Id {
        match self {
            Message::Call { id, .. }
            | Message::Reply { id, .. }
            | Message::Error { id, .. }
            | Message::Post { id, .. }
            | Message::Event { id, .. }
            | Message::Capabilities { id, .. }
            | Message::Cancel { id, .. }
            | Message::Canceled { id, .. } => *id,
        }
    }

    /// The address of the message.
    pub fn address(&self) -> Address {
        match self {
            Message::Call { address, .. }
            | Message::Reply { address, .. }
            | Message::Error { address, .. }
            | Message::Post { address, .. }
            | Message::Event { address, .. }
            | Message::Capabilities { address, .. }
            | Message::Cancel { address, .. }
            | Message::Canceled { address, .. } => *address,
        }
    }

    /// The type of the message.
    pub fn ty(&self) -> Type {
        match self {
            Message::Call { .. } => Type::Call,
            Message::Reply { .. } => Type::Reply,
            Message::Error { .. } => Type::Error,
            Message::Post { .. } => Type::Post,
            Message::Event { .. } => Type::Event,
            Message::Capabilities { .. } => Type::Capabilities,
            Message::Cancel { .. } => Type::Cancel,
            Message::Canceled { .. } => Type::Canceled,
        }
    }

    pub(crate) fn into_parts(self) -> Result<(MetaData, Bytes), format::Error> {
        match self {
            Message::Call {
                id,
                address,
                payload,
                flags,
            } => Ok((
                MetaData {
                    id,
                    address,
                    ty: Type::Call,
                    flags,
                },
                payload,
            )),
            Message::Reply {
                id,
                address,
                payload,
                flags,
            } => Ok((
                MetaData {
                    id,
                    address,
                    ty: Type::Reply,
                    flags,
                },
                payload,
            )),
            Message::Error { id, address, error } => Ok((
                MetaData {
                    id,
                    address,
                    ty: Type::Error,
                    flags: Flags::NONE,
                },
                format::to_bytes(&Dynamic(error))?,
            )),
            Message::Post {
                id,
                address,
                payload,
                flags,
            } => Ok((
                MetaData {
                    id,
                    address,
                    ty: Type::Post,
                    flags,
                },
                payload,
            )),
            Message::Event {
                id,
                address,
                payload,
                flags,
            } => Ok((
                MetaData {
                    id,
                    address,
                    ty: Type::Event,
                    flags,
                },
                payload,
            )),
            Message::Capabilities {
                id,
                address,
                capabilities,
            } => Ok((
                MetaData {
                    id,
                    address,
                    ty: Type::Capabilities,
                    flags: Flags::NONE,
                },
                format::to_bytes(&capabilities)?,
            )),
            Message::Cancel {
                id,
                address,
                call_id,
            } => Ok((
                MetaData {
                    id,
                    address,
                    ty: Type::Cancel,
                    flags: Flags::NONE,
                },
                format::to_bytes(&call_id)?,
            )),
            Message::Canceled { id, address } => Ok((
                MetaData {
                    id,
                    address,
                    ty: Type::Canceled,
                    flags: Flags::NONE,
                },
                format::to_bytes(&())?,
            )),
        }
    }

    pub(crate) fn from_parts(meta: MetaData, payload: Bytes) -> Result<Self, format::Error> {
        let MetaData {
            id,
            address,
            ty,
            flags,
        } = meta;
        let msg = match ty {
            Type::Call => Self::Call {
                id,
                address,
                payload,
                flags,
            },
            Type::Reply => Self::Reply {
                id,
                address,
                payload,
                flags,
            },
            Type::Error => Self::Error {
                id,
                address,
                error: format::from_slice::<Dynamic<String>>(&payload)?.into_inner(),
            },
            Type::Post => Self::Post {
                id,
                address,
                payload,
                flags,
            },
            Type::Event => Self::Event {
                id,
                address,
                payload,
                flags,
            },
            Type::Capabilities => Self::Capabilities {
                id,
                address,
                capabilities: format::from_slice(&payload)?,
            },
            Type::Cancel => Self::Cancel {
                id,
                address,
                call_id: format::from_slice(&payload)?,
            },
            Type::Canceled => Self::Canceled { id, address },
        };
        Ok(msg)
    }
}

#[derive(
    Default,
    Copy,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Debug,
    Hash,
    derive_more::Display,
    serde::Serialize,
    serde::Deserialize,
)]
#[display("{id}:{ty}@{address}")]
pub struct MetaData {
    pub(crate) id: Id,
    pub(crate) address: Address,
    pub(crate) ty: Type,
    pub(crate) flags: Flags,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub(crate) enum Response {
    Reply(Bytes, Flags),
    Error(String),
    Canceled,
}
