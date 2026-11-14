#!/bin/sh
# Runs on the backend host, as root, from a freshly unpacked copy of server/.
# `./factory.ps1 deploy mp3-converter` calls it. By hand:  sudo sh install.sh /tmp/mp3-converter-release
# Replaces the code; keeps deploy/.env, deploy/secrets/ and the data volume.
set -eu
SRC=${1:?unpacked release folder}
APP=/srv/apps/mp3-converter

mkdir -p "$APP/deploy/secrets"
KEEP=$(mktemp -d)
if [ -f "$APP/deploy/.env" ]; then cp -p "$APP/deploy/.env" "$KEEP/.env"; fi
cp -a "$APP/deploy/secrets" "$KEEP/secrets"
find "$APP" -mindepth 1 -delete
cp -a "$SRC/." "$APP/"
rm -rf "$APP/deploy/secrets"
cp -a "$KEEP/secrets" "$APP/deploy/secrets"
if [ -f "$KEEP/.env" ]; then cp -p "$KEEP/.env" "$APP/deploy/.env"; fi
rm -rf "$KEEP" "$SRC"

cd "$APP/deploy"
if [ ! -f .env ]; then
  cp app.env.example .env
  chmod 600 .env
  echo "First deploy: fill in $APP/deploy/.env (and any keys in $APP/deploy/secrets/), then deploy again." >&2
  exit 3
fi
# The container runs as uid 1000 and must be able to read the secrets.
chown -R 1000:1000 secrets
chmod 700 secrets
find secrets -type f -exec chmod 600 {} +

DOMAIN=$(sed -n 's/^APP_DOMAIN=//p' .env | tr -d '"\r ')
[ -n "$DOMAIN" ] || { echo "Set APP_DOMAIN in $APP/deploy/.env" >&2; exit 1; }

docker compose up -d --build
app-site mp3-converter "$DOMAIN" mp3-converter:8080
docker image prune -f >/dev/null
sleep 3
docker compose ps
docker compose logs --tail 15 server
echo "Live at https://$DOMAIN/healthz"
