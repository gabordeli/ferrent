# M2-T1 – GUI spike (4 000 sor)

Mérve 2026-11-19, Debian 13.7, Intel Iris Xe, display `:0`, 1100×700,
release buildek, 5 s auto-scroll.

| Metrika | iced 0.13.1 | Tauri 2.12 (WebKitGTK 2.54) |
|---|---|---|
| Indulás (process → első frame) | 600,4 ms | 764,2–868,2 ms |
| FPS | 59,0 | 57,7–58,6 |
| RSS (process tree) | 100 960 KB ≈ 98,6 MB | 527 612–530 424 KB ≈ 515–518 MB |
| Bináris | 20 MB | 11 MB |
| Látható sorok | 28 | 28 |

Debug referencia: iced 774 ms / 119 MB, Tauri 852 ms / 527 MB.
A Tauri két release futása közötti eltérés futáson belüli szórás; a
végső, CSS-javítás utáni futás 764,2 ms / 58,6 FPS / 518 MB volt.

Screenshotok: `docs/measurements/m2-iced.png`, `docs/measurements/m2-tauri.png`
(XWayland, layout-ellenőrzés).

## Futtatás

```bash
cargo run --manifest-path crates/gui/spikes/iced-spike/Cargo.toml \
    --release -- --auto
cargo run --manifest-path crates/gui/spikes/tauri-spike/Cargo.toml --release
```

Az iced `--auto` nélkül nyitva marad kézi görgetéshez. A Tauri frontend
minden induláskor 5 s-ig auto-scrollol, majd kilép.

## Módszer

- **Virtualizáció:** mindkettő csak a viewport körüli ~28 sort rajzolja ki;
  a 4 000 soros tartományt spacer/offset reprezentálja.
- **FPS:** iced `window::frames()` számláló; Tauri `requestAnimationFrame`
  számláló, 5 s ablakban.
- **RSS:** iced `VmRSS` a saját processből; Tauri a fő process és a
  WebKitWebProcess/NetworkProcess leszármazottak `ps` RSS-összege.
- **Indulás:** iced az app-state létrehozásától az első frame-ig; Tauri a
  `main` kezdetétől az első JS frame `report_startup` hívásáig.
- A Tauri mérés a rendszer WebKitGTK 2.54-et használja; a bináris nem
  tartalmazza a WebKit könyvtárakat.
