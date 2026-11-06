//! The listener's TLS (phone view spec section 4): a self-signed certificate made on first use
//! and kept, its key encrypted for the signed-in Windows user. Phones cannot check a self-signed
//! certificate against anyone, so its SHA-256 fingerprint is shown in Settings and carried in
//! the pairing link, for the person to compare with what the phone's browser shows.

use std::sync::Arc;

use sha2::{Digest, Sha256};
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio_rustls::rustls::{self, ServerConfig};

use crate::store::{PhoneTls, Store};
use crate::util::now;
use crate::vault::org::offline::Protector;
use crate::{Error, Result};

/// What the protector binds the key to, so another blob of Dots' cannot stand in for it.
const KEY_ENTROPY: &[u8] = b"dots phone view tls key";

/// A certificate and its key, ready to serve.
#[derive(Clone)]
pub struct Identity {
    pub config: Arc<ServerConfig>,
    /// SHA-256 of the certificate, as uppercase hex pairs joined by `:` (how Safari and Chrome
    /// show it).
    pub fingerprint: String,
}

/// The fingerprint of a DER certificate, as browsers show it.
pub fn fingerprint(cert_der: &[u8]) -> String {
    Sha256::digest(cert_der)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// The kept certificate, or a new one (kept when the protector can encrypt its key; otherwise
/// it lasts until Dots closes and phones see a new fingerprint next time).
pub async fn load_or_make(store: &Store, protector: &dyn Protector) -> Result<Identity> {
    if let Some(kept) = store.phone_tls().await? {
        match protector.unprotect(&kept.key_protected, KEY_ENTROPY) {
            Ok(key) => match identity(kept.cert_der.clone(), key.to_vec()) {
                Ok(id) => return Ok(id),
                Err(e) => tracing::warn!("the kept phone certificate is unusable, making one: {e}"),
            },
            Err(e) => tracing::warn!("the phone certificate's key cannot be read, making one: {e}"),
        }
    }
    let (cert_der, key_der) = make()?;
    match protector.protect(&key_der, KEY_ENTROPY) {
        Ok(key_protected) => {
            let kept = PhoneTls {
                cert_der: cert_der.clone(),
                key_protected,
            };
            store.put_phone_tls(&kept, &now()).await?;
        }
        Err(e) => tracing::warn!("the phone certificate is not kept: {e}"),
    }
    identity(cert_der, key_der)
}

/// A new self-signed ECDSA P-256 certificate for ten years, named `Dots phone view`. Its name
/// does not matter: no browser trusts it until the person accepts it, and the IP address a
/// phone opens may change.
fn make() -> Result<(Vec<u8>, Vec<u8>)> {
    let err = |e: rcgen::Error| Error::Other(format!("making the phone certificate failed: {e}"));
    let mut params =
        rcgen::CertificateParams::new(vec!["dots-phone.local".to_string()]).map_err(err)?;
    let mut name = rcgen::DistinguishedName::new();
    name.push(rcgen::DnType::CommonName, "Dots phone view");
    params.distinguished_name = name;
    let year = chrono::Datelike::year(&chrono::Utc::now());
    params.not_before = rcgen::date_time_ymd(year - 1, 1, 1);
    params.not_after = rcgen::date_time_ymd(year + 10, 1, 1);
    let key = rcgen::KeyPair::generate().map_err(err)?;
    let cert = params.self_signed(&key).map_err(err)?;
    Ok((cert.der().to_vec(), key.serialize_der()))
}

fn identity(cert_der: Vec<u8>, key_der: Vec<u8>) -> Result<Identity> {
    let fingerprint = fingerprint(&cert_der);
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Other(format!("phone TLS: {e}")))?
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(cert_der)],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der)),
        )
        .map_err(|e| Error::Other(format!("phone TLS: {e}")))?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Identity {
        config: Arc::new(config),
        fingerprint,
    })
}
