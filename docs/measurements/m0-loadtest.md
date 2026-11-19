# M0 load test – 4000 torrents (seed)

| Metrika | Érték | Budget | Eredmény |
|---|---|---|---|
| Torrentek | 4000 | ≥ 4 000 | PASS |
| Payload / torrent | 256 B | – | – |
| Add/load idő | 1.73 s | ≤ 30 s | PASS |
| RSS (add után) | 35.5 MB | – | – |
| RSS (settle után) | 40.4 MB | ≤ 1024 MB | PASS |
| Nyitott fd (settle) | 82 | – | – |
| Aktív CPU (add alatt) | 393.85% | – | – |
| Idle CPU (10 s) | 4.70% | ≤ 5% | PASS |
| Leállás | 0.25 s | – | – |
| Események | 8001 | – | – |
| Esemény/sec | 4627.3 | – | – |
| Progress update | 2000 ms | – | – |

A budget a daemonra vonatkozik; ez a harness a teljes tesztfolyamat RSS-ét méri, tehát felső becslés.
A progress update 2000 ms-onként fut; a state-változások alertből azonnal érkeznek.
