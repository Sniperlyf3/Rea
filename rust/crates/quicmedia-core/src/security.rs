//! TLS 1.3 material and peer verification for QUIC.
//!
//! QUIC mandates TLS 1.3. For the prototype we default to self-signed
//! certificates plus a pluggable verifier: accept-any (dev), exact-cert pin, or
//! SHA-256 cert pin. The verifier is a normal `rustls` component, so swapping in
//! full PKI / WebPKI validation later is a localized change.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};

use crate::config::{PeerVerification, SecurityConfig};
use crate::{Error, Result};

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn tls_err<E: std::fmt::Display>(e: E) -> Error {
    Error::Tls(e.to_string())
}

/// Generate a fresh self-signed certificate for the given subject alt names.
/// Returns `(certificate_der, pkcs8_private_key_der)`.
pub fn generate_self_signed(subject_alt_names: &[String]) -> Result<(Vec<u8>, Vec<u8>)> {
    let sans: Vec<String> = if subject_alt_names.is_empty() {
        vec!["localhost".to_string()]
    } else {
        subject_alt_names.to_vec()
    };
    let certified = rcgen::generate_simple_self_signed(sans)
        .map_err(|e| Error::Tls(format!("self-signed cert generation: {e}")))?;
    let cert_der = certified.cert.der().to_vec();
    let key_der = certified.key_pair.serialize_der();
    Ok((cert_der, key_der))
}

/// SHA-256 digest of a certificate DER, for use with [`PeerVerification::PinnedSha256`].
pub fn cert_sha256(cert_der: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(cert_der);
    hasher.finalize().into()
}

/// Build a `rustls` server configuration from [`SecurityConfig`].
pub fn build_server_config(sec: &SecurityConfig) -> Result<rustls::ServerConfig> {
    let cert = CertificateDer::from(sec.cert_der.clone());
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(sec.key_der.clone()));
    let mut cfg = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(tls_err)?
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .map_err(tls_err)?;
    cfg.alpn_protocols = sec.alpn.clone();
    Ok(cfg)
}

/// Build a `rustls` client configuration from [`SecurityConfig`].
pub fn build_client_config(sec: &SecurityConfig) -> Result<rustls::ClientConfig> {
    let verifier = PinVerifier {
        provider: provider(),
        mode: sec.verification.clone(),
    };
    let mut cfg = rustls::ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(tls_err)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    cfg.alpn_protocols = sec.alpn.clone();
    Ok(cfg)
}

/// Certificate verifier implementing the project's pinning policy.
#[derive(Debug)]
struct PinVerifier {
    provider: Arc<CryptoProvider>,
    mode: PeerVerification,
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        match &self.mode {
            PeerVerification::Insecure => Ok(ServerCertVerified::assertion()),
            PeerVerification::PinnedCert(expected) => {
                if end_entity.as_ref() == expected.as_slice() {
                    Ok(ServerCertVerified::assertion())
                } else {
                    Err(rustls::Error::General("certificate pin mismatch".into()))
                }
            }
            PeerVerification::PinnedSha256(expected) => {
                if &cert_sha256(end_entity.as_ref()) == expected {
                    Ok(ServerCertVerified::assertion())
                } else {
                    Err(rustls::Error::General(
                        "certificate SHA-256 pin mismatch".into(),
                    ))
                }
            }
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_signed_builds_configs() {
        let (cert, key) = generate_self_signed(&["localhost".into()]).unwrap();
        assert!(!cert.is_empty() && !key.is_empty());
        let sec = SecurityConfig {
            cert_der: cert.clone(),
            key_der: key,
            verification: PeerVerification::PinnedSha256(cert_sha256(&cert)),
            alpn: vec![b"rea-qm/1".to_vec()],
        };
        build_server_config(&sec).unwrap();
        build_client_config(&sec).unwrap();
    }
}
