//! Hosting of objects transmitted to a peer.
//!
//! When a local object is transmitted by value to the peer of a session (as an argument, a return
//! value or a signal parameter), it is *hosted* by the session: it gets an identifier under which
//! the peer can address it, for as long as the peer references it or the session lasts.

use crate::{object::AnyObject, service, value::object::Id};
use std::collections::HashMap;

/// The first identifier of objects hosted by the accepting side of a session.
pub(crate) const SERVER_FIRST_ID: u32 = 2;

/// The first identifier of objects hosted by the connecting side of a session.
///
/// The two sides of a session use disjoint identifier ranges, like the reference implementation,
/// so that an address is never ambiguous.
pub(crate) const CLIENT_FIRST_ID: u32 = 1 << 31;

#[derive(Debug)]
pub(crate) struct ObjectHost {
    next_id: u32,
    objects: HashMap<Id, Hosted>,
}

struct Hosted {
    service_id: service::Id,
    object: AnyObject,
}

impl std::fmt::Debug for Hosted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hosted")
            .field("service_id", &self.service_id)
            .field("uid", &self.object.uid())
            .finish()
    }
}

impl ObjectHost {
    pub(crate) fn new(first_id: u32) -> Self {
        Self {
            next_id: first_id,
            objects: HashMap::new(),
        }
    }

    /// Hosts an object under the given service, returning its identifier.
    pub(crate) fn add(&mut self, service_id: service::Id, object: AnyObject) -> Id {
        let id = Id(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        self.objects.insert(id, Hosted { service_id, object });
        id
    }

    /// Gets a hosted object by identifier.
    pub(crate) fn get(&self, id: Id) -> Option<&AnyObject> {
        self.objects.get(&id).map(|hosted| &hosted.object)
    }

    /// Releases a hosted object.
    pub(crate) fn remove(&mut self, id: Id) -> Option<AnyObject> {
        self.objects.remove(&id).map(|hosted| hosted.object)
    }
}
