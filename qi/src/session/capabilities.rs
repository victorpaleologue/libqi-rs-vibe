//! Capabilities of a messaging stream.
//!
//! Capabilities are exchanged between peers during the authentication handshake. Each peer
//! advertises its local capabilities, and the *shared* capabilities of a stream are the ones
//! supported by both peers (for boolean capabilities, the logical `and` of both values). The
//! shared capabilities decide the behavior of the messaging layer: wire layout of object
//! references, availability of call cancellation, etc.

use crate::value::{IntoValue, KeyDynValueMap};
use once_cell::sync::Lazy;

/// The variant of the messaging protocol a peer speaks, detected when the session is
/// established.
///
/// The binary format and the messages are the same in both variants; they differ in how a
/// connection starts and in the capabilities the peer may have.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub enum Protocol {
    /// The protocol of `libqi` up to 2.2, the one of NAOqi 2.1 robots.
    ///
    /// There is no authentication: both ends advertise their capabilities with a `Capabilities`
    /// message right after the connection, and a server answers the authentication call of
    /// newer clients with an error. Calls cannot be canceled, object references carry no UID
    /// and service endpoints are absolute.
    Legacy,
    /// The protocol of `libqi` 2.3 and later (NAOqi 2.3 to 2.9, `libqi` 3 and 4).
    ///
    /// The connecting end authenticates first, and the capabilities are exchanged in that
    /// handshake.
    #[default]
    Standard,
}

impl Protocol {
    /// The protocol a NAOqi version speaks, from its `major.minor` prefix: NAOqi 2.1 and
    /// earlier speak the legacy protocol.
    pub fn of_naoqi_version(version: &str) -> Self {
        let mut parts = version.split('.').map(|part| part.parse::<u32>().ok());
        match (parts.next().flatten(), parts.next().flatten()) {
            (Some(major), Some(minor)) if (major, minor) < (2, 3) => Self::Legacy,
            (Some(major), None) if major < 2 => Self::Legacy,
            _ => Self::Standard,
        }
    }
}

impl std::fmt::Display for Protocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Legacy => "legacy",
            Self::Standard => "standard",
        })
    }
}

impl std::str::FromStr for Protocol {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "legacy" => Ok(Self::Legacy),
            "standard" => Ok(Self::Standard),
            _ => Err(format!(
                "unknown protocol \"{s}\": expected \"legacy\" or \"standard\""
            )),
        }
    }
}

/// The set of capabilities known to this implementation.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Capabilities {
    /// A client socket can accept and dispatch calls, so that the stream used by a node to
    /// register a service to the service directory may be reused to talk to that service.
    pub client_server_socket: bool,
    /// The remote supports the flags byte of message headers, and consequently dynamic payloads.
    pub message_flags: bool,
    /// Object references may be replaced by a cache identifier of their meta object.
    ///
    /// This implementation never uses the cache, and advertises this capability as `false`.
    pub meta_object_cache: bool,
    /// The remote supports the cancellation of calls.
    pub remote_cancelable_calls: bool,
    /// Object references carry the UID of the object.
    pub object_ptr_uid: bool,
    /// Service endpoints may be relative to the service directory stream (`qi:` URIs).
    pub relative_endpoint_uri: bool,
}

impl Capabilities {
    pub(crate) const CLIENT_SERVER_SOCKET: &'static str = "ClientServerSocket";
    pub(crate) const MESSAGE_FLAGS: &'static str = "MessageFlags";
    pub(crate) const META_OBJECT_CACHE: &'static str = "MetaObjectCache";
    pub(crate) const REMOTE_CANCELABLE_CALLS: &'static str = "RemoteCancelableCalls";
    pub(crate) const OBJECT_PTR_UID: &'static str = "ObjectPtrUID";
    pub(crate) const RELATIVE_ENDPOINT_URI: &'static str = "RelativeEndpointURI";

    /// The capabilities supported by this implementation.
    pub const LOCAL: Self = Self {
        client_server_socket: true,
        message_flags: true,
        meta_object_cache: false,
        remote_cancelable_calls: true,
        object_ptr_uid: true,
        relative_endpoint_uri: true,
    };

    /// The capabilities this implementation advertises when it emulates a legacy server (the
    /// ones of `libqi` 2.1, except the meta object cache that this implementation does not
    /// support).
    pub const LEGACY_LOCAL: Self = Self {
        client_server_socket: true,
        message_flags: true,
        meta_object_cache: false,
        remote_cancelable_calls: false,
        object_ptr_uid: false,
        relative_endpoint_uri: false,
    };

    /// The capabilities assumed for a remote that advertised nothing.
    ///
    /// This is the behavior of the oldest versions of the protocol.
    pub const NONE: Self = Self {
        client_server_socket: false,
        message_flags: false,
        meta_object_cache: false,
        remote_cancelable_calls: false,
        object_ptr_uid: false,
        relative_endpoint_uri: false,
    };

    /// Reads the capabilities from a capabilities map. Missing capabilities are considered
    /// unsupported.
    pub fn from_map(map: &KeyDynValueMap) -> Self {
        let get = |key| map.get_as(key).unwrap_or(false);
        Self {
            client_server_socket: get(Self::CLIENT_SERVER_SOCKET),
            message_flags: get(Self::MESSAGE_FLAGS),
            meta_object_cache: get(Self::META_OBJECT_CACHE),
            remote_cancelable_calls: get(Self::REMOTE_CANCELABLE_CALLS),
            object_ptr_uid: get(Self::OBJECT_PTR_UID),
            relative_endpoint_uri: get(Self::RELATIVE_ENDPOINT_URI),
        }
    }

    /// Writes the capabilities into a capabilities map.
    pub fn to_map(self) -> KeyDynValueMap {
        KeyDynValueMap::from_iter([
            (
                Self::CLIENT_SERVER_SOCKET.to_owned(),
                self.client_server_socket.into_value(),
            ),
            (
                Self::MESSAGE_FLAGS.to_owned(),
                self.message_flags.into_value(),
            ),
            (
                Self::META_OBJECT_CACHE.to_owned(),
                self.meta_object_cache.into_value(),
            ),
            (
                Self::REMOTE_CANCELABLE_CALLS.to_owned(),
                self.remote_cancelable_calls.into_value(),
            ),
            (
                Self::OBJECT_PTR_UID.to_owned(),
                self.object_ptr_uid.into_value(),
            ),
            (
                Self::RELATIVE_ENDPOINT_URI.to_owned(),
                self.relative_endpoint_uri.into_value(),
            ),
        ])
    }

    /// The capabilities shared with a remote that advertised the given capabilities.
    pub fn shared_with(self, remote: Self) -> Self {
        Self {
            client_server_socket: self.client_server_socket && remote.client_server_socket,
            message_flags: self.message_flags && remote.message_flags,
            meta_object_cache: self.meta_object_cache && remote.meta_object_cache,
            remote_cancelable_calls: self.remote_cancelable_calls && remote.remote_cancelable_calls,
            object_ptr_uid: self.object_ptr_uid && remote.object_ptr_uid,
            relative_endpoint_uri: self.relative_endpoint_uri && remote.relative_endpoint_uri,
        }
    }

    /// The capabilities shared between this implementation and a remote that advertised the
    /// given capabilities map.
    pub fn shared_with_remote_map(map: &KeyDynValueMap) -> Self {
        Self::LOCAL.shared_with(Self::from_map(map))
    }
}

impl Default for Capabilities {
    fn default() -> Self {
        Self::LOCAL
    }
}

/// The map of local capabilities, as advertised to remotes.
pub(crate) fn local_map() -> &'static KeyDynValueMap {
    static LOCAL_CAPABILITIES_MAP: Lazy<KeyDynValueMap> =
        Lazy::new(|| Capabilities::LOCAL.to_map());
    &LOCAL_CAPABILITIES_MAP
}

/// The map of capabilities advertised when emulating a legacy server: only the keys a `libqi`
/// 2.1 process knows.
pub(crate) fn legacy_local_map() -> &'static KeyDynValueMap {
    static MAP: Lazy<KeyDynValueMap> = Lazy::new(|| {
        let mut map = KeyDynValueMap::new();
        map.set(Capabilities::CLIENT_SERVER_SOCKET, true);
        map.set(Capabilities::MESSAGE_FLAGS, true);
        map.set(Capabilities::META_OBJECT_CACHE, false);
        map
    });
    &MAP
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_capabilities_are_the_intersection() {
        let mut remote = KeyDynValueMap::new();
        remote.set(Capabilities::CLIENT_SERVER_SOCKET, true);
        remote.set(Capabilities::MESSAGE_FLAGS, true);
        remote.set(Capabilities::META_OBJECT_CACHE, true);
        remote.set(Capabilities::REMOTE_CANCELABLE_CALLS, false);
        // ObjectPtrUID and RelativeEndpointURI are missing.
        let shared = Capabilities::shared_with_remote_map(&remote);
        assert_eq!(
            shared,
            Capabilities {
                client_server_socket: true,
                message_flags: true,
                meta_object_cache: false,
                remote_cancelable_calls: false,
                object_ptr_uid: false,
                relative_endpoint_uri: false,
            }
        );
    }

    #[test]
    fn local_map_round_trips() {
        assert_eq!(Capabilities::from_map(local_map()), Capabilities::LOCAL);
        assert_eq!(
            Capabilities::from_map(legacy_local_map()),
            Capabilities::LEGACY_LOCAL
        );
    }

    #[test]
    fn protocol_of_naoqi_versions() {
        assert_eq!(Protocol::of_naoqi_version("2.1.4.13"), Protocol::Legacy);
        assert_eq!(Protocol::of_naoqi_version("1.14.5"), Protocol::Legacy);
        assert_eq!(Protocol::of_naoqi_version("2.3.0"), Protocol::Standard);
        assert_eq!(Protocol::of_naoqi_version("2.8.7.4"), Protocol::Standard);
        assert_eq!(Protocol::of_naoqi_version("2.9.5.1"), Protocol::Standard);
        assert_eq!(Protocol::of_naoqi_version("garbage"), Protocol::Standard);
        assert_eq!("legacy".parse(), Ok(Protocol::Legacy));
        assert_eq!(Protocol::Standard.to_string(), "standard");
    }
}
