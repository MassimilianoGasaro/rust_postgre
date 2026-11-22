/// main.rs — Entry point dell'applicazione
///
/// Responsabilità di main():
/// 1. Caricare le variabili d'ambiente dal file .env
/// 2. Inizializzare il sistema di logging (tracing)
/// 3. Leggere la configurazione (config.rs)
/// 4. Creare il pool di connessioni al DB e applicare le migration
/// 5. Costruire l'AppState condiviso
/// 6. Costruire il Router (vedi lib.rs → build_router)
/// 7. Avviare il server TCP in ascolto
use axum_server::tls_rustls::RustlsConfig;

use rust_postgre::{AppState, build_router, config, repositories::import_repository};

#[tokio::main]
async fn main() {
    // 1. Carica le variabili dal file .env (se non già presenti nell'ambiente)
    dotenvy::dotenv().ok();

    // 2. Inizializza il logger differenziato per ambiente
    //
    // #[cfg(debug_assertions)] → true con `cargo run` / `cargo build`
    // #[cfg(not(debug_assertions))] → true con `cargo build --release`
    init_tracing();

    // 3. Legge la configurazione dalle variabili d'ambiente
    let config = config::Config::from_env();

    // 4. Crea il pool di connessioni al DB e applica le migration pendenti
    let pool = config::create_db_pool(&config.database_url).await;
    tracing::info!("Connesso a PostgreSQL");

    sqlx::migrate!()
        .run(&pool)
        .await
        .expect("Impossibile applicare le migration");
    tracing::info!("Migration applicate");

    // Gli import rimasti `processing` da un'esecuzione precedente non verranno
    // mai completati: li chiude come `failed` (item `IMPORT_INTERRUPTED`)
    let interrupted = import_repository::fail_interrupted(&pool)
        .await
        .expect("Impossibile chiudere gli import interrotti");
    if interrupted > 0 {
        tracing::warn!("{} import interrotti chiusi come failed", interrupted);
    }

    // Assicura che la directory degli upload esista
    tokio::fs::create_dir_all(&config.upload_dir)
        .await
        .expect("Impossibile creare la directory di upload");

    // File temporanei di upload rimasti da un'esecuzione interrotta
    match tokio::fs::remove_dir_all(config.upload_dir.join("tmp")).await {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!("Pulizia dei file temporanei di upload fallita: {}", e),
    }

    // Estrai prima i dati che servono dopo, così non ci sono problemi di ownership
    let address: std::net::SocketAddr = config
        .server_address()
        .parse()
        .expect("Indirizzo server non valido");
    let tls_config = build_tls_config(&config).await;

    // 5. Costruisce l'AppState condiviso
    let state = AppState {
        pool,
        jwt_secret: config.jwt_secret,
        jwt_expiration_hours: config.jwt_expiration_hours,
        upload_dir: config.upload_dir,
        max_upload_bytes: config.max_upload_bytes,
        max_file_bytes: config.max_file_bytes,
        cors_origin: config.cors_origin,
    };

    // 6. Costruisce il Router
    let app = build_router(state);

    // 7. Avvia il server HTTPS

    tracing::info!("Server HTTPS in ascolto su https://{}", address);

    axum_server::bind_rustls(address, tls_config)
        .serve(app.into_make_service())
        .await
        .expect("Errore nel server");
}

/// Costruisce la configurazione TLS.
///
/// - Se TLS_CERT_PATH e TLS_KEY_PATH sono impostati nel .env, usa quei file.
/// - Altrimenti genera al volo un certificato self-signed (comodo per sviluppo).
async fn build_tls_config(config: &config::Config) -> RustlsConfig {
    match (&config.tls_cert_path, &config.tls_key_path) {
        (Some(cert), Some(key)) => {
            // Usa certificati reali (es. Let's Encrypt in produzione)
            RustlsConfig::from_pem_file(cert, key)
                .await
                .expect("Impossibile caricare i certificati TLS")
        }
        _ => {
            // Genera un certificato self-signed per sviluppo
            tracing::warn!("Nessun certificato TLS trovato — uso self-signed (solo sviluppo)");
            let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
                .expect("Errore nella generazione del certificato self-signed");
            let cert_pem = cert.cert.pem().into_bytes();
            let key_pem = cert.key_pair.serialize_pem().into_bytes();
            RustlsConfig::from_pem(cert_pem, key_pem)
                .await
                .expect("Impossibile configurare TLS con certificato self-signed")
        }
    }
}

/// Inizializza il sistema di logging in base all'ambiente.
///
/// DEV  (`cargo run` / `cargo build`):
///   - Solo stdout, formato human-readable con colori, livello DEBUG
///
/// PROD (`cargo build --release`):
///   - Stdout + file rotante giornaliero in logs/app.log
///   - Formato JSON (machine-readable, ideale per aggregatori tipo Loki/ELK)
///   - Livello INFO (niente debug noise in produzione)
fn init_tracing() {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

    // Livello di log:
    //   DEV  → debug (vedi tutto)
    //   PROD → info  (solo eventi rilevanti)
    #[cfg(debug_assertions)]
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "rust_postgre=debug,info".into());

    #[cfg(not(debug_assertions))]
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "rust_postgre=info,warn".into());

    #[cfg(debug_assertions)]
    {
        // ── SVILUPPO: solo stdout, colori, pretty-print ───────────────────
        tracing_subscriber::registry()
            .with(env_filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_target(true) // mostra modulo::funzione
                    .with_line_number(true) // mostra numero di riga
                    .pretty(), // output leggibile
            )
            .init();

        tracing::debug!("🛠️  Modalità DEV — logging su stdout");
    }

    #[cfg(not(debug_assertions))]
    {
        // ── PRODUZIONE: stdout + file rotante giornaliero ─────────────────
        //
        // _guard DEVE restare in vita per tutta la durata del programma.
        // Se viene droppato, il writer del file si chiude.
        // Usiamo Box::leak per "leakarlo" intenzionalmente (ok per un process lifetime).
        let file_appender = tracing_appender::rolling::daily("logs", "app.log");
        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
        Box::leak(Box::new(guard)); // mantiene il guard vivo per sempre

        tracing_subscriber::registry()
            .with(env_filter)
            .with(
                // Layer 1: stdout — formato compatto
                tracing_subscriber::fmt::layer()
                    .with_target(false)
                    .with_ansi(true),
            )
            .with(
                // Layer 2: file — formato JSON (facile da parsare con Loki/ELK)
                tracing_subscriber::fmt::layer()
                    .json() // output JSON strutturato
                    .with_writer(non_blocking)
                    .with_ansi(false), // niente escape codes ANSI nel file
            )
            .init();

        tracing::info!("🚀 Modalità PROD — logging su stdout + logs/app.log");
    }
}
