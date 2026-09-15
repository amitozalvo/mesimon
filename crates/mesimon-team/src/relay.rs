//! The relay client: TLS to `host:port`, one frame each way per connection.
//!
//! Trust is either a pinned certificate (self-hosted: the relay prints its
//! pin at startup and the person types it once at sign-in) or the Mozilla
//! root store (managed: an ordinary certificate). Pinning replaces name and
//! chain validation only; the handshake signature is still verified, so the
//! pinned certificate's private key must be held by whoever answers.
use crate::hex;
use crate::wire::{read_frame, write_frame, Credential, ErrorCode, Frame, Request, Response};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayEndpoint {
    pub host: String,
    pub port: u16,
    /// SHA-256 of the server certificate's DER bytes, when self-hosted.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "opt_pin")]
    pub pin: Option<[u8; 32]>,
}

mod opt_pin {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &Option<[u8; 32]>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(pin) => s.serialize_some(&crate::hex::encode(pin)),
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<[u8; 32]>, D::Error> {
        let text: Option<String> = Option::deserialize(d)?;
        text.map(|t| {
            super::parse_pin(&t).ok_or_else(|| serde::de::Error::custom("invalid certificate pin"))
        })
        .transpose()
    }
}

impl RelayEndpoint {
    /// `host`, `host:port`, or either followed by a space and a 64-hex pin.
    /// The default port is 8443.
    pub fn parse(text: &str) -> Option<Self> {
        let mut words = text.split_whitespace();
        let address = words.next()?;
        let pin = match words.next() {
            Some(pin) => Some(parse_pin(pin)?),
            None => None,
        };
        if words.next().is_some() {
            return None;
        }
        let (host, port) = match address.rsplit_once(':') {
            Some((host, port)) if !host.contains(':') => (host, port.parse().ok()?),
            _ => (address, 8443),
        };
        if host.is_empty() {
            return None;
        }
        Some(Self { host: host.to_owned(), port, pin })
    }
    pub fn display(&self) -> String {
        if self.port == 8443 {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

/// A 64-character lowercase hex SHA-256 of the server certificate's DER bytes.
pub fn parse_pin(text: &str) -> Option<[u8; 32]> {
    hex::decode::<32>(&text.trim().to_ascii_lowercase())
}

/// The fingerprint a client pins for this certificate.
pub fn certificate_pin(der: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(der).into()
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

#[derive(Debug)]
struct PinnedCertificate {
    pin: [u8; 32],
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl rustls::client::danger::ServerCertVerifier for PinnedCertificate {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let found = certificate_pin(end_entity.as_ref());
        let mut different = 0u8;
        for (a, b) in found.iter().zip(self.pin.iter()) {
            different |= a ^ b;
        }
        if different == 0 {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General("server certificate pin mismatch".into()))
        }
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

fn client_config(pin: Option<[u8; 32]>) -> Result<rustls::ClientConfig, ErrorCode> {
    let provider = provider();
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|_| ErrorCode::Unavailable)?;
    Ok(match pin {
        Some(pin) => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinnedCertificate { pin, provider }))
            .with_no_client_auth(),
        None => {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            builder.with_root_certificates(roots).with_no_client_auth()
        }
    })
}

pub struct RelayClient {
    endpoint: RelayEndpoint,
    config: Arc<rustls::ClientConfig>,
    timeout: Duration,
}

pub type ControlSocket =
    tungstenite::WebSocket<rustls::StreamOwned<rustls::ClientConnection, std::net::TcpStream>>;

impl RelayClient {
    pub fn control_socket(&self, origin: &str) -> Result<ControlSocket, ErrorCode> {
        let uri: tungstenite::http::Uri = origin.parse().map_err(|_| ErrorCode::InvalidRequest)?;
        if uri.scheme_str() != Some("https") || uri.path() != "/" || uri.query().is_some() {
            return Err(ErrorCode::InvalidRequest);
        }
        let host = uri.host().ok_or(ErrorCode::InvalidRequest)?;
        let port = uri.port_u16().unwrap_or(443);
        let stream =
            std::net::TcpStream::connect((host, port)).map_err(|_| ErrorCode::Unavailable)?;
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .map_err(|_| ErrorCode::Unavailable)?;
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(|_| ErrorCode::Unavailable)?;
        let name = rustls::pki_types::ServerName::try_from(host.to_owned())
            .map_err(|_| ErrorCode::InvalidRequest)?;
        let session = rustls::ClientConnection::new(self.config.clone(), name)
            .map_err(|_| ErrorCode::Unavailable)?;
        let tls = rustls::StreamOwned::new(session, stream);
        let url = format!("wss://{}/control", uri.authority().ok_or(ErrorCode::InvalidRequest)?);
        let config = tungstenite::protocol::WebSocketConfig::default()
            .max_message_size(Some(crate::control::MAX_BYTES))
            .max_frame_size(Some(crate::control::MAX_BYTES));
        let (ws, _) = tungstenite::client::client_with_config(url, tls, Some(config))
            .map_err(|_| ErrorCode::Unavailable)?;
        Ok(ws)
    }

    pub fn new(endpoint: RelayEndpoint) -> Result<Self, ErrorCode> {
        let config = Arc::new(client_config(endpoint.pin)?);
        Ok(Self { endpoint, config, timeout: Duration::from_secs(30) })
    }
    pub fn endpoint(&self) -> &RelayEndpoint {
        &self.endpoint
    }
    /// One request, one response. A transport failure is `Unavailable`; a
    /// relay-side refusal comes back as `Response::Error` and is lifted into
    /// the `Err` arm so callers match on one thing.
    pub fn call(
        &self,
        credential: Option<&Credential>,
        request: Request,
    ) -> Result<Response, ErrorCode> {
        let name = rustls::pki_types::ServerName::try_from(self.endpoint.host.clone())
            .map_err(|_| ErrorCode::InvalidRequest)?;
        let stream =
            std::net::TcpStream::connect((self.endpoint.host.as_str(), self.endpoint.port))
                .map_err(|_| ErrorCode::Unavailable)?;
        stream.set_read_timeout(Some(self.timeout)).map_err(|_| ErrorCode::Unavailable)?;
        stream.set_write_timeout(Some(self.timeout)).map_err(|_| ErrorCode::Unavailable)?;
        let session = rustls::ClientConnection::new(self.config.clone(), name)
            .map_err(|_| ErrorCode::Unavailable)?;
        let mut tls = rustls::StreamOwned::new(session, stream);
        write_frame(&mut tls, &Frame { credential: credential.cloned(), request })?;
        let bytes = read_frame(&mut tls)?;
        match serde_json::from_slice(&bytes).map_err(|_| ErrorCode::InvalidRequest)? {
            Response::Error { code } => Err(code),
            response => Ok(response),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_parse_with_and_without_port_and_pin() {
        let pin = "ab".repeat(32);
        let e = RelayEndpoint::parse(&format!("relay.example.com:9000 {pin}")).unwrap();
        assert_eq!((e.host.as_str(), e.port), ("relay.example.com", 9000));
        assert_eq!(e.pin, Some([0xab; 32]));
        assert_eq!(e.display(), "relay.example.com:9000");
        let e = RelayEndpoint::parse("teams.mesimon.dev").unwrap();
        assert_eq!((e.host.as_str(), e.port, e.pin), ("teams.mesimon.dev", 8443, None));
        assert_eq!(e.display(), "teams.mesimon.dev");
        assert!(RelayEndpoint::parse("").is_none());
        assert!(RelayEndpoint::parse("host:notaport").is_none());
        assert!(RelayEndpoint::parse("host short-pin").is_none());
        assert!(RelayEndpoint::parse("host pin extra").is_none());
        let text = serde_json::to_string(&e).unwrap();
        assert!(!text.contains("pin"));
        assert_eq!(serde_json::from_str::<RelayEndpoint>(&text).unwrap(), e);
    }

    #[test]
    fn both_trust_modes_build_a_config() {
        assert!(client_config(Some([7; 32])).is_ok());
        assert!(client_config(None).is_ok());
    }
}
