# Contribuire

Grazie per l'interesse! Questo è il backend (Rust, axum, sqlx, PostgreSQL) della piattaforma di
allenamento; il client Angular è in un repository separato:
https://github.com/MassimilianoGasaro/rust_postgre_client

Il repository è privato: per contribuire serve un invito come collaboratore.

## Prima di iniziare

- Per modifiche non banali apri prima una **issue** e descrivi cosa vuoi fare: si evita di
  lavorare su qualcosa che poi non può essere accettato.
- Per preparare l'ambiente (Docker, Rust, `sqlx-cli`, `.env`) segui il [README](README.md).

## Branch

| Branch | Ruolo |
|---|---|
| `main` | produzione: ogni merge viene pubblicato su Render |
| `develop` | integrazione: base di partenza per ogni modifica |
| `feature/<descrizione>` | nuove funzionalità, da `develop` |
| `fix/<descrizione>` | correzioni, da `develop` |
| `hotfix/<descrizione>` | correzioni urgenti in produzione, da `main` |

`main` e `develop` sono protetti: non si fa push diretto, si passa sempre da una Pull Request con
la CI verde.

## Flusso di lavoro

```
git switch develop
git pull
git switch -c feature/nome-breve
# ... modifiche e commit ...
git push -u origin feature/nome-breve
```

Poi apri una Pull Request verso **`develop`** e compila il template.

## Messaggi di commit

Si usa [Conventional Commits](https://www.conventionalcommits.org/): `tipo: descrizione` in
inglese, all'imperativo.

- `feat:` nuova funzionalità
- `fix:` correzione di un bug
- `docs:`, `test:`, `refactor:`, `chore:`, `ci:`, `build:`, `style:`

Una modifica incompatibile dell'API si segnala con `!` (es. `feat!: rename stats endpoint`).

## Controlli prima della PR

La CI esegue gli stessi controlli: falliscono lì se falliscono in locale.

```
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

- **Query sqlx**: dopo aver aggiunto o modificato una query `query!`, rigenera la cache offline con
  `cargo sqlx prepare -- --all-targets` e committa `.sqlx/` (la CI compila senza database).
- **Migration**: una nuova migration è un nuovo file in `migrations/`; non modificare quelle già
  applicate (sqlx ne verifica il checksum). I file `.sql` restano con fine riga LF
  (`.gitattributes`).
- **Test**: ogni nuova funzionalità o correzione arriva con i suoi test (`#[sqlx::test]` per
  quelli che usano il database).
- **API**: se cambi un endpoint, aggiorna la tabella nel README e verifica l'impatto sul client.

## Cosa non committare

- `.env` e qualsiasi segreto (token, password, connection string);
- `docs/files/` e qualsiasi file di attività reale: contengono posizioni GPS e dati personali;
- `uploads/`, `target/`, `logs/`.

## Licenza

Contribuendo accetti che il tuo contributo sia distribuito con la [licenza MIT](LICENSE) del
progetto.
