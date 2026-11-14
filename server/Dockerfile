FROM node:24-slim
WORKDIR /app
ENV NODE_ENV=production NODE_NO_WARNINGS=1
COPY package.json ./
RUN npm install --omit=dev --no-audit --no-fund
COPY src ./src
# Data lives on a mounted volume so it survives redeploys. A new named volume
# copies this folder's owner, so the unprivileged user can write to it.
ENV DATA_DIR=/data PORT=8080
RUN mkdir -p /data && chown node:node /data
VOLUME /data
EXPOSE 8080
USER node
CMD ["node", "src/main.ts"]
