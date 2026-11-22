//! archive — Espansione di un upload (file singolo o archivio ZIP)
//!
//! Ogni byte dell'upload è ostile: dimensioni dichiarate non fidate, nomi con
//! percorsi, ZIP annidati, zip bomb. Il modulo è sincrono (va chiamato in
//! `spawn_blocking`). I risultati sono consegnati a un `sink` appena letti,
//! senza accumularli; gli ZIP annidati sono copiati in un file temporaneo
//! anonimo (cancellato alla chiusura), mai tenuti in memoria.

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
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
            // Gli export Garmin contengono molti file JSON oltre alle attività
            max_entries: 50_000,
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

/// Destinazione dei risultati, chiamata una volta per ogni file o rifiuto.
type Sink<'a> = &'a mut dyn FnMut(Expanded);

/// Interruzione dell'espansione: superato il budget di voci o di byte.
struct BudgetExceeded;

/// Esito della lettura limitata di una voce.
enum ReadOutcome {
    Data(Vec<u8>),
    TooLarge,
    Failed,
}

/// Motivo per cui un archivio non viene aperto.
enum OpenError {
    /// Directory centrale assente o illeggibile
    Malformed,
    /// Troppe voci rispetto al budget residuo
    Exceeded,
}

/// Budget condiviso tra lo ZIP esterno e gli ZIP annidati.
struct Budget<'a> {
    limits: &'a ArchiveLimits,
    entries: usize,
    total_bytes: u64,
}

impl Budget<'_> {
    fn new(limits: &ArchiveLimits) -> Budget<'_> {
        Budget {
            limits,
            entries: 0,
            total_bytes: 0,
        }
    }

    fn remaining_entries(&self) -> usize {
        self.limits.max_entries.saturating_sub(self.entries)
    }

    fn remaining_bytes(&self) -> u64 {
        self.limits.max_total_bytes.saturating_sub(self.total_bytes)
    }

    /// Conta le voci di un archivio; oltre il massimo interrompe tutto.
    fn add_entries(&mut self, n: usize) -> Result<(), BudgetExceeded> {
        self.entries = self.entries.saturating_add(n);
        if self.entries > self.limits.max_entries {
            return Err(BudgetExceeded);
        }
        Ok(())
    }

    /// Legge al più `max_file_bytes + 1` byte: la dimensione dichiarata nello
    /// ZIP non è fidata, conta solo ciò che esce davvero dal decompressore
    /// (`size_hint` serve solo a prenotare memoria, comunque limitata).
    /// I byte letti si sommano al totale decompresso dell'upload.
    fn read_bounded(
        &mut self,
        reader: impl Read,
        size_hint: u64,
    ) -> Result<ReadOutcome, BudgetExceeded> {
        let cap = self.limits.max_file_bytes.saturating_add(1);
        let mut buf = Vec::with_capacity(size_hint.min(cap).try_into().unwrap_or(0));
        let read = reader.take(cap).read_to_end(&mut buf);
        self.total_bytes = self.total_bytes.saturating_add(buf.len() as u64);
        if self.total_bytes > self.limits.max_total_bytes {
            return Err(BudgetExceeded);
        }
        Ok(match read {
            Err(_) => ReadOutcome::Failed,
            Ok(_) if buf.len() as u64 > self.limits.max_file_bytes => ReadOutcome::TooLarge,
            Ok(_) => {
                // read_to_end può lasciare fino al doppio della capacità
                buf.shrink_to_fit();
                ReadOutcome::Data(buf)
            }
        })
    }

    /// Copia uno ZIP annidato in un file temporaneo, entro il budget di byte
    /// residuo. I byte del contenitore non vengono contati: lo saranno solo
    /// quelli decompressi delle sue voci. `Ok(None)` se la copia fallisce.
    fn spool(&mut self, reader: impl Read) -> Result<Option<File>, BudgetExceeded> {
        let Ok(mut tmp) = tempfile::tempfile() else {
            return Ok(None);
        };
        let limit = self.remaining_bytes().saturating_add(1);
        match io::copy(&mut reader.take(limit), &mut tmp) {
            Ok(copied) if copied >= limit => Err(BudgetExceeded),
            Ok(_) => Ok(Some(tmp)),
            Err(_) => Ok(None),
        }
    }
}

const EOCD_SIGNATURE: [u8; 4] = *b"PK\x05\x06";
const ZIP64_LOCATOR_SIGNATURE: [u8; 4] = *b"PK\x06\x07";
const ZIP64_EOCD_SIGNATURE: [u8; 4] = *b"PK\x06\x06";
const EOCD_LEN: usize = 22;
const ZIP64_LOCATOR_LEN: usize = 20;
const ZIP64_EOCD_LEN: usize = 56;
/// Il commento finale dello ZIP è lungo al più u16::MAX byte
const MAX_EOCD_SEARCH: u64 = EOCD_LEN as u64 + u16::MAX as u64;

fn le_u16(b: &[u8], at: usize) -> u64 {
    u16::from_le_bytes([b[at], b[at + 1]]) as u64
}

fn le_u64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().expect("8 byte"))
}

/// Numero di voci dichiarato da un record ZIP64, se presente prima dell'EOCD.
fn zip64_declared_entries<R: Read + Seek>(r: &mut R, eocd_pos: u64) -> io::Result<Option<u64>> {
    let Some(locator_pos) = eocd_pos.checked_sub(ZIP64_LOCATOR_LEN as u64) else {
        return Ok(None);
    };
    let mut locator = [0u8; ZIP64_LOCATOR_LEN];
    r.seek(SeekFrom::Start(locator_pos))?;
    r.read_exact(&mut locator)?;
    if locator[..4] != ZIP64_LOCATOR_SIGNATURE {
        return Ok(None);
    }
    let mut record = [0u8; ZIP64_EOCD_LEN];
    r.seek(SeekFrom::Start(le_u64(&locator, 8)))?;
    if r.read_exact(&mut record).is_err() || record[..4] != ZIP64_EOCD_SIGNATURE {
        return Ok(None);
    }
    Ok(Some(le_u64(&record, 24).max(le_u64(&record, 32))))
}

/// Numero di voci dichiarato nei record di fine directory centrale.
///
/// Si leggono solo gli ultimi byte del file, prima di `ZipArchive::new` che
/// altrimenti materializzerebbe l'intera directory centrale. Se nella coda
/// compaiono più firme EOCD (es. finte, nel commento) vale la più alta: è
/// l'ipotesi peggiore, qualunque sia quella scelta dal lettore ZIP.
/// `None` se non c'è alcun EOCD.
fn declared_entries<R: Read + Seek>(r: &mut R) -> io::Result<Option<u64>> {
    let len = r.seek(SeekFrom::End(0))?;
    let tail_len = len.min(MAX_EOCD_SEARCH);
    if tail_len < EOCD_LEN as u64 {
        return Ok(None);
    }
    let tail_start = len - tail_len;
    let mut tail = vec![0u8; tail_len as usize];
    r.seek(SeekFrom::Start(tail_start))?;
    r.read_exact(&mut tail)?;

    let mut declared: Option<u64> = None;
    for pos in 0..=(tail.len() - EOCD_LEN) {
        if tail[pos..pos + 4] != EOCD_SIGNATURE {
            continue;
        }
        // Il più alto tra i conteggi "su questo disco" e "totale"
        let mut count = le_u16(&tail[pos..], 8).max(le_u16(&tail[pos..], 10));
        // 0xFFFF indica che il valore reale è nel record ZIP64
        if count == u16::MAX as u64 {
            if let Some(z) = zip64_declared_entries(r, tail_start + pos as u64)? {
                count = count.max(z);
            }
        }
        declared = Some(declared.map_or(count, |d| d.max(count)));
    }
    Ok(declared)
}

/// Apre un archivio solo dopo aver verificato, dall'EOCD, che il numero di
/// voci dichiarato rientri nel budget residuo; poi le conta.
fn open_archive<R: Read + Seek>(
    mut reader: R,
    budget: &mut Budget<'_>,
) -> Result<ZipArchive<R>, OpenError> {
    match declared_entries(&mut reader) {
        Ok(Some(n)) if n > budget.remaining_entries() as u64 => return Err(OpenError::Exceeded),
        Ok(Some(_)) => {}
        Ok(None) | Err(_) => return Err(OpenError::Malformed),
    }
    let archive = ZipArchive::new(reader).map_err(|_| OpenError::Malformed)?;
    // Rete di sicurezza: il lettore ZIP potrebbe aver scelto un altro EOCD
    budget
        .add_entries(archive.len())
        .map_err(|_| OpenError::Exceeded)?;
    Ok(archive)
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

/// Espande un upload in uno o più file di attività, consegnandoli a `sink`
/// uno alla volta (nulla viene accumulato).
///
/// - `.fit/.gpx/.tcx` → un solo `File` (o `Rejected FILE_TOO_LARGE`);
/// - `.zip` → le voci supportate; ZIP annidati espansi un solo livello,
///   il resto (voci non supportate, directory, ZIP più profondi) ignorato;
/// - altra estensione → `Rejected UNSUPPORTED_FORMAT`.
///
/// Superato il budget di voci o di byte l'espansione si ferma con un unico
/// `Rejected ARCHIVE_TOO_LARGE` (nome dello ZIP caricato); quanto già
/// consegnato resta valido: un nuovo upload è sicuro perché l'import deduplica.
///
/// `filename` è il nome dichiarato dal client: se ne usa solo il nome base.
pub fn expand_upload(
    filename: &str,
    path: &Path,
    limits: &ArchiveLimits,
    sink: &mut dyn FnMut(Expanded),
) {
    let name = basename(filename);

    if is_zip(&name) {
        expand_zip_upload(&name, path, limits, sink);
        return;
    }

    let Some(format) = FileFormat::from_filename(&name) else {
        sink(rejected(&name, "UNSUPPORTED_FORMAT"));
        return;
    };
    let Ok(file) = File::open(path) else {
        sink(rejected(&name, "PARSE_ERROR"));
        return;
    };
    let size_hint = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut budget = Budget::new(limits);
    sink(match budget.read_bounded(file, size_hint) {
        Ok(ReadOutcome::Data(bytes)) => Expanded::File(InputFile {
            filename: name,
            format,
            bytes,
        }),
        Ok(ReadOutcome::TooLarge) => rejected(&name, "FILE_TOO_LARGE"),
        Ok(ReadOutcome::Failed) => rejected(&name, "PARSE_ERROR"),
        // Possibile solo con limiti incoerenti (max_total_bytes < max_file_bytes)
        Err(BudgetExceeded) => rejected(&name, "ARCHIVE_TOO_LARGE"),
    });
}

fn expand_zip_upload(name: &str, path: &Path, limits: &ArchiveLimits, sink: Sink<'_>) {
    let Ok(file) = File::open(path) else {
        sink(rejected(name, "PARSE_ERROR"));
        return;
    };
    let mut budget = Budget::new(limits);
    let mut archive = match open_archive(BufReader::new(file), &mut budget) {
        Ok(archive) => archive,
        Err(OpenError::Malformed) => {
            sink(rejected(name, "PARSE_ERROR"));
            return;
        }
        Err(OpenError::Exceeded) => {
            sink(rejected(name, "ARCHIVE_TOO_LARGE"));
            return;
        }
    };
    if expand_archive(&mut archive, name, 0, &mut budget, sink).is_err() {
        sink(rejected(name, "ARCHIVE_TOO_LARGE"));
    }
}

/// Espande le voci di un archivio. `depth` è 0 per lo ZIP caricato e 1 per
/// uno ZIP in esso contenuto (che non può a sua volta contenere altri ZIP).
fn expand_archive<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    archive_name: &str,
    depth: u8,
    budget: &mut Budget<'_>,
    sink: Sink<'_>,
) -> Result<(), BudgetExceeded> {
    let mut unreadable_reported = false;
    for index in 0..archive.len() {
        // Nome ed estensione si guardano prima di aprire la voce: così le voci
        // non supportate si ignorano anche se cifrate o con compressione ignota.
        // Le directory hanno nome base vuoto.
        let name = match archive.name_for_index(index) {
            Some(raw) => basename(raw),
            None => String::new(),
        };
        let nested_zip = is_zip(&name);
        let format = FileFormat::from_filename(&name);
        if name.is_empty() || (nested_zip && depth >= 1) || (!nested_zip && format.is_none()) {
            continue;
        }

        let mut entry = match archive.by_index(index) {
            Ok(entry) => entry,
            Err(_) => {
                // Voce illeggibile (es. cifrata): una sola segnalazione per archivio
                if !unreadable_reported {
                    unreadable_reported = true;
                    sink(rejected(archive_name, "PARSE_ERROR"));
                }
                continue;
            }
        };

        if nested_zip {
            let Some(spooled) = budget.spool(&mut entry)? else {
                sink(rejected(&name, "PARSE_ERROR"));
                continue;
            };
            match open_archive(BufReader::new(spooled), budget) {
                Ok(mut inner) => expand_archive(&mut inner, &name, depth + 1, budget, sink)?,
                Err(OpenError::Malformed) => sink(rejected(&name, "PARSE_ERROR")),
                Err(OpenError::Exceeded) => return Err(BudgetExceeded),
            }
        } else if let Some(format) = format {
            let size = entry.size();
            sink(match budget.read_bounded(&mut entry, size)? {
                ReadOutcome::Data(bytes) => Expanded::File(InputFile {
                    filename: name,
                    format,
                    bytes,
                }),
                ReadOutcome::TooLarge => rejected(&name, "FILE_TOO_LARGE"),
                ReadOutcome::Failed => rejected(&name, "PARSE_ERROR"),
            });
        }
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
        let mut out = Vec::new();
        expand_upload(name, &path, limits, &mut |e| out.push(e));
        out
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
        // Streaming: le prime due voci sono già state consegnate, poi si ferma
        assert_eq!(out.len(), 3, "{out:?}");
        assert_eq!(files(&out).len(), 2);
        match &out[2] {
            Expanded::Rejected { filename, code } => {
                assert_eq!((filename.as_str(), *code), ("sum.zip", "ARCHIVE_TOO_LARGE"));
            }
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    /// Byte pseudo-casuali (incomprimibili) per ZIP di dimensione prevedibile.
    fn noise(n: usize) -> Vec<u8> {
        let mut x: u32 = 0x1234_5678;
        (0..n)
            .map(|_| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (x >> 24) as u8
            })
            .collect()
    }

    #[test]
    fn nested_zip_larger_than_file_limit_expands_within_total_budget() {
        let inner = build_zip(&[
            ("1.gpx", &noise(300)),
            ("2.gpx", &noise(300)),
            ("3.gpx", &noise(300)),
        ]);
        let outer = build_zip(&[("Part1.zip", &inner)]);
        let limits = ArchiveLimits {
            max_file_bytes: 400,
            ..Default::default()
        };
        assert!(inner.len() as u64 > limits.max_file_bytes);
        let out = expand("takeout.zip", &outer, &limits);
        let names: Vec<_> = files(&out).iter().map(|f| f.filename.clone()).collect();
        assert_eq!(names, ["1.gpx", "2.gpx", "3.gpx"], "{out:?}");
    }

    #[test]
    fn nested_zip_container_bytes_are_not_counted_in_total() {
        let inner = build_zip(&[("1.gpx", &noise(300))]);
        let outer = build_zip(&[("Part1.zip", &inner)]);
        // Basta per il contenitore o per le voci, non per entrambi insieme
        let limits = ArchiveLimits {
            max_total_bytes: inner.len() as u64 + 10,
            ..Default::default()
        };
        let out = expand("takeout.zip", &outer, &limits);
        assert_eq!(files(&out).len(), 1, "{out:?}");
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn nested_zip_over_remaining_total_budget_rejected_archive_too_large() {
        let inner = build_zip(&[("1.gpx", &noise(600))]);
        let outer = build_zip(&[("ok.gpx", GPX), ("Part1.zip", &inner)]);
        let limits = ArchiveLimits {
            max_total_bytes: 200,
            ..Default::default()
        };
        let out = expand("takeout.zip", &outer, &limits);
        assert_eq!(out.len(), 2, "{out:?}");
        assert_eq!(files(&out)[0].filename, "ok.gpx");
        assert!(matches!(
            &out[1],
            Expanded::Rejected { filename, code: "ARCHIVE_TOO_LARGE" } if filename == "takeout.zip"
        ));
    }

    /// Record EOCD di 22 byte con il conteggio di voci indicato.
    fn eocd(entries: u16, cd_size: u32, cd_offset: u32) -> Vec<u8> {
        let mut b = b"PK\x05\x06".to_vec();
        b.extend_from_slice(&[0, 0, 0, 0]); // numero disco, disco della directory
        b.extend_from_slice(&entries.to_le_bytes());
        b.extend_from_slice(&entries.to_le_bytes());
        b.extend_from_slice(&cd_size.to_le_bytes());
        b.extend_from_slice(&cd_offset.to_le_bytes());
        b.extend_from_slice(&[0, 0]); // lunghezza commento
        b
    }

    #[test]
    fn eocd_declaring_too_many_entries_rejected_without_opening() {
        // Solo l'EOCD: dichiara 65535 voci ma non esiste alcuna directory centrale
        let out = expand("huge.zip", &eocd(u16::MAX, 0, 0), &ArchiveLimits::default());
        // Default 50_000 < 65535
        assert_eq!(only_rejected(&out), ("huge.zip", "ARCHIVE_TOO_LARGE"));
    }

    #[test]
    fn zip64_eocd_declaring_a_million_entries_rejected() {
        let mut zip = b"PK\x06\x06".to_vec();
        zip.extend_from_slice(&44u64.to_le_bytes()); // dimensione del record
        zip.extend_from_slice(&[45, 0, 45, 0]); // versioni
        zip.extend_from_slice(&[0; 8]); // numero disco, disco della directory
        zip.extend_from_slice(&1_000_000u64.to_le_bytes()); // voci su questo disco
        zip.extend_from_slice(&1_000_000u64.to_le_bytes()); // voci totali
        zip.extend_from_slice(&[0; 16]); // dimensione e offset della directory
        assert_eq!(zip.len(), ZIP64_EOCD_LEN);
        zip.extend_from_slice(b"PK\x06\x07"); // locator
        zip.extend_from_slice(&[0; 4]);
        zip.extend_from_slice(&0u64.to_le_bytes()); // offset del record ZIP64
        zip.extend_from_slice(&1u32.to_le_bytes());
        zip.extend_from_slice(&eocd(u16::MAX, 0, 0));

        let out = expand("zip64.zip", &zip, &ArchiveLimits::default());
        assert_eq!(only_rejected(&out), ("zip64.zip", "ARCHIVE_TOO_LARGE"));
    }

    #[test]
    fn eocd_pointing_to_missing_directory_rejected_parse_error() {
        let out = expand("lie.zip", &eocd(1, 46, 0), &ArchiveLimits::default());
        assert_eq!(only_rejected(&out), ("lie.zip", "PARSE_ERROR"));
    }

    #[test]
    fn nested_zip_with_huge_declared_entries_rejected_archive_too_large() {
        let outer = build_zip(&[("ok.gpx", GPX), ("Part1.zip", &eocd(u16::MAX, 0, 0))]);
        let out = expand("takeout.zip", &outer, &ArchiveLimits::default());
        assert_eq!(out.len(), 2, "{out:?}");
        assert!(matches!(
            &out[1],
            Expanded::Rejected { filename, code: "ARCHIVE_TOO_LARGE" } if filename == "takeout.zip"
        ));
    }

    /// Cambia il metodo di compressione di una voce (locale e centrale).
    fn patch_method(zip: &mut [u8], name: &str, method: u16) {
        let needle = name.as_bytes();
        let positions: Vec<usize> = (0..zip.len() - needle.len())
            .filter(|&i| &zip[i..i + needle.len()] == needle)
            .collect();
        // Prima occorrenza: header locale (nome a +30); seconda: directory (+46)
        let local = positions[0] - 30;
        let central = positions[1] - 46;
        zip[local + 8..local + 10].copy_from_slice(&method.to_le_bytes());
        zip[central + 10..central + 12].copy_from_slice(&method.to_le_bytes());
    }

    #[test]
    fn unreadable_entry_with_unsupported_extension_is_skipped_silently() {
        let mut zip = build_zip(&[("a.gpx", GPX), ("weird.json", b"{}")]);
        patch_method(&mut zip, "weird.json", 14);
        let out = expand("a.zip", &zip, &ArchiveLimits::default());
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(files(&out)[0].filename, "a.gpx");
    }

    #[test]
    fn unreadable_activity_entry_reports_one_parse_error() {
        let mut zip = build_zip(&[("a.gpx", GPX), ("b.gpx", GPX), ("c.tcx", GPX)]);
        patch_method(&mut zip, "b.gpx", 14);
        patch_method(&mut zip, "c.tcx", 14);
        let out = expand("a.zip", &zip, &ArchiveLimits::default());
        assert_eq!(files(&out).len(), 1, "{out:?}");
        assert_eq!(out.len(), 2, "{out:?}");
        assert!(matches!(
            &out[1],
            Expanded::Rejected {
                code: "PARSE_ERROR",
                ..
            }
        ));
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
