# ADR 0004 – daemon ↔ client transport

- **Státusz:** elfogadva
- **Dátum:** 2026-11-19
- **Kontextus:** M1-T3, natív thin client, távoli seedbox/NAS, delta-stream

## Döntés

**gRPC over HTTP/2** (`tonic` 0.14 + `prost`), **rustls** TLS (`tls-ring`),
önaláírt tanúsítvánnyal (rcgen), a kliens a certet CA-ként pinneli, és
`authorization: Bearer <token>` fejlécet küld. A szerződés a
`crates/protocol/proto/ferrent/v1/ferrent.proto`.

## Alternatívák

- **WebSocket + JSON/bináris:** egyszerű böngészős klienshez, de saját
  framing, séma-verziózás, hibakódok és stream-kezelés kell; több kód.
- **REST/JSON:** egyszerű CRUD, de a delta-streamhez külön mechanizmus
  (SSE/WebSocket) és saját típusgenerálás kell.
- **Nyers TCP:** minimális overhead, de minden protokoll-réteget nekünk kell
  megírni.

## Miért gRPC

- Típusos `.proto` szerződés, kliens/szerver codegen, `buf` breaking-check.
- Server streaming a deltákhoz; HTTP/2 multiplexelés egy kapcsolaton.
- Standard státuszkódok: `UNAUTHENTICATED`, `NOT_FOUND`, `FAILED_PRECONDITION`.
- A natív thin client (iced/Tauri) tudja használni; a későbbi WebUIhoz
  gRPC-Web vagy gateway kell.

## TLS és pinning

- Első induláskor a daemon önaláírt certet generál (`rcgen`); a SHA-256
  fingerprintet kiírja.
- A kliens csak ezt a certet trustolja CA-ként; más certnél a handshake
  elbukik (a teszt ezt ellenőrzi). Ez a self-signed cert pinningje.
- A token `authorization: Bearer ...` fejlécben megy; a szerver
  constant-time összehasonlítást használ; rossz token → `UNAUTHENTICATED`.
- A token nem kerül logba; rotálható.

## Tesztek (2026-11-19)

- TLS-en át add/list/pause/resume/remove: **PASS**.
- Rossz token: `UNAUTHENTICATED`.
- Más CA: TLS handshake hiba.
- `Watch`: Added esemény megérkezik; 500 ms idle alatt nincs esemény.
- `buf lint`: PASS; a breaking-check a CI `proto` jobjában fut.

## Következmények

- Protobuf/buf tooling; a `protoc`ot a `protoc-bin-vendored` adja a buildhez.
- A böngészős WebUIhoz később gRPC-Web vagy gateway kell.
- Cert-rotációnál a klienseknek újra kell pinnelniük; token-rotáció külön.
- A gRPC üzenetméret default 4 MB; `.torrent` fájloknál elegendő, de nagy
  metainfo esetén állítható.
