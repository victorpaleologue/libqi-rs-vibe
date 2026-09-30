// TODO: #![deny(missing_docs)]
#![doc = include_str!("messaging/README.md")]

mod address;
pub mod channel;
mod client;
pub mod codec;
pub mod endpoint;
mod error;
pub mod handler;
mod id;
pub mod message;
mod server;

pub use self::{
    address::{Address, Error as AddressError},
    client::{Client, WeakClient},
    codec::{DecodeError, Decoder, EncodeError, Encoder},
    error::Error,
    handler::{
        CallHandler, CancellationToken, CapabilitiesHandler, EventHandler, Handler, PostHandler,
    },
    message::{Flags, Message},
};
pub use crate::{format, value};
