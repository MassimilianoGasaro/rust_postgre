//! storage — Salvataggio dei file originali caricati dagli utenti
//!
//! I file vengono salvati come `{upload_dir}/{user_id}/{sha256}.{ext}`: il nome
//! dipende solo dal contenuto, mai da stringhe fornite dall'utente.

use std::io::{Error, ErrorKind};
use std::path::{Component, Path};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

use crate::parsing::FileFormat;

/// Contatore per rendere unico il nome del file temporaneo.
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// SHA-256 del contenuto, in esadecimale minuscolo.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Salva il file e restituisce il percorso relativo `"{user_id}/{sha256}.{ext}"`.
///
/// Crea la cartella dell'utente se manca. La scrittura avviene su un file
/// temporaneo nella stessa cartella, poi rinominato: un lettore non vede mai
/// un file a metà. Se il file esiste già (stesso hash) viene sovrascritto.
pub async fn save(
    upload_dir: &Path,
    user_id: i64,
    sha256: &str,
    format: FileFormat,
    bytes: &[u8],
) -> std::io::Result<String> {
    // L'hash finisce nel percorso: deve essere davvero un hash
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::new(ErrorKind::InvalidInput, "invalid sha256"));
    }

    let user_dir = upload_dir.join(user_id.to_string());
    tokio::fs::create_dir_all(&user_dir).await?;

    let relative = format!("{user_id}/{sha256}.{}", format.as_str());
    let target = upload_dir.join(&relative);
    let tmp = user_dir.join(format!(
        "{sha256}.{}.{}.tmp",
        std::process::id(),
        TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));

    if let Err(e) = tokio::fs::write(&tmp, bytes).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e);
    }
    if let Err(e) = tokio::fs::rename(&tmp, &target).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e);
    }
    Ok(relative)
}

/// Elimina un file salvato; se non esiste più l'operazione riesce comunque.
///
/// Il percorso deve essere relativo e senza `..`: non si esce da `upload_dir`.
pub async fn remove(upload_dir: &Path, relative: &str) -> std::io::Result<()> {
    let safe = !relative.is_empty()
        && !relative.contains('\\')
        && Path::new(relative)
            .components()
            .all(|c| matches!(c, Component::Normal(_)));
    if !safe {
        return Err(Error::new(ErrorKind::InvalidInput, "invalid relative path"));
    }
    match tokio::fs::remove_file(upload_dir.join(relative)).await {
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[tokio::test]
    async fn save_then_remove_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let sha = sha256_hex(b"contenuto");

        let rel = save(dir.path(), 7, &sha, FileFormat::Gpx, b"contenuto")
            .await
            .unwrap();
        assert_eq!(rel, format!("7/{sha}.gpx"));
        assert_eq!(std::fs::read(dir.path().join(&rel)).unwrap(), b"contenuto");

        // Stesso contenuto: la sovrascrittura è ammessa
        let again = save(dir.path(), 7, &sha, FileFormat::Gpx, b"contenuto")
            .await
            .unwrap();
        assert_eq!(again, rel);

        // Nessun file temporaneo residuo
        let leftovers: Vec<_> = std::fs::read_dir(dir.path().join("7"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(leftovers.len(), 1);

        remove(dir.path(), &rel).await.unwrap();
        assert!(!dir.path().join(&rel).exists());
        // Già assente: NotFound ignorato
        remove(dir.path(), &rel).await.unwrap();
    }

    #[tokio::test]
    async fn save_rejects_non_hex_sha() {
        let dir = tempfile::tempdir().unwrap();
        let err = save(dir.path(), 1, "../../etc/x", FileFormat::Fit, b"x")
            .await
            .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[tokio::test]
    async fn remove_rejects_paths_outside_upload_dir() {
        let dir = tempfile::tempdir().unwrap();
        for bad in ["../x.gpx", "/etc/passwd", "1/../../x", "C:\\x.gpx", ""] {
            let err = remove(dir.path(), bad).await.unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput, "{bad}");
        }
    }
}
