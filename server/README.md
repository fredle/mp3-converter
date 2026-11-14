# MP4 to MP3 & Audio Converter server

The backend for MP4 to MP3 & Audio Converter: Node 24 running TypeScript directly (type
stripping, so no enums, namespaces or parameter properties) with no build step.

- `npm install`, then `npm test` and `npx tsc` (type check only).
- `npm start` listens on `PORT` (default 8080). `GET /healthz` answers `{"ok":true}`.
- Settings come from environment variables. On the host they live in
  `/srv/apps/mp3-converter/deploy/.env` (see `deploy/app.env.example`).
- Persistent data goes under `DATA_DIR` (`/data` in the container, a named volume).
- Secret files (keys) go in `/srv/apps/mp3-converter/deploy/secrets/` on the host and
  appear read-only under `/run/secrets/` in the container.

## Deploy

From app-factory: `./factory.ps1 deploy mp3-converter`. It packs this folder (never
`.env`, secrets, `node_modules` or databases), copies it to the shared backend
host over IAP and runs `deploy/install.sh` there. That rebuilds the container
and routes `APP_DOMAIN` to it with automatic HTTPS.

The first deploy stops after creating `deploy/.env` on the host. Fill it in
(`./factory.ps1 host` prints the SSH command), point the domain's DNS A record
at the host's IP, and deploy again.
