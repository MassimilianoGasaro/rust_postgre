# Rilasciare una nuova versione

Procedura per pubblicare una versione del backend: cambio di versione, rilascio in produzione,
tag e Release su GitHub. Il client ha una procedura analoga nel suo repository
(`RELEASING.md` di `rust_postgre_client`); backend e client hanno versioni indipendenti.

Nei comandi, `X.Y.Z` è la nuova versione (es. `0.2.0`).

## Panoramica

```
release/vX.Y.Z ──PR──▶ develop ──PR──▶ main ──▶ Render pubblica ──▶ tag vX.Y.Z
                                         │
                                         └──PR──▶ develop (riallineamento)
```

`main` e `develop` sono protetti: ogni passaggio avviene con una Pull Request e la CI verde.
Ogni merge su `main` viene pubblicato su Render automaticamente, dopo che la CI del commit è
verde.

## 1. Scegli il numero di versione

Si segue il [Semantic Versioning](https://semver.org/lang/it/), `MAJOR.MINOR.PATCH`:

| Cosa contiene il rilascio | Cosa aumenta | Esempio |
|---|---|---|
| solo correzioni, documentazione, manutenzione (`fix:`, `docs:`, `chore:` …) | PATCH | `0.2.3 → 0.2.4` |
| almeno una funzionalità nuova (`feat:`) | MINOR | `0.2.4 → 0.3.0` |
| una modifica incompatibile dell'API (`feat!:`, `BREAKING CHANGE`) | MAJOR (in `0.x`: MINOR) | `1.4.0 → 2.0.0` |

Quando un numero sale, quelli alla sua destra tornano a 0.

Ultimo tag e commit da rilasciare:

```
git switch develop
git pull
git fetch --tags
git describe --tags --abbrev=0
git log --oneline v0.1.0..develop
```

`git describe` restituisce l'ultima versione rilasciata: usala al posto di `v0.1.0` nel comando
successivo, che elenca i commit non ancora rilasciati.

### Compatibilità con il client

Il client dipende dall'API: la versione minima dell'API richiesta da ogni versione del client è
nella tabella "Compatibilità con l'API" del README del client. Prima di rilasciare, controlla:

- **solo aggiunte** (nuovi endpoint, nuovi campi nelle risposte, parametri opzionali): i client
  già pubblicati continuano a funzionare, nessuna azione;
- **modifiche incompatibili** (campo rimosso o rinominato, unità o formato cambiati, endpoint
  eliminato): i client già pubblicati smettono di funzionare. Conviene procedere in due passi:
  l'API supporta per un periodo sia il formato vecchio sia il nuovo, il client passa al nuovo, e
  il formato vecchio si toglie in un rilascio successivo. Se non è possibile, scrivilo nella
  Release (punto 5), ad esempio: "Non compatibile con client < v0.3.0".

**Ordine dei rilasci:** quando una versione del client richiede una nuova versione dell'API,
rilascia **prima l'API** e poi il client, così il client nuovo trova già pubblicato ciò che usa.

`GET /api/health` restituisce la versione pubblicata (`"version": "X.Y.Z"`), utile per
verificare cosa è online.

## 2. Aggiorna la versione nel progetto

La versione in `Cargo.toml` deve coincidere con il tag.

```
git switch develop
git pull
git switch -c release/vX.Y.Z
```

Modifica `version` in `Cargo.toml`:

```toml
[package]
version = "X.Y.Z"
```

Poi aggiorna `Cargo.lock` (contiene anche la versione del pacchetto) e verifica:

```
cargo check
git diff --stat
```

`git diff --stat` deve mostrare solo `Cargo.toml` e `Cargo.lock`. Se dimentichi `Cargo.lock`,
la build Docker fallisce (usa `--locked`).

```
git commit -am "chore: release vX.Y.Z"
git push -u origin release/vX.Y.Z
```

Su GitHub apri la Pull Request `release/vX.Y.Z → develop`, attendi la CI verde e fai il merge.

## 3. Rilascia: da `develop` a `main`

Su GitHub: **Pull requests → New pull request**, base `main`, compare `develop`.

- Titolo: `release: vX.Y.Z`
- Attendi la CI verde, poi **Merge pull request**.

Render pubblica la nuova versione da solo. Verifica:

- nella dashboard di Render il deploy è **Live**;
- l'API risponde:

  ```
  curl https://rust-postgre-api.onrender.com/api/health
  ```

  Risposta attesa: `{"status":"ok","version":"X.Y.Z"}`, con la versione appena rilasciata. Dopo un periodo di inattività la prima risposta può
  richiedere fino a un minuto.

Se il deploy fallisce, la versione precedente resta online: correggi con un `fix` (dal punto 2)
e ripeti, senza creare il tag.

## 4. Crea il tag

Il tag va sul commit di `main` appena pubblicato.

```
git switch main
git pull
git log --oneline -1
git tag -a vX.Y.Z -m "vX.Y.Z: <riassunto in una riga>"
git push origin vX.Y.Z
```

- `-a` crea un tag annotato (con autore, data e messaggio): è quello da usare per i rilasci.
- `git push` da solo non invia i tag: va indicato il nome.

Verifica che sia su GitHub:

```
git ls-remote --tags origin vX.Y.Z
```

## 5. Crea la Release su GitHub

Nel repository: **Releases → Draft a new release**.

1. **Choose a tag**: `vX.Y.Z`.
2. **Generate release notes**: GitHub elenca le Pull Request incluse.
3. Rileggi le note, aggiungi in cima un riassunto se serve, poi **Publish release**. Se il
   rilascio non è compatibile con i client già pubblicati, aggiungi una sezione:

   ```markdown
   ## Attenzione
   - Non compatibile con client < vA.B.C: <cosa è cambiato>.
   ```

## 6. Riallinea `develop`

Il merge su `main` crea un commit che `develop` non ha. Su GitHub apri la Pull Request
`main → develop` (titolo `chore: sync develop with main after vX.Y.Z`), attendi la CI e fai il
merge.

Poi aggiorna la copia locale ed elimina il branch di rilascio, ormai inutile:

```
git switch develop
git pull
git branch -d release/vX.Y.Z
git push origin --delete release/vX.Y.Z
```

## Hotfix: correzione urgente in produzione

Quando un bug in produzione va corretto subito e `develop` contiene già modifiche non pronte
per il rilascio:

```
git switch main
git pull
git switch -c hotfix/descrizione-breve
```

1. Correggi il bug, aggiungi il test e porta `version` in `Cargo.toml` alla PATCH successiva
   (poi `cargo check` per aggiornare `Cargo.lock`).
2. Commit e push:

   ```
   git commit -am "fix: descrizione"
   git push -u origin hotfix/descrizione-breve
   ```

3. Pull Request `hotfix/descrizione-breve → main`, CI verde, merge: Render pubblica.
4. Tag e Release come ai punti 4 e 5.
5. Pull Request `main → develop` per portare la correzione anche in `develop`: senza questo
   passaggio il bug torna al rilascio successivo.

## Correggere un tag sbagliato

- **Non ancora pushato**: eliminalo e ricrealo.

  ```
  git tag -d vX.Y.Z
  git tag -a vX.Y.Z -m "..."
  ```

- **Già pushato**: non spostarlo, perché chi lo ha già scaricato resta con la versione vecchia e
  la Release vi fa riferimento. Rilascia invece la versione successiva. Solo se nessuno lo ha
  ancora usato, si può eliminare anche da GitHub:

  ```
  git push origin --delete vX.Y.Z
  git tag -d vX.Y.Z
  ```

## Checklist

- [ ] Numero di versione scelto e compatibilità con il client verificata (punto 1)
- [ ] `Cargo.toml` e `Cargo.lock` aggiornati, PR `release/vX.Y.Z → develop` unita
- [ ] PR `develop → main` unita, deploy su Render **Live**, `/api/health` risponde
- [ ] Tag `vX.Y.Z` creato su `main` e pushato
- [ ] Release pubblicata su GitHub
- [ ] PR `main → develop` unita, branch `release/vX.Y.Z` eliminato

## Comandi utili

| Comando | A cosa serve |
|---|---|
| `git tag` | elenco dei tag locali |
| `git ls-remote --tags origin` | elenco dei tag su GitHub |
| `git fetch --tags` | scarica i tag creati su GitHub |
| `git show vX.Y.Z` | dettagli del tag e del commit |
| `git log --oneline v0.1.0..v0.2.0` | cosa è cambiato tra due versioni |
| `git switch --detach vX.Y.Z` | guarda il codice di una versione (poi `git switch develop`) |
