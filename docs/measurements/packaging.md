# Csomagolás (M3-T1) – mérések

Mérve 2026-11-20, Debian 13 host, 12 szál (i7-1355U), docker 29.9.1,
`cargo-deb` 3.8.0, `cargo` 1.100.0.

## M1 – A csomag

| Build | Parancs | Idő | `.deb` | Telepített méret |
|---|---|---:|---:|---:|
| Debian 13 (host) | `cargo deb -p ferrent-daemon --features libtorrent` | 22,5 s (meleg target) | 3,79 MB | 14,9 MB |
| Debian 12 (`debian:12`, tiszta toolchain) | ugyanaz konténerben | 2 m 12 s (teljes release) | 3,79 MB | 14,9 MB |

Tartalom: `/usr/bin/ferrentd`, `lib/systemd/system/ferrentd.service`,
`/etc/ferrent/config.toml` (conffile), `man1/ferrentd.1.gz`, copyright.
A token **nincs** a csomagban: a `postinst` generálja 32 bájtos base64-ként
(44 karakter) `/etc/ferrent/token`-be, `0640 root:ferrent`.

**A függőségek a build host suite-jét követik** (`$auto`):

| Suite | Depends |
|---|---|
| bookworm | `libc6 (>= 2.34), libssl3, libtorrent-rasterbar2.0 (>= 2.0.8)` |
| trixie | `libc6 (>= 2.39), libssl3t64, libtorrent-rasterbar2.0t64 (>= 2.0.11)` |

Egy trixie-n épült `.deb` tehát **nem telepíthető** bookwormra (libc6 2.36 < 2.39,
nincs `libssl3t64`/`t64` libtorrent). A cél-suite build konténer az M3-T2 dolga;
addig a `packaging/debian/README.md` recipe-je szerint kell építeni.

## M2 – Telepítés friss konténerbe

`packaging/debian/container-test.sh` — 32 assert, Debian 12, Debian 13 és Ubuntu 24.04
ellen: **32/32 PASS**, exit 0 mindhármon (a statikus csomagnál).

| Ellenőrzés | Eredmény |
|---|---|
| `ferrent` system user/group, shell `nologin` | PASS |
| `/var/lib/ferrent` `0750 ferrent:ferrent` | PASS |
| `config.toml` / `token` `0640 root:ferrent` | PASS |
| token = 32 bájt → 44 karakter | PASS |
| `postinst` enableli a unitot (systemd fut) | PASS |
| `systemctl start` → active, `127.0.0.1:50051` listen | PASS |
| `startup_ms` 8,8–16,1; RSS ~9,5 MB (üres állapot) | PASS |
| `systemctl stop` 33 ms, `saved fastresume` | PASS |
| restart új PID, TLS cert + fingerprint változatlan | PASS |
| `apt remove`: state/config/token megmarad, binary/unit eltűnik, service leáll | PASS |
| `apt purge`: `/var/lib/ferrent`, `/etc/ferrent`, token törölve | PASS |
| `apt purge`: a `ferrent` user **megmarad** (dokumentált) | PASS |

## M3 – Konténer-specifikus csapdák (mind a szkriptben kezelve)

- A `debian:*-slim` képek `/usr/sbin/policy-rc.d`-je `101`-gyel tér vissza, így
  a `prerm` `deb-systemd-invoke stop`-ja csendben nem állítja le a szervizt
  (a `remove` után „active" marad, a binary már törölve). A teszt törli a
  `policy-rc.d`-t; valódi Debianon nincs ilyen.
- A kép build közben nincs futó systemd, ezért a `postinst`
  `deb-systemd-helper enable` ága kimarad. A teszt `--reinstall`-lal futtatja
  újra a `postinst`-ot a futó systemd alatt, és azt is ellenőrzi.
- systemd PID1-ként csak `--cgroupns=host` + `-v /sys/fs/cgroup:/sys/fs/cgroup:rw`
  mellett indul; `:ro` esetén a konténer azonnal `255`-tel kilép, üres loggal.
- A unit útja `systemctl show -p FragmentPath` szerint
  `/lib/systemd/system/...` (merged-usr); a teszt nem hardcode-ol `/usr/lib`-ot.

## M4 – Ismételhető ellenőrzés

```bash
packaging/debian/container-test.sh debian:12 target/debian/ferrentd_0.1.0-1_amd64.deb
packaging/debian/container-test.sh debian:13 target/debian/ferrentd_0.1.0-1_amd64.deb
```

A szkript a `.deb`-ből épít egy eldobható systemd-s képet, lefuttatja a 32
assertet, és `exit 1`-gyel áll meg az első bukásnál (a PASS/FAIL sorok
megmondják, melyik).

## M5 – Statikus libtorrent, egy csomag több disztróra (M3-T2)

Build: `packaging/debian/build.sh` — `debian:12-slim` konténer, libtorrent
2.0.11 forrásból `BUILD_SHARED_LIBS=OFF`, majd a daemon
`FERRENT_LIBTORRENT_STATIC=1`-gyel. OpenSSL/zlib dinamikus marad (ADR 0005).

| Metrika | Dinamikus (T1) | Statikus amd64 | Statikus arm64 |
|---|---:|---:|---:|
| `ferrentd` bináris | 15,2 MB | 26,8 MB | 18,2 MB |
| `.deb` | 3,79 MB | 5,27 MB | 4,65 MB |
| Telepített méret | 14,9 MB | 19,5 MB | 17,8 MB |
| `ldd` libtorrent | igen | **nincs** | **nincs** |
| `Depends` libtorrent | `libtorrent-rasterbar2.0 (>= 2.0.8)` | – | – |

A statikus csomag `Depends`-e: `adduser, init-system-helpers,
libc6 (>= 2.34), libssl3 (>= 3.0.0), libstdc++6 (>= 12)`. Az arm64 csomagnál a
`libstdc++6:arm64` multiarch utótagot a cross build környezete teszi be; arm64
gépen ugyanazt jelenti.

`ldd` a statikus binárison: `libssl.so.3`, `libcrypto.so.3`, `libstdc++.so.6`,
`libgcc_s.so.1`, `libm.so.6`, `libc.so.6` — libtorrent nélkül.

### Telepítés és futás

| Cél | Eredmény |
|---|---|
| Debian 12, Debian 13, Ubuntu 24.04 (amd64) | `container-test.sh`: **32/32 PASS** mindhármon |
| arm64 (cross, `aarch64-linux-gnu-g++`) | `qemu-smoke.sh`: PASS — indul, TLS-t nyit, `state.redb`-t ír; `startup_ms=211–300` (qemu emuláció) |

Build idők (hideg, 12 szál): amd64 ~7 perc, arm64 ~10 perc — a konténer
rétegei (toolchain, cargo-deb, libtorrent) cache-elődnek, ismételt buildnél
csak a Rust fordítás fut.

### Ismételhető ellenőrzés

```bash
packaging/debian/build.sh debian:12 amd64
packaging/debian/build.sh debian:12 arm64
packaging/debian/container-test.sh debian:12 target/debian-dist/ferrentd_0.1.0-1_amd64.deb
packaging/debian/qemu-smoke.sh target/debian-dist/ferrentd_0.1.0-1_arm64.deb
```

Ugyanezt futtatja a `.github/workflows/release-deb.yml` minden `v*` tagon
(build amd64+arm64, verify a három suite-on, arm64 qemu smoke). Publikálás
(APT repo, GHCR) még nincs a workflow-ban — az M3-T3/T4.

## M6 – APT repó (M3-T3)

`packaging/apt/build-repo.sh` a `target/debian-dist` .deb-jeiből épít
`dists/stable` + `pool` fát, `apt-ftparchive`-ral számolt checksumokkal
(MD5/SHA1/SHA256/SHA512), és aláírja (`InRelease`, `Release.gpg`). A szkript a
végén `gpgv`-vel ellenőrzi a saját aláírását a repóban tárolt publikus kulccsal.

| Metrika | Érték |
|---|---:|
| Repó építése (3 .deb, 2 arch) | **1,95 s** |
| `dists/stable/Release` / `InRelease` | 2,4 KB / 3,3 KB |
| `Packages` / `Packages.gz` (amd64) | 674 B / 467 B |
| Publikált fa mérete (3 .deb-fel) | 9,5 MB |

Aláíró kulcs (2026-11-22 óta): Ed25519 + cv25519,
`3E028C8A4521C1289117CCBE1F9CEB5C0BB1A431`, érvényes 2031-11-21-ig,
jelszóval védve; a privát kulcs és a passphrase a repón kívül
(`~/.config/ferrent/apt/`, 0600). A régi RSA 4096
(`DB161AB5A22473586AC52CEDD727691B9D3B58AA`) átmenetileg a publikus
keyringben marad.

### Telepítési teszt (`packaging/apt/repo-test.sh`)

Debian 12 konténer, `file:/repo` forrás, DEB822 `Signed-By`:

| Ellenőrzés | Eredmény |
|---|---|
| `apt update` figyelmeztetés nélkül | PASS |
| Mindkét verzió listázva (`0.0.9-1`, `0.1.0-1`) | PASS |
| Candidate a legújabb | PASS |
| Korábbi verzió telepítése (`--allow-downgrades`) | PASS |
| Frissítés a legújabbra | PASS |
| Kulcs nélkül az `apt update` elbukik | PASS |

A „korábbi verzió" egy `dpkg-deb -R/-b` átcsomagolt `0.0.9-1`, tehát a
pool-megőrzés valóban tesztelt, nem csak deklarált. **6/6 PASS.**

Két hiba az első futásból, mindkettő javítva: a `dpkg-scanpackages` abszolút
`Filename:` mezőt írt (a repó gyökeréből relatív útvonallal kell futtatni), és
a negatív kulcs-teszt hamis PASS-t adott, mert az apt a cache-elt listákat
ellenőrzés nélkül elfogadta (`rm -rf /var/lib/apt/lists/*` kell előtte).

### Publikálás

`.github/workflows/release-apt.yml`: tagonként build (amd64+arm64) → `gh-pages`
worktree (vagy üres orphan branch) → repó építés + aláírás a
`GPG_PRIVATE_KEY`/`GPG_KEY_ID` secretből → commit+push. A régi verziók a
`gh-pages` history-ban maradnak. Hiányzó secret → explicit `::error::` és
bukó job. A Pages beállítás (branch: `gh-pages`) és a kliens útmutató
(`docs/install-debian.md`) dokumentálva.



## M7 – Verzió-lánc (2026-11-20)

`packaging/debian/version.sh` az egyetlen leképezés: a `Cargo.toml` a
verzióforrás, a tagot ellenőrzi, és csak a Debian-verziót rendereli másképp
(a pre-release utótag `~`-t kap, hogy az rc **kisebb** legyen a finalnál).

| Tag | Cargo | `.deb` |
|---|---|---|
| `v0.1.0` | `0.1.0` | `0.1.0-1` |
| `v0.1.0-rc1` | `0.1.0-rc1` | `0.1.0~rc1-1` |
| `v0.2.0` | `0.2.0` | `0.2.0-1` (revízió: `DEB_REVISION`) |

A `build.sh` a `TAG` env-ből számol és `--deb-version`-nel adja át a
`cargo deb`-nek. Ellenőrzés: `packaging/debian/version.sh --self-test`
(4 assert), és a `cargo deb --deb-version 0.1.0-2` kézzel is
`ferrentd_0.1.0-2_amd64.deb`-et adott.

GitLab release: `.gitlab-ci.yml` `release-deb` job `v*` tagon — verzió
ellenőrzés, amd64+arm64 build, majd `packaging/apt/publish-gitlab.sh` a
Debian Package Registrybe (`bookworm`/`trixie`/`noble` kódnevekre; a
`Release`-t a GitLab írja alá a saját kulcsával).

## M8 – Docker image (M3-T4)

`packaging/docker/Dockerfile` — két lépcső: `rust:1-trixie` + libtorrent dev
build, majd `debian:trixie-slim` + `libtorrent-rasterbar2.0`. A konténer
`ferrentd`-t és `ferrent-cli`-t visz (nincs GUI/web), nem-rootként fut, a
`/var/lib/ferrent` a volume.

| Metrika | Érték |
|---|---:|
| Image mérete | **122 MB** |
| Build idő (hideg, 12 szál) | **5 perc 7 s** |
| Konténer user | `uid=1000(ferrent)` |
| Bind | `0.0.0.0:50129` (a `.deb` loopbacket használ) |

Ellenőrzés (mérve 2026-11-20):

```bash
docker build -f packaging/docker/Dockerfile -t ferrent .
docker run -d --name ferrent-test -e FERRENT_TOKEN=test-secret \
    -p 50139:50129 -v /tmp/ferrent-docker:/var/lib/ferrent ferrent
docker inspect --format '{{.State.Health.Status}}' ferrent-test   # healthy
ferrent-cli --server https://localhost:50139 \
    --ca /tmp/ferrent-docker/tls-cert.pem --token test-secret list
```

- A healthcheck (`ferrent-cli list --limit 1`) **0/0** exit kóddal futott, a
  státusz `healthy`; a volume-ban megjelenik a `tls-cert.pem`,
  `tls-key.pem`, `tls-fingerprint.txt` és a `state.redb`.
- A hostról a publikált porton a CLI **csatlakozik** (üres lista), tehát a
  TLS + token lánc a konténerben is jó.
- A healthcheck első változata `--json status`-t hívott — nincs ilyen
  alparancs, ezért a `list --limit 1` a próba.

## M9 – AppImage (M3-T4)

`packaging/appimage/build.sh` — `linuxdeploy` + `linuxdeploy-plugin-gtk`

- `appimagetool`, a GTK3 stack és a loaderek az AppDirbe kerülnek; a
  `libfontconfig`, `libfribidi`, `libwayland-*`, `libxkbcommon*` explicit
  `--library` (a linuxdeploy alapból kihagyja őket, egy minimalista
  konténerben viszont hiányoznak).
- Ikon: 256×256 (a `mark.svg`-ből), desktop fájl `packaging/appimage/`.

| Metrika | Érték |
|---|---:|
| AppImage méret | **34,6 MB** |
| Build (meleg cargo target) | ~1,5 perc |

Fedora 42 konténer, Xvfb 1440×900, `ICED_BACKEND=tiny-skia`
(a wgpu Vulkan nélkül elszáll; valódi asztalon nem kell):

| Ellenőrzés | Eredmény |
|---|---|
| Elindul, ablakot nyit (`xwininfo`: `"Ferrent" 1440x900`) | PASS |
| Csatlakozik a daemonhoz (`--connect`, status bar `connected`) | PASS |
| 12 torrent listázva, logó a sidebaron | PASS (`docs/measurements/m3-appimage-fedora.png`) |

A kép a Fedorán futó AppImage-et mutatja, a laptop daemonjához csatlakozva.

### CI (M13-T3, #69)

A `release-appimage` job ugyanabból a release commitból épít, mint a
`release-deb`, és a Generic Registrybe tölti
(`packages/generic/ferrent-gui/<ver>/Ferrent-<ver>-x86_64.AppImage`).

- **Konténerben épül** (`packaging/appimage/container-build.sh`,
  `debian:12-slim` + GTK/X dev + rustup): a runner shell executorán nincs
  cargo/GTK, csak docker.
- **A fájlnév verziós**: `Ferrent-<Cargo version>-x86_64.AppImage`
  (pl. `Ferrent-1.0.0-rc1-x86_64.AppImage`), nem `continuous`.
- **Az eszközök pinnelve**: `packaging/appimage/tools.lock` tartalmazza a
  linuxdeploy / appimagetool / GTK plugin **sha256**-ját; eltérés esetén a
  build elbukik (a felfelé mutató URL `continuous`, a hash a valódi pin).
- **Csak x86_64**: az arm64-et a `.deb` fedi, és a runner amd64.
- **Smoke** (`packaging/appimage/smoke.sh`): a `ferrent` .deb-ből telepített
  daemonhoz csatlakozik az AppImage `--appimage-extract-and-run --measure`
  módban Xvfb alatt; a `rows=<n>` sor a bizonyíték (csak csatlakozás után
  íródik ki).
- Helyben mérve 2026-11-22: build 1 m 11 s meleg targetből (`--no-build`),
  **34 MB**, `rows=13`, `fps=117`, `rss_kb=51 380` a 13 torrentes élő
  daemonon.
