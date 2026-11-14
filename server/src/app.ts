import type { IncomingMessage, ServerResponse } from 'node:http';
import { createPrivateKey, sign } from 'node:crypto';
import type { KeyObject } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { pickLanguage, texts } from './i18n.ts';

// The licence server. It keeps no database: a licence key is the buyer's
// email signed with Ed25519, so the paid Stripe checkout session is the only
// record, and opening its "thanks" link again shows the same key.

export interface CheckoutSession {
  status?: string;
  payment_status?: string;
  customer_details?: { email?: string | null } | null;
}

export interface Deps {
  /** Stripe Payment Link that /buy redirects to. */
  paymentLink: string;
  /** Loads a Stripe Checkout Session, or null if Stripe doesn't know it. */
  fetchSession: (id: string) => Promise<CheckoutSession | null>;
  /** Ed25519 private key that signs licence keys. */
  signingKey: () => KeyObject;
}

const KEY_PREFIX = 'MAC1';
const SESSION_ID = /^cs_(test|live)_[A-Za-z0-9]{10,200}$/;

let cachedKey: KeyObject | undefined;

const defaults: Deps = {
  paymentLink: process.env.PAYMENT_LINK_URL ?? '',
  signingKey: () =>
    (cachedKey ??= createPrivateKey(
      readFileSync(process.env.LICENSE_KEY_FILE ?? '/run/secrets/license-signing-key.pem'),
    )),
  fetchSession: async (id) => {
    const res = await fetch(`https://api.stripe.com/v1/checkout/sessions/${id}`, {
      headers: { authorization: `Bearer ${process.env.STRIPE_SECRET_KEY ?? ''}` },
    });
    if (res.status === 404) return null;
    if (!res.ok) throw new Error(`Stripe answered ${res.status}`);
    return (await res.json()) as CheckoutSession;
  },
};

/** `MAC1.<base64url email>.<base64url signature>`, checked offline by the app. */
export function issueKey(email: string, key: KeyObject): string {
  const payload = Buffer.from(email, 'utf8');
  return `${KEY_PREFIX}.${payload.toString('base64url')}.${sign(null, payload, key).toString('base64url')}`;
}

// All routes. Kept apart from main.ts so tests can call it without a socket.
export async function handle(
  req: IncomingMessage,
  res: ServerResponse,
  deps: Deps = defaults,
): Promise<void> {
  const url = new URL(req.url ?? '/', 'http://localhost');
  if (req.method !== 'GET') return json(res, 404, { error: 'not found' });

  if (url.pathname === '/healthz') return json(res, 200, { ok: true });

  if (url.pathname === '/buy') {
    if (!deps.paymentLink) return json(res, 503, { error: 'not on sale yet' });
    res.writeHead(302, { location: deps.paymentLink });
    res.end();
    return;
  }

  if (url.pathname === '/thanks') {
    const lang = pickLanguage(req.headers['accept-language']);
    const t = texts[lang];
    const id = url.searchParams.get('session_id') ?? '';
    if (!SESSION_ID.test(id)) {
      return page(res, 400, lang, t.incompleteTitle, escapeHtml(t.incompleteBody));
    }
    const session = await deps.fetchSession(id);
    const email = session?.customer_details?.email?.trim().toLowerCase();
    if (!session || session.payment_status !== 'paid' || !email) {
      return page(res, 402, lang, t.notPaidTitle, escapeHtml(t.notPaidBody));
    }
    const key = issueKey(email, deps.signingKey());
    return page(
      res,
      200,
      lang,
      t.thanksTitle,
      escapeHtml(t.thanksBody)
        .replace('{email}', escapeHtml(email))
        .replace('{unlock}', `<b>${escapeHtml(t.unlock)}</b>`)
        .replace('{activate}', `<b>${escapeHtml(t.activate)}</b>`),
      key,
    );
  }

  json(res, 404, { error: 'not found' });
}

export function escapeHtml(text: string): string {
  return text.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
}

/** `body` is HTML: callers escape everything they put in it. */
function page(res: ServerResponse, status: number, lang: string, title: string, body: string, key?: string): void {
  const t = texts[lang];
  const keyBlock = key
    ? `<textarea id="k" readonly rows="4">${escapeHtml(key)}</textarea>
<button data-done="${escapeHtml(t.copied)}" onclick="navigator.clipboard.writeText(document.getElementById('k').value);this.textContent=this.dataset.done">${escapeHtml(t.copy)}</button>`
    : '';
  res.writeHead(status, {
    'content-type': 'text/html; charset=utf-8',
    'cache-control': 'no-store',
    'referrer-policy': 'no-referrer',
    'x-robots-tag': 'noindex',
    vary: 'accept-language',
  });
  res.end(`<!doctype html><html lang="${lang}"><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escapeHtml(title)}</title>
<style>body{font:16px/1.5 system-ui,sans-serif;max-width:34rem;margin:3rem auto;padding:0 1rem;color:#222}
textarea{width:100%;font:14px monospace;padding:.5rem;box-sizing:border-box}
button{font:inherit;padding:.5rem 1rem;margin-top:.5rem}</style>
<h1>${escapeHtml(title)}</h1><p>${body}</p>${keyBlock}</html>`);
}

export function json(res: ServerResponse, status: number, body: unknown): void {
  res.writeHead(status, { 'content-type': 'application/json' });
  res.end(JSON.stringify(body));
}
