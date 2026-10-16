// HTML bodies in the shapes real senders use: table layouts, inline styles, remote images and
// tracking pixels. Remote URLs use the reserved `.example` TLD (RFC 2606), so they never
// resolve; they exist to exercise remote-image blocking.

const shell = (inner: string, bg = '#f4f5f7') => `<!doctype html>
<html><head><meta charset="utf-8"><style>
body{margin:0;background:${bg};font-family:Helvetica,Arial,sans-serif;color:#1d2433}
a{color:#2a5bd7}
.wrap{max-width:600px;margin:0 auto;background:#ffffff}
</style></head><body>${inner}</body></html>`;

export function newsletter(opts: {
  title: string;
  issue: string;
  items: { heading: string; text: string }[];
  brandColor: string;
}): string {
  const items = opts.items
    .map(
      (i) => `<tr><td style="padding:16px 32px;border-top:1px solid #e6e8ee">
<h3 style="margin:0 0 6px;font-size:17px">${i.heading}</h3>
<p style="margin:0;font-size:14px;line-height:1.55;color:#4a5263">${i.text}</p>
<p style="margin:8px 0 0;font-size:13px"><a href="https://read.example/${encodeURIComponent(i.heading)}">Read more &rarr;</a></p></td></tr>`,
    )
    .join('');
  return shell(`<table class="wrap" width="100%" cellpadding="0" cellspacing="0">
<tr><td style="background:${opts.brandColor};padding:28px 32px;color:#fff">
<img src="https://images.example/newsletter/logo.png" width="120" alt="${opts.title}" style="display:block;margin-bottom:12px">
<div style="font-size:12px;letter-spacing:.08em;text-transform:uppercase;opacity:.8">${opts.issue}</div>
<div style="font-size:24px;font-weight:bold;margin-top:4px">${opts.title}</div></td></tr>
<tr><td><img src="https://images.example/newsletter/hero.jpg" width="600" alt="" style="display:block;width:100%"></td></tr>
${items}
<tr><td style="padding:24px 32px;font-size:12px;color:#8a91a1">You are receiving this because you subscribed.
<a href="https://unsubscribe.example/u/123">Unsubscribe</a></td></tr>
</table><img src="https://track.example/open.gif?u=maya" width="1" height="1" alt="">`);
}

export function receipt(opts: {
  vendor: string;
  orderId: string;
  lines: { item: string; amount: string }[];
  total: string;
}): string {
  const rows = opts.lines
    .map(
      (l) =>
        `<tr><td style="padding:8px 0;border-bottom:1px solid #eee">${l.item}</td><td align="right" style="padding:8px 0;border-bottom:1px solid #eee">${l.amount}</td></tr>`,
    )
    .join('');
  return shell(`<table class="wrap" width="100%" cellpadding="0" cellspacing="0"><tr><td style="padding:32px">
<div style="font-size:13px;color:#8a91a1">${opts.vendor}</div>
<h2 style="margin:6px 0 20px;font-size:22px">Receipt for order ${opts.orderId}</h2>
<table width="100%" cellpadding="0" cellspacing="0" style="font-size:14px">${rows}
<tr><td style="padding:12px 0;font-weight:bold">Total</td><td align="right" style="padding:12px 0;font-weight:bold">${opts.total}</td></tr></table>
<p style="font-size:12px;color:#8a91a1;margin-top:24px">Questions? Reply to this email.</p>
</td></tr></table>`);
}

export function notification(opts: {
  service: string;
  headline: string;
  body: string;
  cta: string;
  href: string;
  accent: string;
}): string {
  return shell(`<table class="wrap" width="100%" cellpadding="0" cellspacing="0"><tr><td style="padding:28px 32px">
<div style="font-weight:bold;color:${opts.accent};font-size:13px">${opts.service}</div>
<h2 style="font-size:18px;margin:10px 0">${opts.headline}</h2>
<div style="font-size:14px;line-height:1.6;color:#3c4354">${opts.body}</div>
<p style="margin:22px 0 0"><a href="${opts.href}" style="background:${opts.accent};color:#fff;padding:9px 16px;border-radius:6px;text-decoration:none;font-size:14px">${opts.cta}</a></p>
</td></tr></table>`);
}

/** A hostile body: the sandbox must keep the script from running. */
export function suspicious(): string {
  return shell(
    `<div class="wrap" style="padding:32px">
<h1 style="color:#c0262d">You have WON!</h1>
<p>Claim your prize before midnight. <a href="https://lucky-draw.example/claim">Claim now</a></p>
<script>document.body.innerHTML = 'script ran: sandbox broken';</script>
<img src="https://lucky-draw.example/pixel.gif" width="1" height="1">
<form action="https://lucky-draw.example/collect"><input name="card" placeholder="Card number"></form>
</div>`,
    '#fff4f4',
  );
}

/** Simple personal HTML, the way webmail clients write a reply. */
export function personal(paragraphs: string[], quote?: { who: string; text: string }): string {
  const ps = paragraphs.map((p) => `<div>${p}</div><div><br></div>`).join('');
  const q = quote
    ? `<div class="gmail_quote"><div>On a previous day, ${quote.who} wrote:</div><blockquote style="margin:0 0 0 .8ex;border-left:1px solid #ccc;padding-left:1ex">${quote.text}</blockquote></div>`
    : '';
  return `<div dir="ltr">${ps}${q}</div>`;
}
