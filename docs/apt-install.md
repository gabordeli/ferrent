# Telepítés APT repóból

A `ferrentd` csomag egy aláírt APT repóból telepíthető Debian 12/13-on és
Ubuntu 24.04-on (amd64 és arm64). A repó a `gh-pages` ágon él, a Pages URL-je
`https://gabordeli.github.io/ferrent/`.

> A GitHub repó a `v0.1.0` kiadásig **privát**, ezért a Pages URL addig
> kívülről nem érhető el. A lenti lépések a repó publikálása után működnek
> változatlanul.

## 1. Kulcs

```bash
sudo install -d -m 0755 /etc/apt/keyrings
curl -fsSL https://gabordeli.github.io/ferrent/ferrent.gpg \
  | sudo tee /etc/apt/keyrings/ferrent.gpg >/dev/null
```

Ellenőrzés (a fájl ujjlenyomata):

```bash
gpg --show-keys --with-fingerprint /etc/apt/keyrings/ferrent.gpg
# 3E028C8A4521C1289117CCBE1F9CEB5C0BB1A431  (Ed25519, az aktuális aláíró)
# DB161AB5A22473586AC52CEDD727691B9D3B58AA  (RSA 4096, átmeneti)
```

A letöltött fájl a rotáció idején **két kulcsot** tartalmaz: az új Ed25519
aláíró kulcsot és a régi RSA 4096-ot, hogy a korábbi aláírások is
ellenőrizhetők legyenek.

## 2. Forrás (DEB822)

```bash
sudo tee /etc/apt/sources.list.d/ferrent.sources >/dev/null <<'EOF'
Types: deb
URIs: https://gabordeli.github.io/ferrent/
Suites: stable
Components: main
Architectures: amd64 arm64
Signed-By: /etc/apt/keyrings/ferrent.gpg
EOF
```

`Signed-By` a letöltött kulcsra mutat, tehát nincs `apt-key`, és a kulcs csak
erre a repóra érvényes.

## 3. Telepítés

```bash
sudo apt update
sudo apt install ferrentd
```

Utána:

```bash
systemctl status ferrentd
sudo cat /etc/ferrent/token     # a kliens ezzel csatlakozik
```

A token a telepítéskor generálódik (`0640 root:ferrent`), a csomagban nincs
benne. A daemon a `127.0.0.1:50051`-en hallgat TLS-sel; a GUI/CLI a
`https://localhost:50051` címet használja, és az önaláírt tanúsítvány
ujjlenyomatát az első csatlakozáskor kell jóváhagyni.

## Verzió rögzítése, visszalépés

A repó minden korábbi verziót megtart:

```bash
apt-cache policy ferrentd          # elérhető verziók
sudo apt install ferrentd=0.1.0-1  # rögzítés
sudo apt install --allow-downgrades ferrentd=0.0.9-1
```

## Eltávolítás

```bash
sudo apt remove ferrentd    # /etc/ferrent és /var/lib/ferrent megmarad
sudo apt purge ferrentd     # az állapot, a token és a conffile is törlődik
```

A `ferrent` rendszerfelhasználó purge után is megmarad (újratelepítéskor
ugyanaz az UID).

## Hibakeresés

| Tünet | Ok |
|---|---|
| `NO_PUBKEY` / `The following signatures couldn't be verified` | a kulcsfájl hiányzik, vagy más kulcs van benne |
| `Release file ... is not valid yet` | a build gép órája járt előre; `apt update` újra |
| `404` a `pool/...` fájlra | a `gh-pages` ág még nem tartalmazza azt a verziót |

A repó felépítése és a kulcs rotációja: `packaging/apt/README.md`.

## GitLab Package Registry (saját GitLab)

A kiadott `.deb`-ek a saját GitLabon, a **Packages & Registries** alatt is
megjelennek (`ferrentd`, verziónként, arch-onként külön fájl). Ezen a GitLabon
(19.5.5) a Debian registry API-route-jai nincsenek meg (`/debian_distributions`
→ 404, mérve), ezért a publikálás a **generic registry**-be megy; az apt
telepítés továbbra is az aláírt repóból (`packaging/apt/build-repo.sh`) megy.

A registry-ből kézzel:
`https://cirice-gitlab.ddns.net/gabor/ferrent/-/packages` (tokenes böngésző).
