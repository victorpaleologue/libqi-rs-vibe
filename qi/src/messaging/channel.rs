//! Transport channels: connecting to and serving endpoints, framing messages.

mod tls;

pub use self::tls::{CERTIFICATE_ENV, PRIVATE_KEY_ENV};
use crate::messaging::{
    address::{Address, SslKind},
    DecodeError, Decoder, EncodeError, Encoder, Message,
};
use async_stream::stream;
use futures::{Sink, Stream};
use std::pin::Pin;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::{TcpListener, TcpStream},
};
use tokio_util::codec::{FramedRead, FramedWrite};
use tracing::debug;

type BoxRead = Pin<Box<dyn AsyncRead + Send>>;
type BoxWrite = Pin<Box<dyn AsyncWrite + Send>>;

fn unsupported(what: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Unsupported, what.to_owned())
}

/// Connects to an endpoint, returning the stream of incoming messages and the sink of outgoing
/// messages.
pub async fn connect(
    address: Address,
) -> Result<
    (
        impl Stream<Item = Result<Message, DecodeError>>,
        impl Sink<Message, Error = EncodeError>,
    ),
    std::io::Error,
> {
    let (read, write): (BoxRead, BoxWrite) = match address {
        Address::Tcp { address, ssl: None } => {
            let (read, write) = TcpStream::connect(address).await?.into_split();
            (Box::pin(read), Box::pin(write))
        }
        Address::Tcp {
            address,
            ssl: Some(SslKind::Simple),
        } => {
            let tcp = TcpStream::connect(address).await?;
            let (read, write) = tokio::io::split(tls::connect(tcp, address).await?);
            (Box::pin(read), Box::pin(write))
        }
        Address::Tcp {
            ssl: Some(SslKind::Mutual),
            ..
        } => {
            return Err(unsupported(
                "connecting with mutual TLS authentication (tcpsm) is not supported",
            ))
        }
    };
    let stream = FramedRead::new(read, Decoder::default());
    let sink = FramedWrite::new(write, Encoder);
    Ok((stream, sink))
}

/// Binds a server to an address, returning the stream of connecting clients (each as a stream of
/// incoming messages, a sink of outgoing messages and the address of the client) and the local
/// address the server is bound to.
pub async fn serve(
    address: Address,
) -> Result<
    (
        impl Stream<
            Item = (
                impl Stream<Item = Result<Message, DecodeError>>,
                impl Sink<Message, Error = EncodeError>,
                Address,
            ),
        >,
        Address,
    ),
    std::io::Error,
> {
    let Address::Tcp { address, ssl } = address;
    let acceptor = match ssl {
        None => None,
        Some(SslKind::Simple) => Some(tls::acceptor()?),
        Some(SslKind::Mutual) => {
            return Err(unsupported(
                "serving with mutual TLS authentication (tcpsm) is not supported",
            ))
        }
    };
    let listener = TcpListener::bind(address).await?;
    let endpoint = listener
        .local_addr()
        .map(|address| Address::Tcp { address, ssl })
        .unwrap_or(Address::Tcp { address, ssl });
    let clients = stream! {
        loop {
            // TODO: Handle case when accept returns an error that is fatal for this listener.
            let Ok((socket, address)) = listener.accept().await else {
                continue;
            };
            let (read, write): (BoxRead, BoxWrite) = match &acceptor {
                None => {
                    let (read, write) = socket.into_split();
                    (Box::pin(read), Box::pin(write))
                }
                Some(acceptor) => match acceptor.accept(socket).await {
                    Ok(stream) => {
                        let (read, write) = tokio::io::split(stream);
                        (Box::pin(read), Box::pin(write))
                    }
                    Err(error) => {
                        debug!(
                            %address,
                            error = &error as &dyn std::error::Error,
                            "TLS handshake failed"
                        );
                        continue;
                    }
                },
            };
            let stream = FramedRead::new(read, Decoder::default());
            let sink = FramedWrite::new(write, Encoder);
            yield (stream, sink, Address::Tcp { address, ssl });
        }
    };
    Ok((clients, endpoint))
}
