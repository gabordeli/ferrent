# ADR 0003 – GUI keretrendszer

- **Státusz:** elfogadva
- **Dátum:** 2026-11-19
- **Kontextus:** M2-T1, 4 000 soros virtualizált tábla, alacsony RAM, Linux/Windows/macOS thin client

## Döntés

A thin client **iced 0.13** (wgpu + tiny-skia) lesz, saját canvas-alapú,
virtualizált táblával.

## Alternatívák

- **Tauri 2 + WebKitGTK/HTML:** ismerős HTML/CSS/JS tábla, gyors prototípus,
  de WebKitGTK/GTK3 rendszerfüggőség és külön WebKit-folyamatok.
- **egui:** nem mértük; az immediate-mode modell a nagy virtualizált táblához
  és a hosszú távú natív widgetekhez kevésbé illik.

## Mérések (release, Debian 13.7, Intel Iris Xe, 1100×700, 5 s auto-scroll)

| Metrika | iced 0.13.1 | Tauri 2.12 (WebKitGTK 2.54) |
|---|---|---|
| Indulás (process → első frame) | **600 ms** | 764–868 ms |
| FPS | **59,0** | 57,7–58,6 |
| RSS (process tree) | **98,6 MB** | 515–518 MB |
| Bináris | 20 MB | **11 MB** |
| Látható sorok rajzolva | 28 | 28 |
| Virtualizáció | saját canvas, csak a viewport | JS virtual list |

Debug buildben ugyanez: iced 774 ms / 119 MB, Tauri 852 ms / 527 MB.
A Tauri két release futása 764 és 868 ms indulást, 57,7–58,6 FPS-t és
515–518 MB RSS-t adott; a különbség futáson belüli szórás, a döntést nem
befolyásolja. A Tauri RSS a fő process + WebKitWebProcess/NetworkProcess
összege; a WebKit rendszerkönyvtárak nem számítanak a binárisméretbe.

## Miért iced

- Az **150 MB-os kliens-RSS budget** alatt van; a Tauri a 250 MB-os felső
  becslést is jelentősen túllépi.
- Egy natív folyamat, nincs WebKitGTK/GTK3 rendszerfüggőség; a csomagolás
  kiszámíthatóbb.
- A canvas csak a látható ~28 sort rajzolja; 4 000 sornál 59 FPS.
- A tábla a termék magja; a saját widget/scroll költsége elfogadható.
- A Tauri HTML/JS fejlesztési sebessége előny, de a RAM-dominancia miatt nem
  elég.

## Következmények

- A valódi GUI (M2-T2+) iced-del készül; a Tauri spike a
  `crates/gui/spikes/tauri-spike/` alatt marad, a workspace-ből és a release
  binárisból kizárva.
- A canvas-tábla saját scroll/input kezelést igényel (M2-T3).
- Ha később a WebUI/gRPC-Web válik fontossá, a Tauri újranyitható.
- Screenshotok: `docs/measurements/m2-iced.png`, `docs/measurements/m2-tauri.png`
  (XWayland alatt, a layout vizuális ellenőrzéséhez).
