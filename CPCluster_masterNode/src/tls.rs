use std::error::Error;
use std::fs;

use rcgen::generate_simple_self_signed;
use tokio_rustls::rustls;

pub fn load_or_generate_tls_config(
    cert_path: &str,
    key_path: &str,
    host: &str,
) -> Result<rustls::ServerConfig, Box<dyn Error + Send + Sync>> {
    if std::path::Path::new(cert_path).exists() && std::path::Path::new(key_path).exists() {
        let mut cert_file = std::io::BufReader::new(fs::File::open(cert_path)?);
        let mut key_file = std::io::BufReader::new(fs::File::open(key_path)?);
        let certs = rustls_pemfile::certs(&mut cert_file)?
            .into_iter()
            .map(rustls::Certificate)
            .collect();
        let keys = rustls_pemfile::pkcs8_private_keys(&mut key_file)?;
        let key = keys.first().ok_or("no private key found")?.clone();
        let config = rustls::ServerConfig::builder()
            .with_safe_defaults()
            .with_no_client_auth()
            .with_single_cert(certs, rustls::PrivateKey(key))?;
        Ok(config)
    } else {
        let cert = generate_simple_self_signed(vec![host.to_string()])?;
        fs::write(cert_path, cert.serialize_pem()?)?;
        fs::write(key_path, cert.serialize_private_key_pem())?;
        let cert_der = cert.serialize_der()?;
        let key_der = cert.serialize_private_key_der();
        let config = rustls::ServerConfig::builder()
            .with_safe_defaults()
            .with_no_client_auth()
            .with_single_cert(
                vec![rustls::Certificate(cert_der)],
                rustls::PrivateKey(key_der),
            )?;
        Ok(config)
    }
}
