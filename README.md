# rust_postgre

API REST in Rust (axum + sqlx + PostgreSQL) per una piattaforma di allenamento sportivo.
Fase 1: autenticazione, profilo, relazione allenatore-atleta, import di file di attività
(FIT, TCX, GPX, anche dentro uno ZIP), elenco attività e statistiche per intervallo di date.

## Prerequisiti

- [Docker](https://www.docker.com/) (per PostgreSQL)
- [Rust](https://rustup.rs/) 1.88 o successivo (edition 2024; `rust-version` in `Cargo.toml`)
- `sqlx-cli`:

  ```
  cargo install sqlx-cli --no-default-features --features rustls,postgres
  ```

## Avvio in locale

```
copy .env.example .env      # su Linux/macOS: cp .env.example .env
docker compose up -d
sqlx migrate run
cargo run
```

Le macro `sqlx::query!` verificano le query a compile time. Con `DATABASE_URL` impostato le
controllano sul database (che deve essere **attivo e migrato**); senza, usano la cache offline
versionata in `.sqlx/`, quindi il progetto compila anche senza database (es. editor, CI).
Dopo ogni nuova migration eseguire `sqlx migrate run`; dopo aver aggiunto o modificato una query
rigenerare la cache con `cargo sqlx prepare -- --all-targets` e committare `.sqlx/`.

Il server ascolta su `https://127.0.0.1:3000` (host e porta da `SERVER_HOST` / `SERVER_PORT`).
Se `TLS_CERT_PATH` e `TLS_KEY_PATH` sono vuoti viene generato un certificato autofirmato:
con `curl` usare `-k`, ad esempio:

```
curl -k https://127.0.0.1:3000/api/health
```

Per cancellare il database di sviluppo: `docker compose down -v`.

All'avvio il server chiude come `failed` gli import rimasti `processing` (item
`IMPORT_INTERRUPTED`) e svuota `UPLOAD_DIR/tmp`: entrambe le operazioni assumono **una sola
istanza** del server sullo stesso database e sulla stessa cartella di upload.

### Configurazione (`.env`)

| Variabile | Descrizione |
|---|---|
| `DATABASE_URL` | Connessione a PostgreSQL |
| `JWT_SECRET` | Segreto di firma dei token (almeno 32 caratteri) |
| `JWT_EXPIRATION_HOURS` | Durata del token in ore |
| `SERVER_HOST`, `SERVER_PORT` | Indirizzo di ascolto |
| `PORT` | Porta impostata dalla piattaforma di hosting (Render); se presente ha la precedenza su `SERVER_PORT` |
| `TLS_ENABLED` | `false` = HTTP semplice, quando il TLS lo termina un proxy (default `true`) |
| `CORS_ORIGIN` | Origine ammessa dal CORS |
| `UPLOAD_DIR` | Cartella dei file importati |
| `MAX_UPLOAD_BYTES` | Dimensione massima del corpo multipart (default 1 GiB) |
| `MAX_FILE_BYTES` | Dimensione massima di un singolo file, anche dentro uno ZIP (default 50 MiB) |
| `MAX_CONCURRENT_IMPORTS` | Import elaborati in parallelo su tutto il server (default 2); gli altri restano `processing` in attesa |
| `TLS_CERT_PATH`, `TLS_KEY_PATH` | Certificato e chiave PEM; se vuoti, certificato autofirmato |

Limiti degli archivi ZIP: al massimo 50000 voci e 2 GiB non compressi in totale.

## Test

```
cargo test
cargo test --test real_files -- --ignored
```

I test di integrazione usano `#[sqlx::test]`: per ogni test viene creato (e poi eliminato) un
database temporaneo sul server PostgreSQL di sviluppo indicato da `DATABASE_URL`, con le
migration già applicate; il database di sviluppo non viene toccato. Il secondo comando
esegue i test sui file reali di `docs/files/`.

`docs/files/` contiene file di attività personali (con posizioni GPS): la cartella è locale
e ignorata da git (come `uploads/` e `.env`), non va mai committata.

Controlli di qualità:

```
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

## Deploy (Render + Neon)

Il backend gira su [Render](https://render.com) (piano gratuito) come immagine Docker
(`Dockerfile`); il database è un PostgreSQL gestito su [Neon](https://neon.tech) (piano gratuito).
Il servizio è descritto in `render.yaml`: deploy dal branch `main`, solo quando la CI del commit
è verde (`autoDeployTrigger: checksPass`), health check su `/api/health`.

Nel container il server ascolta in HTTP (`TLS_ENABLED=false`, `SERVER_HOST=0.0.0.0`) sulla porta
`PORT` assegnata da Render: l'HTTPS pubblico lo gestisce Render. Le migration vengono applicate
all'avvio.

Configurazione una tantum:

1. Su Neon crea un progetto (regione Europa, es. Frankfurt) e copia la connection string,
   nella forma `postgres://...?sslmode=require`.
2. Su Render: **New → Blueprint**, collega il repository e conferma il servizio
   `rust-postgre-api` letto da `render.yaml`. Alla richiesta inserisci:
   - `DATABASE_URL`: la connection string di Neon;
   - `CORS_ORIGIN`: l'indirizzo del client su Cloudflare Pages
     (es. `https://rust-postgre-client.pages.dev`, senza `/` finale).

   `JWT_SECRET` viene generato da Render.
3. L'indirizzo pubblico sarà `https://rust-postgre-api.onrender.com` (o simile, se il nome è
   occupato): deve coincidere con `apiBaseUrl` in `environment.prod.ts` del client.

Limiti del piano gratuito da tenere presenti:

- il servizio si sospende dopo circa 15 minuti senza richieste: la prima richiesta successiva
  impiega fino a un minuto; un import in corso viene interrotto e alla ripartenza chiuso come
  `failed` (`IMPORT_INTERRUPTED`);
- il disco è effimero: i file originali caricati in `UPLOAD_DIR` si perdono a ogni deploy o
  riavvio. Oggi vengono solo archiviati (le attività sono nel database), quindi non ci sono
  effetti visibili; per conservarli servirà uno storage esterno (es. Cloudflare R2);
- 512 MB di RAM: `render.yaml` riduce `MAX_UPLOAD_BYTES` a 256 MiB e gli import paralleli a 1.

Per provare l'immagine in locale (con il Postgres di `docker compose`):

```
docker build -t rust-postgre-api .
docker run --rm -p 8080:8080 -e DATABASE_URL=postgres://myuser:mypassword@host.docker.internal:5432/myapp -e JWT_SECRET=<almeno 32 caratteri> rust-postgre-api
curl http://localhost:8080/api/health
```

## API

Tutte le route sono sotto il prefisso `/api`. Le route protette richiedono l'header
`Authorization: Bearer <token>` (il token si ottiene da register o login).

| Metodo | Percorso | Ruolo | Descrizione |
|---|---|---|---|
| GET | `/api/health` | pubblica | Verifica che il servizio sia attivo |
| POST | `/api/auth/register` | pubblica | Registra un account (`athlete` o `coach`) e restituisce il JWT |
| POST | `/api/auth/login` | pubblica | Autentica con email e password e restituisce il JWT |
| GET | `/api/users/me` | atleta, coach | Account e profilo dell'utente autenticato |
| PATCH | `/api/users/me` | atleta, coach | Aggiornamento parziale di account e profilo (campo assente = invariato); per cambiare `email` o `password` serve `current_password` |
| DELETE | `/api/users/me` | atleta, coach | Elimina il proprio account (204); body `{"current_password": "..."}` |
| POST | `/api/coach-links` | atleta | L'atleta richiede il collegamento a un coach |
| GET | `/api/coach-links?status=` | atleta, coach | Elenco dei propri link (atleta) o di quelli ricevuti (coach), filtrabile per stato |
| POST | `/api/coach-links/:id/accept` | coach | Il coach accetta una richiesta in attesa |
| POST | `/api/coach-links/:id/reject` | coach | Il coach rifiuta una richiesta in attesa |
| POST | `/api/coach-links/:id/cancel` | atleta | L'atleta annulla una richiesta ancora in attesa |
| POST | `/api/coach-links/:id/revoke` | atleta | L'atleta revoca un collegamento attivo |
| GET | `/api/athletes` | coach | Atleti collegati al coach con link attivo |
| POST | `/api/imports` | atleta | Upload multipart (campo `files` ripetibile; FIT, TCX, GPX o ZIP); elaborato in background, risponde 202; `409 IMPORT_IN_PROGRESS` se l'utente ha già un import in corso |
| GET | `/api/imports` | atleta, coach | Ultimi 20 import dell'utente |
| GET | `/api/imports/:id` | atleta, coach | Dettaglio di un import con l'esito di ogni file (`imported`, `duplicate`, errore) |
| GET | `/api/activities` | atleta, coach | Elenco paginato con filtri (`from`, `to`, `sport`, `page`, `per_page`, `athlete_id`) |
| DELETE | `/api/activities/:id` | atleta, coach | Elimina un'attività propria (204) |
| GET | `/api/stats/summary` | atleta, coach | Totali, dettaglio per sport e serie per periodo in un intervallo (`from` e `to` obbligatori; `sport`, `granularity`, `athlete_id` opzionali) |

Note:

- Un coach può leggere attività e statistiche di un atleta passando `athlete_id`, solo se esiste
  un link attivo; negli altri casi risponde `403 ATHLETE_ACCESS_DENIED`.
- Un file è considerato duplicato se lo stesso utente ha già un'attività con lo stesso sport
  normalizzato e inizio entro 60 secondi; lo sport `other` (sport non riconosciuto nel file) è
  compatibile con qualsiasi sport.
- `current_password` mancante → `400 VALIDATION_FAILED`; errata → `401 INVALID_CREDENTIALS`.
- Il token di un account eliminato o disattivato → `401 UNAUTHORIZED`.
- `from`/`to` devono avere un anno tra 1900 e 2200 (altrimenti `400 VALIDATION_FAILED`).
- Route inesistente → `404 NOT_FOUND`; metodo non previsto → `405 METHOD_NOT_ALLOWED`.
- Tutte le risposte di errore hanno la forma:

  ```json
  {"error": "<messaggio>", "code": "<CODICE>", "status": 400}
  ```

### Esempio

```
curl -k -X POST https://127.0.0.1:3000/api/auth/register \
  -H "content-type: application/json" \
  -d '{"email":"atleta@example.com","username":"atleta","password":"<password>","role":"athlete"}'

curl -k -X POST https://127.0.0.1:3000/api/imports \
  -H "Authorization: Bearer <token>" \
  -F files=@attivita.fit -F files=@altra.tcx

curl -k "https://127.0.0.1:3000/api/stats/summary?from=2026-01-01&to=2026-12-31" \
  -H "Authorization: Bearer <token>"
```

## Contribuire e licenza

Come contribuire (branch, commit, controlli prima della PR) è descritto in
[CONTRIBUTING.md](CONTRIBUTING.md). Il progetto è distribuito con licenza [MIT](LICENSE).
