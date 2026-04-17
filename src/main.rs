/// main.rs — Entry point dell'applicazione
///
/// Responsabilità di main():
/// 1. Caricare le variabili d'ambiente dal file .env
/// 2. Inizializzare il sistema di logging (tracing)
/// 3. Leggere la configurazione (config.rs)
/// 4. Creare il pool di connessioni al DB
/// 5. Costruire l'AppState condiviso
/// 6. Definire il Router con tutti gli endpoint
/// 7. Avviare il server TCP in ascolto

mod config;
mod controllers;
mod dto;
mod errors;
mod middleware;
mod models;
mod repositories;
mod services;

use axum::{
    middleware::from_fn_with_state,
    routing::{delete, get, patch, post},
    Router,
};
use axum_server::tls_rustls::RustlsConfig;
use sqlx::PgPool;

use controllers::{auth_controller, user_controller};

/// AppState — stato condiviso tra tutti gli handler.
///
/// In Axum lo stato viene clonato per ogni request, quindi deve
/// implementare Clone. Usiamo Arc<PgPool> perché il pool è già
/// thread-safe internamente, ma Arc ci evita copie costose.
///
/// Tutti gli handler ricevono questo stato con:
///   `State(state): State<AppState>`
#[derive(Clone)]
pub struct AppState {
    /// Pool di connessioni a PostgreSQL — thread-safe, condiviso
    pub pool: PgPool,
    /// Segreto per firmare/verificare i JWT
    pub jwt_secret: String,
    /// Ore di validità del token JWT
    pub jwt_expiration_hours: i64,
}

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

    // 4. Crea il pool di connessioni al DB
    let pool = config::create_db_pool(&config.database_url).await;
    tracing::info!("Connesso a PostgreSQL");

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

/// Costruisce e restituisce il Router dell'applicazione.
///
/// Separare questa funzione da main() ci permette di
/// riutilizzarla nei test di integrazione.
///
/// Struttura delle route:
///   /auth  → route pubbliche (no JWT richiesto)
///   /users → route protette (JWT obbligatorio tramite middleware)
fn build_router(state: AppState) -> Router {
    // ── Route PUBBLICHE — accessibili senza token ──────────────────────────
    let auth_routes = Router::new()
        .route("/register", post(auth_controller::register))
        // POST /auth/register → registra un nuovo utente
        .route("/login", post(auth_controller::login));
        // POST /auth/login → autentica e restituisce il JWT

    // ── Route PROTETTE — richiedono JWT valido nell'header ─────────────────
    let user_routes = Router::new()
        .route("/", get(user_controller::list))
        // GET /users → lista paginata utenti
        .route("/:id", get(user_controller::get_by_id))
        // GET /users/:id → singolo utente
        .route("/:id", patch(user_controller::update))
        // PATCH /users/:id → aggiornamento parziale
        .route("/:id", delete(user_controller::delete))
        // DELETE /users/:id → elimina utente
        //
        // Applica il middleware JWT a TUTTE le route di questo gruppo.
        // Il middleware viene eseguito PRIMA dell'handler.
        .route_layer(from_fn_with_state(
            state.clone(),
            middleware::auth_middleware,
        ));

    // ── Router principale — unisce tutto con il prefix ─────────────────────
    Router::new()
        .nest("/auth", auth_routes)  // /auth/register, /auth/login
        .nest("/users", user_routes) // /users, /users/:id
        .with_state(state) // inietta AppState in tutti gli handler
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
                    .with_target(true)    // mostra modulo::funzione
                    .with_line_number(true) // mostra numero di riga
                    .pretty()             // output leggibile
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
                    .with_ansi(true)
            )
            .with(
                // Layer 2: file — formato JSON (facile da parsare con Loki/ELK)
                tracing_subscriber::fmt::layer()
                    .json()               // output JSON strutturato
                    .with_writer(non_blocking)
                    .with_ansi(false)     // niente escape codes ANSI nel file
            )
            .init();

        tracing::info!("🚀 Modalità PROD — logging su stdout + logs/app.log");
    }
}
