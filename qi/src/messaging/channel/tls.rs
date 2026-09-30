//! TLS for `tcps://` endpoints.
//!
//! The semantics are those of the reference implementation: peers do not verify each other's
//! certificates (only mutual authentication, `tcpsm://`, does, and it is not supported yet), so
//! servers may use a self-signed certificate. The certificate and private key of servers are
//! taken from the PEM files named by the `QI_TLS_CERTIFICATE` and `QI_TLS_PRIVATE_KEY`
//! environment variables when both are set, and generated for the process otherwise.

use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::CryptoProvider,
    pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer, ServerName, UnixTime},
    ClientConfig, DigitallySignedStruct, ServerConfig, SignatureScheme,
};
use std::{
    net::SocketAddr,
    sync::{Arc, OnceLock},
};
use tokio::net::TcpStream;
use tokio_rustls::{client, TlsAcceptor, TlsConnector};

/// The environment variable naming the PEM file of the certificate chain of servers.
pub const CERTIFICATE_ENV: &str = "QI_TLS_CERTIFICATE";
/// The environment variable naming the PEM file of the private key of servers.
pub const PRIVATE_KEY_ENV: &str = "QI_TLS_PRIVATE_KEY";

fn provider() -> Arc<CryptoProvider> {
    static PROVIDER: OnceLock<Arc<CryptoProvider>> = OnceLock::new();
    Arc::clone(PROVIDER.get_or_init(|| Arc::new(rustls::crypto::ring::default_provider())))
}

/// Accepts any server certificate, as the clients of the reference implementation do for
/// `tcps://` endpoints (the certificate is checked in mutual authentication only).
#[derive(Debug)]
struct AcceptAnyServerCertificate(Arc<CryptoProvider>);

impl ServerCertVerifier for AcceptAnyServerCertificate {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

fn client_config() -> Arc<ClientConfig> {
    static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    Arc::clone(CONFIG.get_or_init(|| {
        let provider = provider();
        let config = ClientConfig::builder_with_provider(Arc::clone(&provider))
            .with_safe_default_protocol_versions()
            .expect("the default protocol versions are supported by the provider")
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAnyServerCertificate(provider)))
            .with_no_client_auth();
        Arc::new(config)
    }))
}

/// Starts a TLS session over a connected socket, as a client.
pub(super) async fn connect(
    tcp: TcpStream,
    address: SocketAddr,
) -> std::io::Result<client::TlsStream<TcpStream>> {
    let server_name = ServerName::IpAddress(address.ip().into());
    TlsConnector::from(client_config())
        .connect(server_name, tcp)
        .await
}

/// The acceptor of TLS sessions of servers, with the certificate of the process.
pub(super) fn acceptor() -> std::io::Result<TlsAcceptor> {
    static CONFIG: OnceLock<Result<Arc<ServerConfig>, String>> = OnceLock::new();
    CONFIG
        .get_or_init(server_config)
        .clone()
        .map(TlsAcceptor::from)
        .map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("cannot configure TLS for servers: {error}"),
            )
        })
}

fn server_config() -> Result<Arc<ServerConfig>, String> {
    let (certificates, key) = match (
        std::env::var_os(CERTIFICATE_ENV),
        std::env::var_os(PRIVATE_KEY_ENV),
    ) {
        (Some(certificate), Some(key)) => {
            let certificates = CertificateDer::pem_file_iter(&certificate)
                .and_then(|certificates| certificates.collect::<Result<Vec<_>, _>>())
                .map_err(|error| {
                    format!(
                        "cannot read the certificate file {}: {error}",
                        certificate.to_string_lossy()
                    )
                })?;
            let key = PrivateKeyDer::from_pem_file(&key).map_err(|error| {
                format!(
                    "cannot read the private key file {}: {error}",
                    key.to_string_lossy()
                )
            })?;
            (certificates, key)
        }
        _ => self_signed()?,
    };
    ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|error| error.to_string())?
        .with_no_client_auth()
        .with_single_cert(certificates, key)
        .map(Arc::new)
        .map_err(|error| error.to_string())
}

/// A self-signed certificate for this process.
fn self_signed() -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>), String> {
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])
        .map_err(|error| format!("cannot generate a self-signed certificate: {error}"))?;
    let key = PrivateKeyDer::try_from(certified.key_pair.serialize_der())
        .map_err(|error| format!("cannot serialize the generated private key: {error}"))?;
    Ok((vec![certified.cert.der().clone()], key))
}
