/// services/date_bounds.rs — Limiti delle date usate come filtro
///
/// Le date `from`/`to` di elenchi e statistiche devono avere un anno
/// plausibile: date estreme (es. `0001-01-01`, `9999-12-31`) non hanno
/// significato e porterebbero il DB fuori dai range gestiti nei calcoli.
use chrono::{Datelike, NaiveDate};

use crate::errors::AppError;
use crate::services::auth_service::validation_error;

/// Primo anno ammesso.
pub const MIN_YEAR: i32 = 1900;
/// Ultimo anno ammesso.
pub const MAX_YEAR: i32 = 2200;

/// `400 VALIDATION_FAILED` se la data è presente e il suo anno è fuori da
/// `MIN_YEAR..=MAX_YEAR`.
pub fn check_year(field: &str, date: Option<NaiveDate>) -> Result<(), AppError> {
    match date {
        Some(d) if !(MIN_YEAR..=MAX_YEAR).contains(&d.year()) => Err(validation_error(format!(
            "{field} deve avere un anno compreso tra {MIN_YEAR} e {MAX_YEAR}"
        ))),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> Option<NaiveDate> {
        NaiveDate::from_ymd_opt(y, m, d)
    }

    #[test]
    fn bounds_are_inclusive() {
        assert!(check_year("from", None).is_ok());
        assert!(check_year("from", date(1900, 1, 1)).is_ok());
        assert!(check_year("to", date(2200, 12, 31)).is_ok());
        assert!(check_year("from", date(1899, 12, 31)).is_err());
        assert!(check_year("to", date(2201, 1, 1)).is_err());
    }
}
