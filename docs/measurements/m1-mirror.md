# M1-T2 – state mirror (4 000 torrents)

Mérve 2026-11-19, Debian 13.7, debug build.

## Query p95

- 4 000 torrent, szűrő `torrent-1` (~1 000 találat), rendezés `download_rate`
  csökkenő, lapozás 50.
- 10 warmup + 100 mért query.
- **p95: 3,1 ms** (budget: ≤ 50 ms).
- A read path `ArcSwap` snapshot; a query nem vesz fel lockot és nem hívja az
  engine-t.

## Delta-kezelés

- 100 000 progress esemény egy torrentre: a mirror map mérete **1** marad.
- A dirty set torrent ID-ket tartalmaz; `Removed` esetén is, hogy a
  perzisztencia törölni tudja.
- A `run_mirror` task 100 ms-onként publikál; leállításkor a maradék
  eseményeket draineli és publikál.

## Megjegyzés

A mérés debug buildben készült; release-ben a p95 ennél alacsonyabb. A
`p95` a `state::tests::query_p95_is_under_50ms_at_4000_torrents` tesztben
újrafuttatható.
