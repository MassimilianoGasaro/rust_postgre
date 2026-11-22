/// dto/import.rs — DTO degli import di file
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::models::import::{Import, ImportItem};

/// Stato di un import. `items` è assente nell'elenco (`GET /imports`).
#[derive(Debug, Serialize)]
pub struct ImportDto {
    pub id: i64,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<ImportItemDto>>,
}

/// Esito di un singolo file dell'import.
#[derive(Debug, Serialize)]
pub struct ImportItemDto {
    pub filename: String,
    pub status: String,
    /// Attività creata (`imported`) o già esistente (`duplicate`)
    pub activity_id: Option<i64>,
    /// Motivo del fallimento (`failed`), es. `PARSE_ERROR`, `UNSUPPORTED_FORMAT`
    pub error_code: Option<String>,
}

impl ImportDto {
    /// Import con l'elenco dei suoi item.
    pub fn with_items(import: Import, items: Vec<ImportItem>) -> Self {
        ImportDto {
            items: Some(items.into_iter().map(ImportItemDto::from).collect()),
            ..ImportDto::from(import)
        }
    }
}

/// Import senza item (per l'elenco).
impl From<Import> for ImportDto {
    fn from(i: Import) -> Self {
        ImportDto {
            id: i.id,
            status: i.status,
            created_at: i.created_at,
            finished_at: i.finished_at,
            items: None,
        }
    }
}

impl From<ImportItem> for ImportItemDto {
    fn from(i: ImportItem) -> Self {
        ImportItemDto {
            filename: i.filename,
            status: i.status,
            activity_id: i.activity_id,
            error_code: i.error_code,
        }
    }
}
