//! Configurazione TLS del server HTTPS (sviluppo, `TLS_ENABLED=true`).
//!
//! Riproduce il percorso di `main.rs` con certificato self-signed: se nel build sono attivi
//! due provider crittografici di rustls (ring e aws-lc-rs), rustls non sa quale usare e la
//! costruzione della configurazione va in panic.

use axum_server::tls_rustls::RustlsConfig;

#[tokio::test]
async fn self_signed_tls_config_builds() {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();

    RustlsConfig::from_pem(
        cert.cert.pem().into_bytes(),
        cert.key_pair.serialize_pem().into_bytes(),
    )
    .await
    .expect("configurazione TLS self-signed");
}
