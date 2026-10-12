use std::{net::SocketAddr, str::FromStr};
use url::Url;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Address {
    Tcp {
        address: SocketAddr,
        ssl: Option<SslKind>,
    },
}

impl Address {
    /// The address of a URL. A host name resolves to its first address; see
    /// [`resolve_all`](Self::resolve_all) for every address it resolves to.
    pub fn from_url(url: &Url) -> Result<Self, Error> {
        Self::all_from_url(url)?
            .into_iter()
            .next()
            .ok_or(Error::ResolveHost(None))
    }

    /// Every address of a URL, in the order the resolver gives them: a host name may resolve to
    /// several (an IPv6 and an IPv4 address, typically), not all of them reachable. Connecting
    /// to the first that answers is what [`Node::connect_to_space_at`] does, as libqi does.
    ///
    /// [`Node::connect_to_space_at`]: crate::node::InitializingNode::connect_to_space_at
    ///
    /// # Resolution request and threading
    /// The host is resolved with `std::net::ToSocketAddrs`, blocking the current thread until
    /// the request terminates.
    pub fn resolve_all(url: &str) -> Result<Vec<Self>, Error> {
        Self::all_from_url(&url.parse().map_err(Error::InvalidUrl)?)
    }

    fn all_from_url(url: &Url) -> Result<Vec<Self>, Error> {
        match url.scheme().parse()? {
            Scheme::Tcp(ssl) => Ok(socket_addrs_from_url(url)?
                .into_iter()
                .map(|address| Self::Tcp { address, ssl })
                .collect()),
        }
    }

    /// Returns true if the address refers to an endpoint of the local machine.
    ///
    /// # Resolution request and threading
    /// This will try to DNS resolve the host, using `std::net::ToSocketAddrs`,
    /// blocking the current thread until the request terminates. If the request
    /// fails, the address is considered not local.
    pub fn is_machine_local(&self) -> bool {
        match self {
            Address::Tcp { address, .. } => address.ip().is_loopback(),
        }
    }
}

impl FromStr for Address {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_url(&s.parse().map_err(Error::InvalidUrl)?)
    }
}

impl std::fmt::Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tcp { address, ssl } => {
                let protocol = match ssl {
                    None => "tcp",
                    Some(SslKind::Simple) => "tcps",
                    Some(SslKind::Mutual) => "tcpsm",
                };
                write!(f, "{protocol}://{address}")
            }
        }
    }
}

fn socket_addrs_from_url(url: &Url) -> Result<Vec<SocketAddr>, Error> {
    const DEFAULT_PORT: u16 = 9559;
    let addresses = url
        .socket_addrs(|| match Scheme::from_str(url.scheme()) {
            Ok(Scheme::Tcp(_)) => Some(DEFAULT_PORT),
            Err(_) => None,
        })
        .map_err(|err| Error::ResolveHost(Some(err)))?;
    if addresses.is_empty() {
        return Err(Error::ResolveHost(None));
    }
    Ok(addresses)
}

#[derive(Debug)]
enum Scheme {
    Tcp(Option<SslKind>),
}

impl FromStr for Scheme {
    type Err = Error;

    fn from_str(scheme: &str) -> Result<Self, Self::Err> {
        match scheme {
            "tcp" => Ok(Self::Tcp(None)),
            "tcps" => Ok(Self::Tcp(Some(SslKind::Simple))),
            "tcpsm" => Ok(Self::Tcp(Some(SslKind::Mutual))),
            _ => Err(Error::UnsupportedUrlScheme(scheme.to_owned())),
        }
    }
}

#[derive(Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum SslKind {
    #[default]
    Simple,
    Mutual,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("address is not a valid URL")]
    InvalidUrl(#[source] url::ParseError),

    #[error("address URL scheme \"{0}\" is not supported")]
    UnsupportedUrlScheme(String),

    #[error("could not resolve address host")]
    ResolveHost(#[source] Option<std::io::Error>),
}
