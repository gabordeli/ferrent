fn main() {
    println!("cargo:rerun-if-changed=src/libtorrent/bridge.cpp");
    println!("cargo:rerun-if-changed=src/libtorrent/bridge.h");
    println!("cargo:rerun-if-changed=src/libtorrent/bridge.rs");

    if std::env::var_os("CARGO_FEATURE_LIBTORRENT").is_none() {
        return;
    }

    cxx_build::bridge("src/libtorrent/bridge.rs")
        .file("src/libtorrent/bridge.cpp")
        .std("c++17")
        .compile("ferrent_engine_bridge");

    println!("cargo:rustc-link-lib=torrent-rasterbar");
}
