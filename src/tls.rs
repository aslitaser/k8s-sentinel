use std::fs;
use std::io::BufReader;
use std::sync::Arc;

use rustls::ServerConfig;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TlsError {
    #[error("failed to read cert file '{path}': {source}")]
    CertFileRead {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to read key file '{path}': {source}")]
    KeyFileRead {
        path: String,
        source: std::io::Error,
    },
    #[error("no valid certificates found in '{0}'")]
    NoCerts(String),
    #[error("no valid private key found in '{0}'")]
    NoKey(String),
    #[error("failed to build TLS config: {0}")]
    RustlsConfig(#[from] rustls::Error),
}

pub fn load_tls_config(cert_path: &str, key_path: &str) -> Result<Arc<ServerConfig>, TlsError> {
    let cert_data = fs::read(cert_path).map_err(|e| TlsError::CertFileRead {
        path: cert_path.to_string(),
        source: e,
    })?;
    let key_data = fs::read(key_path).map_err(|e| TlsError::KeyFileRead {
        path: key_path.to_string(),
        source: e,
    })?;

    let certs: Vec<_> = rustls_pemfile::certs(&mut BufReader::new(cert_data.as_slice()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| TlsError::CertFileRead {
            path: cert_path.to_string(),
            source: e,
        })?;

    if certs.is_empty() {
        return Err(TlsError::NoCerts(cert_path.to_string()));
    }

    let key = rustls_pemfile::private_key(&mut BufReader::new(key_data.as_slice()))
        .map_err(|e| TlsError::KeyFileRead {
            path: key_path.to_string(),
            source: e,
        })?
        .ok_or_else(|| TlsError::NoKey(key_path.to_string()))?;

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;

    Ok(Arc::new(config))
}
