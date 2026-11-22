# CI, kiadás és GitHub-tükör

## Rétegek

```text
GitLab (fejlesztés, kanonikus repó)
  cleanup -> quality (clippy) -> sonar -> version (manuális) -> release (deb + registry)

GitHub Actions (a tükör repón; cél: build/teszt és a nyilvános kiadás)
  ci.yml          minden push/PR: proto lint, fmt, clippy, crate-tesztek
  release-deb.yml heti / manuális: független .deb újraépítés + 3 suite + qemu smoke
  release-apt.yml release published: a GitLab által feltöltött .deb-ek aláírása → gh-pages
```

A fejlesztés a saját GitLabon folyik (`gabor/ferrent`, project 56); a GitHub
repó egyirányú tükör. A helyi kapuk ugyanazok, amit a CI futtat:

```bash
ci/check.sh            # fmt + clippy + teszt, MockEngine (--no-default-features)
ci/check-libtorrent.sh # a cxx bridge clippy + teszt libtorrenttel
```

## GitLab pipeline

| Stage | Job | Mikor |
|---|---|---|
| cleanup | `cleanup-workspace` | mindig; a Dockeres futás root-owned fájljait javítja |
| quality | `clippy-report` | MR és default branch; clippy JSON → Sonar generic issues |
| quality | `buf-lint` | MR és default branch; `buf lint` a protón (ugyanaz, mint a GitHub `proto` job) |
| sonar | `sonarqube` | az előző artifactjára épül, blocking quality gate |
| version | `release-version` | **manuális**: `Run pipeline` (web/api) a default branch-en |
| release | `release-deb` | a version job után (vagy `v*` tagen): 2 arch, container-teszt, GitLab generic registry + GitHub Release asset |

A `LabelResponse` a `CreateLabel` és `UpdateLabel` közös response-a; a két
RPC-névre vonatkozó lint-szabály (`RPC_REQUEST_RESPONSE_UNIQUE`,
`RPC_RESPONSE_STANDARD_NAME`) alól a `buf.yaml` `ignore_only`-jal kiveszi. A
`buf breaking` kapu érintetlen — egy response-típus átnevezése továbbra is
bukik.

A kiadás belépője a GitLab UI egy mezője (`release_version`, vagy
`release_type` = patch/minor/major) — nem kézi tag push. A `release-version`
megírja a `Cargo.toml`-t, commitol, tagel; a `release-deb` abból a commitból
dolgozik. A CI-jobb-tokenes push nem indít új pipeline-t.

## GitHub-tükör

A GitHub repó (`gabordeli/ferrent`) a privát GitLab repó **egyirányú,
read-only tükre**; a GitLab `Settings → Repository → Mirroring repositories`
push-mirrorja viszi a `main`-t és a tageket. Fejlesztés GitHubon nem folyik,
a GitHub issue/PR portolása a maintainer dolga.

A `.deb` **egyszer** épül: a GitLab `release-deb` konténerbuildje adja a
`ferrent` és `ferrent-gui` csomagot, publikálja a GitLab registrybe, majd
`packaging/apt/publish-github-release.sh`-val a GitHub Release assetbe teszi.
A GitHub `release-apt` ezt a release-t figyeli (`release: published`), letölti a
`.deb`-eket és csak aláír + `gh-pages`. Ehhez a GitLab CI-ban egy
**`GITHUB_RELEASE_TOKEN`** változó kell (Contents: read and write a tükörre).

A `release-deb` (GitHub) már nem tagenként fut: heti független újraépítés +
3 suite/qemu ellenőrzés.

A push-mirrorhoz használt GitHub fine-grained PAT (a repóra scope-olva) jogai:

| Permission | Miért |
|---|---|
| Contents: Read and write | a mirrorolt ágak/tagek pusholása |
| Workflows: Read and write | a repóban `.github/workflows/*` van |
| Metadata: Read | kötelező alap |
| Pages: Read and write | a Pages forrás beállítása API-ból |

A GitHub Actions a repó saját `GITHUB_TOKEN`-jét használja
(`permissions: contents: write`), oda nem kell külön PAT.

### GitHub CI jobok rendszercsomagjai

A GitHub jobok a `.github/actions/install-deps` composite actiont hívják, két
kapcsolóval: `gui` (iced/winit + rfd/GTK) és `libtorrent` (a daemon default
feature-e). Ugyanaz a csomagkészlet, mint a GitLab `clippy-report` jobjában —
a kettőt együtt kell tartani.

| Job | `gui` | `libtorrent` |
|---|---|---|
| `clippy` (workspace, default features) | igen | igen |
| `build-no-cpp` (GUI + CLI) | igen | nem |
| `test-gui` (dev-dep daemon) | igen | igen |
| `test-daemon` | nem | igen |
| `test-cli` (dev-dep daemon) | nem | igen |
| `test-engine-libtorrent` | nem | igen |
| `test-engine`, `test-protocol`, `fmt`, `proto` | nem | nem |

A futás állapota API-ból is megnézhető (a gépen nincs `gh` CLI); a script a
legfrissebb `main` futást, vagy egy megadott run id-t jelent:

```bash
ci/github-ci-status.sh                     # legfrissebb main futás
ci/github-ci-status.sh gabordeli/ferrent 44957670322
```

A token a `GITHUB_TOKEN` env változóból vagy a
`~/.config/ferrent/github.pat` fájlból jön; ehhez `Actions: read` jog kell (a
tükör-PAT tudja). A script a bukott joboknál a log cleanup előtti részét és a
hibasorokat írja ki.

Ellenőrzés (a token a `~/.config/ferrent/github.pat` fájlban):

```bash
TOKEN=$(tr -d '\n\r ' < ~/.config/ferrent/github.pat)
curl -s -H "Authorization: Bearer $TOKEN" \
  "https://api.github.com/repos/gabordeli/ferrent/commits?per_page=1" \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)[0]["sha"][:8])'
```

A tükör akkor jó, ha ez a GitLab `main` legfrissebb commitját adja (a mirror
néhány perc alatt követi a pushot).

**Amíg a repó privát**, a GitHub Pages és így az APT repó URL-je
(`https://gabordeli.github.io/ferrent/`) kívülről nem érhető el; a
`gh-pages` ág és a tartalma akkor válik használhatóvá, amikor a repó nyilvános
lesz (`docs/apt-install.md`).

## APT repó (GitHub Pages)

- Pages forrás: **Deploy from a branch**, branch `gh-pages`, folder `/`.
- A `gh-pages` ág létezik (üres root commit + `index.html` placeholder +
  `.nojekyll`), a `release-apt.yml` ebbe ír: `dists/stable`,
  `pool/main/f/ferrentd`, `ferrent.gpg`, `index.html`; a régi verziók a
  poolban maradnak.
- **A Pages site még nincs létrehozva:** a `POST /repos/.../pages` a jelenlegi
  csomaggal `422 Your current plan does not support GitHub Pages for this
  repository` — a Free csomag privát repóhoz nem ad Pages-t. A repó
  publikálásakor egyszer létre kell hozni (UI: Settings → Pages, vagy
  `POST /repos/gabordeli/ferrent/pages` `{"source":{"branch":"gh-pages","path":"/"}}`).
- Secrets (repository settings → Secrets and variables → Actions):

| Secret | Érték |
|---|---|
| `GPG_PRIVATE_KEY` | `base64 -w0 ~/.config/ferrent/apt/ferrent-apt-private.asc` |
| `GPG_KEY_ID` | `3E028C8A4521C1289117CCBE1F9CEB5C0BB1A431` |
| `GPG_PASSPHRASE` | `cat ~/.config/ferrent/apt/passphrase` |

A kulcs részletei és a rotáció: `packaging/apt/README.md`.

## Perf harness

A `ci/perf.sh` friss adatkönyvtáron mér: `mvp_prepare` (alapból 2000 torrent,
a workflow 1000-rel fut), daemon-indítás, majd 30 `ListTorrents` hívás egy
csatornán. A kimenet JSON: `startup_ms`, `rss_kb`, `list_p50_ms`,
`list_p95_ms`; a `ci/perf-baseline.json`-hoz hasonlít **10%** toleranciával,
és `1`-gyel lép ki regressziónál. A baseline-t az első futás írja; utána
szándékosan kell frissíteni (pl. nagyobb gép, új verzió).

- Workflow: `.github/workflows/perf.yml` (nightly 03:17 + kézi indítás),
  feltölti a `perf-result.json` artifactot.
- A mérés a `localhost` SAN-t használja (a tanúsítványban nincs IP SAN).
- Mérve 2026-11-21, 1000 torrent: `startup_ms 657–880`, `rss_kb 45 120`,
  `list_p95_ms 48.1`.

## Kiadási kapu

A `v0.1.0` nem soak-eredményhez kötött: a daemonnak **hónapokig** kell éles
torrenteken futnia, közben a Bugsinkben látszó hibákat javítjuk; a repó addig
privát marad. A részletes kapu: `ROADMAP.md` → M2-T5, GitLab #26.
