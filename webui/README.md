# Ferrent web UI

A Deluge Mobile UI (webui2) portja a Ferrent REST API-ra. A komponensek a
Deluge szókincsét használják (`download_payload_rate`, 0–100 progress), ezért
a `src/api.ts` adapter minden választ arra képez; a gateway nem támogatott
végpontjai (queue, per-file priority, push) semleges értéket adnak.

```bash
npm ci
npm run typecheck
npm test          # vitest
npm run build     # dist/
npx playwright test   # mock API ellen, 390/360 px
```

A `ferrent-web` a `webui/dist`-et szolgálja ki, ha a configban
`spa_dir = "/path/to/webui/dist"` szerepel (SPA-fallbackkel).
