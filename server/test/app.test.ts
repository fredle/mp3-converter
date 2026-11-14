import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import type { AddressInfo } from 'node:net';
import { generateKeyPairSync, verify } from 'node:crypto';
import { handle, issueKey, escapeHtml } from '../src/app.ts';
import type { CheckoutSession, Deps } from '../src/app.ts';
import { pickLanguage, texts } from '../src/i18n.ts';

const { publicKey, privateKey } = generateKeyPairSync('ed25519');
const SESSION = 'cs_test_a1B2c3D4e5F6g7H8';

async function withServer(
  sessions: Record<string, CheckoutSession>,
  run: (origin: string) => Promise<void>,
  paymentLink = 'https://buy.stripe.com/test_123',
): Promise<void> {
  const deps: Deps = {
    paymentLink,
    signingKey: () => privateKey,
    fetchSession: async (id) => sessions[id] ?? null,
  };
  const server = createServer((req, res) => void handle(req, res, deps));
  await new Promise<void>((resolve) => server.listen(0, resolve));
  try {
    await run(`http://127.0.0.1:${(server.address() as AddressInfo).port}`);
  } finally {
    server.close();
  }
}

test('healthz answers ok', async () => {
  await withServer({}, async (origin) => {
    const res = await fetch(`${origin}/healthz`);
    assert.equal(res.status, 200);
    assert.deepEqual(await res.json(), { ok: true });
  });
});

test('issued keys carry the email and a valid signature', () => {
  const [prefix, payload, signature, extra] = issueKey('buyer@example.com', privateKey).split('.');
  assert.equal(prefix, 'MAC1');
  assert.equal(extra, undefined);
  assert.equal(Buffer.from(payload, 'base64url').toString(), 'buyer@example.com');
  assert.ok(verify(null, Buffer.from('buyer@example.com'), publicKey, Buffer.from(signature, 'base64url')));
  assert.ok(!verify(null, Buffer.from('other@example.com'), publicKey, Buffer.from(signature, 'base64url')));
});

test('buy redirects to the payment link, or says so when there is none', async () => {
  await withServer({}, async (origin) => {
    const res = await fetch(`${origin}/buy`, { redirect: 'manual' });
    assert.equal(res.status, 302);
    assert.equal(res.headers.get('location'), 'https://buy.stripe.com/test_123');
  });
  await withServer({}, async (origin) => assert.equal((await fetch(`${origin}/buy`)).status, 503), '');
});

test('a paid session shows the same key every time, for the lower-cased email', async () => {
  const sessions = { [SESSION]: { payment_status: 'paid', customer_details: { email: ' Buyer@Example.com ' } } };
  await withServer(sessions, async (origin) => {
    const res = await fetch(`${origin}/thanks?session_id=${SESSION}`);
    assert.equal(res.status, 200);
    assert.equal(res.headers.get('cache-control'), 'no-store');
    const html = await res.text();
    const key = issueKey('buyer@example.com', privateKey);
    assert.ok(html.includes(key));
    assert.ok((await (await fetch(`${origin}/thanks?session_id=${SESSION}`)).text()).includes(key));
  });
});

test('unpaid, unknown and malformed sessions get no key', async () => {
  const sessions = {
    [SESSION]: { payment_status: 'unpaid', customer_details: { email: 'a@b.c' } },
    cs_test_noemail00000: { payment_status: 'paid', customer_details: null },
  };
  await withServer(sessions, async (origin) => {
    for (const [query, status] of [
      [`session_id=${SESSION}`, 402],
      ['session_id=cs_test_noemail00000', 402],
      ['session_id=cs_test_unknown000000', 402],
      ['session_id=../../etc/passwd', 400],
      ['', 400],
    ] as const) {
      const res = await fetch(`${origin}/thanks?${query}`);
      assert.equal(res.status, status, query);
      assert.ok(!(await res.text()).includes('MAC1.'), query);
    }
  });
});

test('pages follow the browser language, falling back to English', async () => {
  const sessions = { [SESSION]: { payment_status: 'paid', customer_details: { email: 'buyer@example.com' } } };
  await withServer(sessions, async (origin) => {
    const get = async (language: string) =>
      (await fetch(`${origin}/thanks?session_id=${SESSION}`, { headers: { 'accept-language': language } })).text();
    const key = issueKey('buyer@example.com', privateKey);
    for (const [language, lang, phrase] of [
      ['de-DE,de;q=0.9,en;q=0.8', 'de', 'Vielen Dank'],
      ['ja', 'ja', 'ライセンスキー'],
      ['zh-TW', 'zh-hant', '授權金鑰'],
      ['zh-CN', 'zh-hans', '许可证密钥'],
      ['sv-SE,sv;q=0.9', 'en', 'Thank you'],
      ['en;q=0.5,fr;q=0.9', 'fr', 'Merci'],
    ]) {
      const html = await get(language);
      assert.ok(html.includes(`<html lang="${lang}">`), language);
      assert.ok(html.includes(phrase), language);
      assert.ok(html.includes(key), language);
      assert.ok(html.includes('buyer@example.com') && !/\{(email|unlock|activate)\}/.test(html), language);
    }
  });
});

test('every language has every text and keeps the placeholders', () => {
  const keys = Object.keys(texts.en).sort();
  assert.equal(Object.keys(texts).length, 14);
  for (const [lang, t] of Object.entries(texts)) {
    assert.deepEqual(Object.keys(t).sort(), keys, lang);
    for (const value of Object.values(t)) assert.ok(value.trim(), lang);
    for (const placeholder of ['{email}', '{unlock}', '{activate}']) {
      assert.equal(t.thanksBody.split(placeholder).length, 2, `${lang} ${placeholder}`);
    }
  }
  assert.equal(pickLanguage(undefined), 'en');
});

test('other paths and methods are not found', async () => {
  await withServer({}, async (origin) => {
    assert.equal((await fetch(`${origin}/nope`)).status, 404);
    assert.equal((await fetch(`${origin}/thanks`, { method: 'POST' })).status, 404);
  });
});

test('html is escaped', () => {
  assert.equal(escapeHtml(`<a href="x">&'`), '&#60;a href=&#34;x&#34;&#62;&#38;&#39;');
});
