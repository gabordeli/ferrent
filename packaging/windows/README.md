# Windows (mingw cross-build)

A Windows csomag Linuxról készül, `x86_64-pc-windows-gnu` targettel. A
`windows-cross` CI job ugyanezt futtatja a runner konténerében; ez a leírás a
kézi reprodukáláshoz kell.

## Előfeltételek

```bash
sudo apt install gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64 cmake libboost-dev
rustup target add x86_64-pc-windows-gnu
```

## Build

```bash
packaging/windows/build-libtorrent.sh /tmp/lt-win/prefix      # ~4 perc, utána cache-elhető

PKG_CONFIG_ALLOW_CROSS=1 \
PKG_CONFIG_PATH=/tmp/lt-win/prefix/lib/pkgconfig \
FERRENT_LIBTORRENT_STATIC=1 \
RUSTFLAGS="-C link-args=-static" \
cargo build --release --target x86_64-pc-windows-gnu \
  -p ferrent-daemon -p ferrent-cli -p ferrent-gui

packaging/windows/stage-artifacts.sh      # → target/windows/
```

## Döntések, amiket érdemes tudni

- **Nincs cross OpenSSL.** A libtorrent a beépített RC4/SHA1-jét használja, ha
  nem talál OpenSSLt/GnuTLS-t/libgcryptot; a CMake-et ezért
  `CMAKE_DISABLE_FIND_PACKAGE_OpenSSL=ON`-nal hívjuk (különben a host Linux
  OpenSSL-jét próbálná belinkelni a Windows binárisba).
- **A Boost csak fejléc.** A `/usr/include`-ot nem szabad include-gyökérnek
  adni (a glibc `wchar.h`-ja elnyomja a mingw-ét), ezért a script külön
  könyvtárba symlinkeli csak a `boost/`-ot.
- **`libstdc++-6.dll` kell a daemon mellé.** A `cxx` bridge C++ kódja miatt a
  `ferrentd.exe` dinamikusan köti a C++ runtimet: a rustc a natív libeket
  `-Bdynamic` csoportba teszi, ezért a `-static` sem viszi be statikusan. A
  staging script a DLL-t is kimásolja (stripre 2,2 MB).
- **A binárisok strippelve** kerülnek a csomagba: 124 MB → 24 MB. A `.deb`
  ezzel szemben megtartja a szimbólumokat (a Bugsink a folyamatban oldja fel a
  kereteket); Windowson ez a trade-off a ZIP mérete miatt fordítva van, és az
  első VM-es futás dönti el, kell-e külön szimbólum-történet.
