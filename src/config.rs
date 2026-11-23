/// config.rs — Configurazione centralizzata dell'applicazione
///
/// Questo file legge le variabili d'ambiente e le mette in una struct
/// tipizzata. Centralizzare la config evita di chiamare std::env::var()
/// sparso ovunque nel codice, rendendo gli errori di config evidenti
/// all'avvio dell'app (fail-fast).
use std::path::PathBuf;

use sqlx::{PgPool, postgres::PgPoolOptions};

/// Legge una variabile opzionale: assente o vuota (es. `TLS_CERT_PATH=`) → None.
fn optional_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// Interpreta un booleano da variabile d'ambiente; assente → `default`.
fn parse_bool(value: Option<&str>, default: bool) -> Option<bool> {
    match value.map(|v| v.trim().to_ascii_lowercase()) {
        None => Some(default),
        Some(v) => match v.as_str() {
            "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" => Some(false),
            _ => None,
        },
    }
}

/// Contiene tutta la configurazione dell'app.
/// Viene creata una sola volta in main() e poi condivisa via Arc<AppState>.
pub struct Config {
    pub database_url: String,
    pub jwt_secret: String,
    pub jwt_expiration_hours: i64,
    pub server_host: String,
    pub server_port: u16,
    /// Se false il server ascolta in HTTP semplice: in produzione il TLS lo termina il proxy
    /// della piattaforma (es. Render). Default true.
    pub tls_enabled: bool,
    /// Percorso del certificato TLS (PEM). Se assente, si usa un certificato self-signed.
    pub tls_cert_path: Option<String>,
    /// Percorso della chiave privata TLS (PEM). Se assente, si usa una chiave self-signed.
    pub tls_key_path: Option<String>,
    /// Origine consentita dal CORS (es. http://localhost:4200). Se assente, nessuna origine è abilitata.
    pub cors_origin: Option<String>,
    /// Directory in cui vengono salvati i file caricati.
    pub upload_dir: PathBuf,
    /// Dimensione massima del body di una richiesta di upload (byte).
    pub max_upload_bytes: usize,
    /// Dimensione massima di un singolo file caricato (byte).
    pub max_file_bytes: usize,
    /// Numero massimo di import elaborati in parallelo (su tutti gli utenti).
    pub max_concurrent_imports: usize,
}

impl Config {
    /// Legge tutte le variabili d'ambiente necessarie.
    /// Chiama .expect() intenzionalmente: se manca una var, l'app NON deve avviarsi.
    pub fn from_env() -> Self {
        Self {
            database_url: std::env::var("DATABASE_URL").expect("DATABASE_URL mancante nel .env"),

            jwt_secret: std::env::var("JWT_SECRET").expect("JWT_SECRET mancante nel .env"),

            jwt_expiration_hours: std::env::var("JWT_EXPIRATION_HOURS")
                .unwrap_or_else(|_| "24".to_string())
                .parse()
                .expect("JWT_EXPIRATION_HOURS deve essere un numero"),

            server_host: std::env::var("SERVER_HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),

            // `PORT` è la variabile impostata dalle piattaforme di hosting (Render): ha la
            // precedenza su SERVER_PORT
            server_port: optional_env("PORT")
                .or_else(|| optional_env("SERVER_PORT"))
                .unwrap_or_else(|| "3000".to_string())
                .parse()
                .expect("PORT / SERVER_PORT deve essere un numero"),

            tls_enabled: parse_bool(optional_env("TLS_ENABLED").as_deref(), true)
                .expect("TLS_ENABLED deve essere true o false"),

            tls_cert_path: optional_env("TLS_CERT_PATH"),
            tls_key_path: optional_env("TLS_KEY_PATH"),

            cors_origin: optional_env("CORS_ORIGIN"),

            upload_dir: PathBuf::from(
                std::env::var("UPLOAD_DIR").unwrap_or_else(|_| "./uploads".to_string()),
            ),

            max_upload_bytes: std::env::var("MAX_UPLOAD_BYTES")
                .unwrap_or_else(|_| "1073741824".to_string())
                .parse()
                .expect("MAX_UPLOAD_BYTES deve essere un numero"),

            max_file_bytes: std::env::var("MAX_FILE_BYTES")
                .unwrap_or_else(|_| "52428800".to_string())
                .parse()
                .expect("MAX_FILE_BYTES deve essere un numero"),

            max_concurrent_imports: std::env::var("MAX_CONCURRENT_IMPORTS")
                .unwrap_or_else(|_| "2".to_string())
                .parse()
                .expect("MAX_CONCURRENT_IMPORTS deve essere un numero"),
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

#[cfg(test)]
mod tests {
    use super::parse_bool;

    #[test]
    fn parse_bool_accepts_common_spellings() {
        assert_eq!(parse_bool(None, true), Some(true));
        assert_eq!(parse_bool(None, false), Some(false));
        for v in ["true", "TRUE", " 1 ", "yes"] {
            assert_eq!(parse_bool(Some(v), false), Some(true), "{v}");
        }
        for v in ["false", "False", "0", "no"] {
            assert_eq!(parse_bool(Some(v), true), Some(false), "{v}");
        }
        assert_eq!(parse_bool(Some("forse"), true), None);
    }
}
