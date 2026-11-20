fn main() {
    println!("cargo:rerun-if-changed=src/libtorrent/bridge.cpp");
    println!("cargo:rerun-if-changed=src/libtorrent/bridge.h");
    println!("cargo:rerun-if-changed=src/libtorrent/bridge.rs");

    if std::env::var_os("CARGO_FEATURE_LIBTORRENT").is_none() {
        return;
    }

    // The Debian package is built with OpenSSL, so the public headers need
    // TORRENT_USE_OPENSSL / TORRENT_USE_LIBCRYPTO to select the same hasher
    // implementation as the shared library.
    let libtorrent = {
        let mut config = pkg_config::Config::new();
        config.atleast_version("2.0");
        if std::env::var_os("FERRENT_LIBTORRENT_STATIC").is_some() {
            // packaging/debian/build.sh builds libtorrent with
            // BUILD_SHARED_LIBS=OFF; the .pc Libs.private then lists boost,
            // ssl and zlib, and pkg-config resolves them with --static.
            config.statik(true);
        }
        config
            .probe("libtorrent-rasterbar")
            .expect("libtorrent-rasterbar not found (install libtorrent-rasterbar-dev)")
    };

    let mut build = cxx_build::bridge("src/libtorrent/bridge.rs");
    build.file("src/libtorrent/bridge.cpp").std("c++17");
    for path in &libtorrent.include_paths {
        build.include(path);
    }
    for (name, value) in &libtorrent.defines {
        match value {
            Some(value) => {
                build.define(name, Some(value.as_str()));
            }
            None => {
                build.define(name, None);
            }
        }
    }
    build.compile("ferrent_engine_bridge");
}
