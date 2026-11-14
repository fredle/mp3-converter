import { createServer } from 'node:http';
import { handle } from './app.ts';

const port = Number(process.env.PORT ?? 8080);

const server = createServer((req, res) => {
  handle(req, res).catch((err: unknown) => {
    console.error(err);
    if (!res.headersSent) res.writeHead(500, { 'content-type': 'application/json' });
    res.end('{"error":"internal"}');
  });
});

server.listen(port, () => console.log(`listening on :${port}`));
for (const sig of ['SIGTERM', 'SIGINT'] as const) {
  process.on(sig, () => server.close(() => process.exit(0)));
}
