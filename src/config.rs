/// config.rs — Configurazione centralizzata dell'applicazione
///
/// Questo file legge le variabili d'ambiente e le mette in una struct
/// tipizzata. Centralizzare la config evita di chiamare std::env::var()
/// sparso ovunque nel codice, rendendo gli errori di config evidenti
/// all'avvio dell'app (fail-fast).

use sqlx::{postgres::PgPoolOptions, PgPool};

/// Contiene tutta la configurazione dell'app.
/// Viene creata una sola volta in main() e poi condivisa via Arc<AppState>.
pub struct Config {
    pub database_url: String,
    pub jwt_secret: String,
    pub jwt_expiration_hours: i64,
    pub server_host: String,
    pub server_port: u16,
    /// Percorso del certificato TLS (PEM). Se assente, si usa un certificato self-signed.
    pub tls_cert_path: Option<String>,
    /// Percorso della chiave privata TLS (PEM). Se assente, si usa una chiave self-signed.
    pub tls_key_path: Option<String>,
}

impl Config {
    /// Legge tutte le variabili d'ambiente necessarie.
    /// Chiama .expect() intenzionalmente: se manca una var, l'app NON deve avviarsi.
    pub fn from_env() -> Self {
        Self {
            database_url: std::env::var("DATABASE_URL")
                .expect("DATABASE_URL mancante nel .env"),

            jwt_secret: std::env::var("JWT_SECRET")
                .expect("JWT_SECRET mancante nel .env"),

            jwt_expiration_hours: std::env::var("JWT_EXPIRATION_HOURS")
                .unwrap_or_else(|_| "24".to_string())
                .parse()
                .expect("JWT_EXPIRATION_HOURS deve essere un numero"),

            server_host: std::env::var("SERVER_HOST")
                .unwrap_or_else(|_| "127.0.0.1".to_string()),

            server_port: std::env::var("SERVER_PORT")
                .unwrap_or_else(|_| "3000".to_string())
                .parse()
                .expect("SERVER_PORT deve essere un numero"),

            tls_cert_path: std::env::var("TLS_CERT_PATH").ok(),
            tls_key_path: std::env::var("TLS_KEY_PATH").ok(),
        }
    }

    /// Restituisce l'indirizzo completo host:porta
    pub fn server_address(&self) -> String {
        format!("{}:{}", self.server_host, self.server_port)
    }
}

/// Crea e restituisce il pool di connessioni al database.
/// Il pool è condiviso tra tutti i thread — sqlx lo gestisce internamente
/// in modo thread-safe.
pub async fn create_db_pool(database_url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(10)
        .connect(database_url)
        .await
        .expect("Impossibile connettersi al database")
}
