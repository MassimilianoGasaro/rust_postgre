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

    // 2. Inizializza il logger — legge RUST_LOG dall'env
    //    es. RUST_LOG=info oppure RUST_LOG=rust_postgre=debug
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "rust_postgre=debug,info".into()),
        )
        .init();

    // 3. Legge la configurazione dalle variabili d'ambiente
    let config = config::Config::from_env();

    // 4. Crea il pool di connessioni al DB
    let pool = config::create_db_pool(&config.database_url).await;
    tracing::info!("✅ Connesso a PostgreSQL");

    // 5. Costruisce l'AppState condiviso
    let state = AppState {
        pool,
        jwt_secret: config.jwt_secret,
        jwt_expiration_hours: config.jwt_expiration_hours,
    };

    // 6. Costruisce il Router
    let app = build_router(state);

    // 7. Avvia il server
    let address = config::Config::from_env().server_address();
    let listener = tokio::net::TcpListener::bind(&address)
        .await
        .expect("Impossibile avviare il server");

    tracing::info!("🚀 Server in ascolto su http://{}", address);

    axum::serve(listener, app)
        .await
        .expect("Errore nel server");
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
        // POST /auth/register  →  registra un nuovo utente
        .route("/login", post(auth_controller::login));
        // POST /auth/login     →  autentica e restituisce il JWT

    // ── Route PROTETTE — richiedono JWT valido nell'header ─────────────────
    let user_routes = Router::new()
        .route("/", get(user_controller::list))
        // GET    /users              →  lista paginata utenti
        .route("/:id", get(user_controller::get_by_id))
        // GET    /users/:id          →  singolo utente
        .route("/:id", patch(user_controller::update))
        // PATCH  /users/:id          →  aggiornamento parziale
        .route("/:id", delete(user_controller::delete))
        // DELETE /users/:id          →  elimina utente
        //
        // Applica il middleware JWT a TUTTE le route di questo gruppo.
        // Il middleware viene eseguito PRIMA dell'handler.
        .route_layer(from_fn_with_state(
            state.clone(),
            middleware::auth_middleware,
        ));

    // ── Router principale — unisce tutto con il prefix ─────────────────────
    Router::new()
        .nest("/auth", auth_routes)   // /auth/register, /auth/login
        .nest("/users", user_routes)  // /users, /users/:id
        .with_state(state)            // inietta AppState in tutti gli handler
}
