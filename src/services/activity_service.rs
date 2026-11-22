/// services/activity_service.rs — Logica di business delle attività
use std::path::Path;

use sqlx::PgPool;

use crate::repositories::activity_repository;
use crate::storage;

/// Rimuove il file originale salvato se nessuna attività lo referenzia.
///
/// Gli errori vengono solo loggati: un file orfano non deve far fallire
/// l'operazione principale.
pub async fn remove_file_if_unreferenced(pool: &PgPool, upload_dir: &Path, file_path: &str) {
    match activity_repository::file_path_in_use(pool, file_path).await {
        Ok(false) => {
            if let Err(e) = storage::remove(upload_dir, file_path).await {
                tracing::warn!("Rimozione del file {file_path} fallita: {e}");
            }
        }
        Ok(true) => {}
        Err(e) => tracing::warn!("Controllo riferimenti al file {file_path} fallito: {e}"),
    }
}
