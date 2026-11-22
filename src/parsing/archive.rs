//! archive — Espansione di un upload (file singolo o archivio ZIP)
//!
//! Ogni byte dell'upload è ostile: dimensioni dichiarate non fidate, nomi con
//! percorsi, ZIP annidati, zip bomb. Il modulo è sincrono (va chiamato in
//! `spawn_blocking`) e non scrive mai su disco.

use std::fs::File;
use std::io::{BufReader, Cursor, Read, Seek};
use std::path::Path;

use zip::ZipArchive;

use super::FileFormat;

/// Limiti applicati all'espansione di un upload.
#[derive(Debug, Clone)]
pub struct ArchiveLimits {
    /// Dimensione massima di un singolo file (byte, già decompresso)
    pub max_file_bytes: u64,
    /// Byte decompressi complessivi letti da un archivio
    pub max_total_bytes: u64,
    /// Numero massimo di voci esaminate in un archivio
    pub max_entries: usize,
}

impl Default for ArchiveLimits {
    fn default() -> Self {
        ArchiveLimits {
            max_file_bytes: 50 * 1024 * 1024,
            max_total_bytes: 2 * 1024 * 1024 * 1024,
            max_entries: 5000,
        }
    }
}

/// File di attività pronto per il parsing.
#[derive(Debug)]
pub struct InputFile {
    /// Solo il nome base: non va mai usato come percorso
    pub filename: String,
    pub format: FileFormat,
    pub bytes: Vec<u8>,
}

/// Esito dell'espansione per un singolo file.
#[derive(Debug)]
pub enum Expanded {
    File(InputFile),
    Rejected {
        filename: String,
        code: &'static str,
    },
}

/// Interruzione dell'espansione: superato il budget di voci o di byte.
struct BudgetExceeded;

/// Esito della lettura limitata di una voce.
enum ReadOutcome {
    Data(Vec<u8>),
    TooLarge,
    Failed,
}

/// Budget condiviso tra lo ZIP esterno e gli ZIP annidati.
struct Budget<'a> {
    limits: &'a ArchiveLimits,
    entries: usize,
    total_bytes: u64,
}

impl Budget<'_> {
    /// Conta le voci di un archivio; oltre il massimo interrompe tutto.
    fn add_entries(&mut self, n: usize) -> Result<(), BudgetExceeded> {
        self.entries = self.entries.saturating_add(n);
        if self.entries > self.limits.max_entries {
            return Err(BudgetExceeded);
        }
        Ok(())
    }

    /// Legge al più `max_file_bytes + 1` byte: la dimensione dichiarata nello
    /// ZIP non è fidata, conta solo ciò che esce davvero dal decompressore.
    /// I byte letti si sommano al totale decompresso dell'upload.
    fn read_bounded(&mut self, reader: impl Read) -> Result<ReadOutcome, BudgetExceeded> {
        let mut buf = Vec::new();
        let read = reader
            .take(self.limits.max_file_bytes.saturating_add(1))
            .read_to_end(&mut buf);
        self.total_bytes = self.total_bytes.saturating_add(buf.len() as u64);
        if self.total_bytes > self.limits.max_total_bytes {
            return Err(BudgetExceeded);
        }
        Ok(match read {
            Err(_) => ReadOutcome::Failed,
            Ok(_) if buf.len() as u64 > self.limits.max_file_bytes => ReadOutcome::TooLarge,
            Ok(_) => ReadOutcome::Data(buf),
        })
    }
}

/// Nome base di una voce: tutto ciò che segue l'ultimo `/` o `\`, senza
/// caratteri di controllo. Il risultato non è mai usato come percorso.
fn basename(name: &str) -> String {
    name.rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .collect()
}

fn is_zip(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("zip"))
}

fn rejected(filename: &str, code: &'static str) -> Expanded {
    Expanded::Rejected {
        filename: filename.to_string(),
        code,
    }
}

/// Espande un upload in uno o più file di attività.
///
/// - `.fit/.gpx/.tcx` → un solo `File` (o `Rejected FILE_TOO_LARGE`);
/// - `.zip` → le voci supportate; ZIP annidati espansi un solo livello,
///   il resto (voci non supportate, directory, ZIP più profondi) ignorato;
/// - altra estensione → `Rejected UNSUPPORTED_FORMAT`.
///
/// `filename` è il nome dichiarato dal client: se ne usa solo il nome base.
pub fn expand_upload(filename: &str, path: &Path, limits: &ArchiveLimits) -> Vec<Expanded> {
    let name = basename(filename);

    if is_zip(&name) {
        return expand_zip_upload(&name, path, limits);
    }

    let Some(format) = FileFormat::from_filename(&name) else {
        return vec![rejected(&name, "UNSUPPORTED_FORMAT")];
    };
    let Ok(file) = File::open(path) else {
        return vec![rejected(&name, "PARSE_ERROR")];
    };
    let mut budget = Budget {
        limits,
        entries: 0,
        total_bytes: 0,
    };
    match budget.read_bounded(file) {
        Ok(ReadOutcome::Data(bytes)) => vec![Expanded::File(InputFile {
            filename: name,
            format,
            bytes,
        })],
        Ok(ReadOutcome::TooLarge) => vec![rejected(&name, "FILE_TOO_LARGE")],
        Ok(ReadOutcome::Failed) => vec![rejected(&name, "PARSE_ERROR")],
        // Possibile solo con limiti incoerenti (max_total_bytes < max_file_bytes)
        Err(BudgetExceeded) => vec![rejected(&name, "ARCHIVE_TOO_LARGE")],
    }
}

fn expand_zip_upload(name: &str, path: &Path, limits: &ArchiveLimits) -> Vec<Expanded> {
    let Ok(file) = File::open(path) else {
        return vec![rejected(name, "PARSE_ERROR")];
    };
    let Ok(mut archive) = ZipArchive::new(BufReader::new(file)) else {
        return vec![rejected(name, "PARSE_ERROR")];
    };
    let mut budget = Budget {
        limits,
        entries: 0,
        total_bytes: 0,
    };
    let mut out = Vec::new();
    match expand_archive(&mut archive, name, 0, &mut budget, &mut out) {
        Ok(()) => out,
        // Un solo esito per l'intero archivio: il contenuto parziale è scartato
        Err(BudgetExceeded) => vec![rejected(name, "ARCHIVE_TOO_LARGE")],
    }
}

/// Espande le voci di un archivio. `depth` è 0 per lo ZIP caricato e 1 per
/// uno ZIP in esso contenuto (che non può a sua volta contenere altri ZIP).
fn expand_archive<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    archive_name: &str,
    depth: u8,
    budget: &mut Budget<'_>,
    out: &mut Vec<Expanded>,
) -> Result<(), BudgetExceeded> {
    budget.add_entries(archive.len())?;

    let mut unreadable_reported = false;
    for index in 0..archive.len() {
        let mut entry = match archive.by_index(index) {
            Ok(entry) => entry,
            Err(_) => {
                // Voce illeggibile (es. cifrata): una sola segnalazione per archivio
                if !unreadable_reported {
                    unreadable_reported = true;
                    out.push(rejected(archive_name, "PARSE_ERROR"));
                }
                continue;
            }
        };
        if entry.is_dir() {
            continue;
        }
        let name = basename(entry.name());
        if name.is_empty() {
            continue;
        }

        if is_zip(&name) {
            if depth >= 1 {
                continue;
            }
            match budget.read_bounded(&mut entry)? {
                ReadOutcome::Data(bytes) => match ZipArchive::new(Cursor::new(bytes)) {
                    Ok(mut inner) => expand_archive(&mut inner, &name, depth + 1, budget, out)?,
                    Err(_) => out.push(rejected(&name, "PARSE_ERROR")),
                },
                ReadOutcome::TooLarge => out.push(rejected(&name, "FILE_TOO_LARGE")),
                ReadOutcome::Failed => out.push(rejected(&name, "PARSE_ERROR")),
            }
        } else if let Some(format) = FileFormat::from_filename(&name) {
            match budget.read_bounded(&mut entry)? {
                ReadOutcome::Data(bytes) => out.push(Expanded::File(InputFile {
                    filename: name,
                    format,
                    bytes,
                })),
                ReadOutcome::TooLarge => out.push(rejected(&name, "FILE_TOO_LARGE")),
                ReadOutcome::Failed => out.push(rejected(&name, "PARSE_ERROR")),
            }
        }
        // Estensione non supportata: ignorata in silenzio
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    const GPX: &[u8] = b"<gpx/>";

    /// Costruisce uno ZIP in memoria; le voci terminanti con '/' sono directory.
    fn build_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in entries {
            if name.ends_with('/') {
                writer.add_directory(*name, options).unwrap();
            } else {
                writer.start_file(*name, options).unwrap();
                writer.write_all(data).unwrap();
            }
        }
        writer.finish().unwrap().into_inner()
    }

    /// Scrive i byte in un file temporaneo e lancia l'espansione.
    fn expand(name: &str, bytes: &[u8], limits: &ArchiveLimits) -> Vec<Expanded> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("upload.bin");
        std::fs::write(&path, bytes).unwrap();
        expand_upload(name, &path, limits)
    }

    fn files(out: &[Expanded]) -> Vec<&InputFile> {
        out.iter()
            .filter_map(|e| match e {
                Expanded::File(f) => Some(f),
                _ => None,
            })
            .collect()
    }

    fn only_rejected(out: &[Expanded]) -> (&str, &'static str) {
        assert_eq!(out.len(), 1, "{out:?}");
        match &out[0] {
            Expanded::Rejected { filename, code } => (filename.as_str(), *code),
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    #[test]
    fn plain_gpx_is_single_file() {
        let out = expand("run.GPX", GPX, &ArchiveLimits::default());
        let f = files(&out);
        assert_eq!(out.len(), 1);
        assert_eq!(f[0].filename, "run.GPX");
        assert_eq!(f[0].format, FileFormat::Gpx);
        assert_eq!(f[0].bytes, GPX);
    }

    #[test]
    fn plain_file_over_limit_rejected_file_too_large() {
        let limits = ArchiveLimits {
            max_file_bytes: 1024,
            ..Default::default()
        };
        let out = expand("big.fit", &[0u8; 2048], &limits);
        assert_eq!(only_rejected(&out), ("big.fit", "FILE_TOO_LARGE"));
    }

    #[test]
    fn unknown_extension_rejected_unsupported() {
        let out = expand("notes.txt", b"hello", &ArchiveLimits::default());
        assert_eq!(only_rejected(&out), ("notes.txt", "UNSUPPORTED_FORMAT"));
    }

    #[test]
    fn zip_with_activity_and_json_keeps_only_activity() {
        let zip = build_zip(&[
            ("dir/", b""),
            ("dir/a.gpx", GPX),
            ("dir/summary.json", b"{}"),
            ("b.TCX", b"<x/>"),
            ("c.FIT", b"fit"),
        ]);
        let out = expand("export.zip", &zip, &ArchiveLimits::default());
        let names: Vec<_> = files(&out).iter().map(|f| f.filename.clone()).collect();
        assert_eq!(names, ["a.gpx", "b.TCX", "c.FIT"]);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn import_nested_zip_one_level() {
        let deepest = build_zip(&[("deep.gpx", GPX)]);
        let inner = build_zip(&[
            ("one.tcx", b"<a/>"),
            ("two.tcx", b"<b/>"),
            ("Deeper.zip", &deepest),
        ]);
        let outer = build_zip(&[(
            "DI_CONNECT/DI-Connect-Uploaded-Files/UploadedFiles_0-_Part1.zip",
            &inner,
        )]);
        let out = expand("takeout.zip", &outer, &ArchiveLimits::default());
        let names: Vec<_> = files(&out).iter().map(|f| f.filename.clone()).collect();
        assert_eq!(out.len(), 2, "{out:?}");
        assert_eq!(names, ["one.tcx", "two.tcx"]);
    }

    #[test]
    fn nested_zip_over_file_limit_rejected_with_its_basename() {
        let inner = build_zip(&[("one.tcx", &[b'x'; 4096])]);
        let outer = build_zip(&[("a/Part1.zip", &inner)]);
        let limits = ArchiveLimits {
            max_file_bytes: inner.len() as u64 - 1,
            ..Default::default()
        };
        let out = expand("takeout.zip", &outer, &limits);
        assert_eq!(only_rejected(&out), ("Part1.zip", "FILE_TOO_LARGE"));
    }

    #[test]
    fn nested_zip_entries_share_the_entry_budget() {
        let inner = build_zip(&[("1.gpx", GPX), ("2.gpx", GPX), ("3.gpx", GPX)]);
        let outer = build_zip(&[("Part1.zip", &inner)]);
        let limits = ArchiveLimits {
            max_entries: 3,
            ..Default::default()
        };
        let out = expand("takeout.zip", &outer, &limits);
        assert_eq!(only_rejected(&out), ("takeout.zip", "ARCHIVE_TOO_LARGE"));
    }

    #[test]
    fn zip_entry_over_limit_rejected_file_too_large() {
        // 1 MiB di zeri compresso in pochi byte: la dimensione dichiarata non conta
        let zip = build_zip(&[("ok.gpx", GPX), ("bomb.gpx", &vec![0u8; 1 << 20])]);
        let limits = ArchiveLimits {
            max_file_bytes: 1024,
            ..Default::default()
        };
        let out = expand("a.zip", &zip, &limits);
        assert_eq!(out.len(), 2);
        assert_eq!(files(&out)[0].filename, "ok.gpx");
        match &out[1] {
            Expanded::Rejected { filename, code } => {
                assert_eq!((filename.as_str(), *code), ("bomb.gpx", "FILE_TOO_LARGE"));
            }
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    #[test]
    fn zip_too_many_entries_rejected() {
        let zip = build_zip(&[
            ("1.gpx", GPX),
            ("2.gpx", GPX),
            ("3.gpx", GPX),
            ("4.gpx", GPX),
        ]);
        let limits = ArchiveLimits {
            max_entries: 3,
            ..Default::default()
        };
        let out = expand("many.zip", &zip, &limits);
        assert_eq!(only_rejected(&out), ("many.zip", "ARCHIVE_TOO_LARGE"));
    }

    #[test]
    fn zip_total_bytes_over_limit_rejected_archive_too_large() {
        let zip = build_zip(&[
            ("1.gpx", &[b'a'; 600]),
            ("2.gpx", &[b'b'; 600]),
            ("3.gpx", &[b'c'; 600]),
        ]);
        let limits = ArchiveLimits {
            max_total_bytes: 1500,
            ..Default::default()
        };
        let out = expand("sum.zip", &zip, &limits);
        assert_eq!(only_rejected(&out), ("sum.zip", "ARCHIVE_TOO_LARGE"));
    }

    #[test]
    fn zip_entry_with_dotdot_path_returns_basename() {
        let zip = build_zip(&[
            ("../../evil.gpx", GPX),
            ("..\\..\\win.tcx", b"<x/>"),
            ("/abs/olute.fit", b"f"),
            ("trailing/.gpx/", b""),
        ]);
        let out = expand("a.zip", &zip, &ArchiveLimits::default());
        let names: Vec<_> = files(&out).iter().map(|f| f.filename.clone()).collect();
        assert_eq!(names, ["evil.gpx", "win.tcx", "olute.fit"]);
    }

    #[test]
    fn zip_entry_name_control_chars_are_stripped() {
        let zip = build_zip(&[("a\0b\nc.gpx", GPX)]);
        let out = expand("a.zip", &zip, &ArchiveLimits::default());
        assert_eq!(files(&out)[0].filename, "abc.gpx");
    }

    #[test]
    fn corrupt_zip_rejected_parse_error() {
        let out = expand(
            "broken.zip",
            b"PK\x03\x04 this is not a zip",
            &ArchiveLimits::default(),
        );
        assert_eq!(only_rejected(&out), ("broken.zip", "PARSE_ERROR"));
    }

    #[test]
    fn zip_extension_is_case_insensitive_and_path_is_stripped() {
        let zip = build_zip(&[("a.gpx", GPX)]);
        let out = expand("C:\\Users\\x\\Export.ZIP", &zip, &ArchiveLimits::default());
        assert_eq!(files(&out).len(), 1);
    }
}
