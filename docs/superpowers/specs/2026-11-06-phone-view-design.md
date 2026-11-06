# Phone view: answer Inbox cards from your phone on the same Wi-Fi

- Date: 2026-11-06
- Status: approved (owner's request, 2026-11-06: "Same Wi-Fi + pairing. Dots serves a
  phone-sized page on your home network. You pair once by scanning a QR code. Works only when
  the phone is on the same network. No cloud.")
- Builds on: the Inbox (approvals, questions from dots, runs held for usage), Settings, the
  native confirmation dialog (`confirm.rs`), DPAPI (`vault::org::offline`).

## 1. Goal

Answer what waits for you in Dots (approve or deny a call, answer a dot's question, start a
run held for usage) from a phone on the same home network, without a cloud service and without
loosening the local server on 127.0.0.1.

## 2. Behaviour

1. **Off by default.** Settings > Phone has an "Answer from my phone" switch
   (`phone_enabled`, default off) and a port (`phone_port`, default 47323, not the local
   server's). It is part of the one Settings draft and applies at Save, without a restart. Off
   closes the listener and every open connection at once and forgets a pairing in progress.
2. **A listener of its own.** On, Dots binds the phone port on this PC's private addresses only
   (`phone::net`): IPv4 10/8, 172.16/12, 192.168/16, 169.254/16, IPv6 fc00::/7 (fe80::/10 is
   private too but left out: browsers take no zone in a URL). Public addresses, loopback,
   100.64/10 (carrier NAT, Tailscale), documentation and multicast ranges are never bound. The
   addresses come from the interfaces that are up (`GetAdaptersAddresses`, loopback and tunnel
   interfaces skipped), the default route's first. A connection from a non-private address is
   dropped before TLS. Dots looks again every 30 seconds and drops addresses that went away, but
   never moves to another network by itself: the network it serves is the one the PC was on when
   the switch was turned on, or when Dots started with it on. Leaving it stops the listener with
   a reason in Settings.
3. **HTTPS with a self-signed certificate.** Made on first use (ECDSA P-256, ten years,
   `rcgen` with `ring`, served by `tokio-rustls`, both of which Dots already had through
   `rustls`), kept in `phone_tls` with its key encrypted by DPAPI for the signed-in Windows user,
   so the fingerprint stays the same across restarts. Settings shows its SHA-256 fingerprint; the
   pairing link carries it too.
4. **Pairing.** "Pair a phone" shows a QR code (drawn from an SVG path the core makes with
   `qrcode`; no image, no injected markup), the address, a short code and the fingerprint.
   - The QR code's link is `https://<address>:<port>/#t=<token>&fp=<fingerprint>`. The token is
     256 random bits, good for 5 minutes and one claim. It is after `#`, so the browser never
     sends it in a request line or a Referer; the page posts it and wipes it from the address bar
     and history at once.
   - The short code (8 characters from 31 that do not read alike, ~40 bits) is for a phone that
     cannot scan. The token and the code are one pairing: either one uses it up. Five wrong
     guesses end the pairing.
   - The phone sends the token or code with a name (at most 40 characters; control, format and
     invisible characters refused as for `claude_path`). It gets a claim secret and a 4-digit
     number, and shows the number.
   - **The PC asks**: "Pair a phone with Dots?", naming the phone, its address and the number,
     in a native dialog the phone and the webview cannot answer, and which `DOTS_CONFIRM=yes`
     does not answer either (`confirm::ask_in_dialog`). The window comes to the front first. The
     claim waits 2 minutes; an answer after that is ignored. One claim at a time.
   - Allowed: the phone collects its credential within 60 seconds, once. The credential is 256
     random bits; only its SHA-256 is stored (`phone_devices.token_hash`). It is sent as
     `__Host-dots_phone=…; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=400 days`, renewed
     on every page load.
5. **Paired phones** are listed in Settings > Phone with name, when paired, last seen and from
   where, and **Remove**, which deletes the row: the phone's next request is refused (every
   request reads the row). A phone can also "Unpair this phone" itself. A phone unused for 90
   days is unpaired at its next request.
6. **The phone page** (served by the listener from three files compiled into Dots; the desktop
   app's bundle is untouched) shows, polling every 4 seconds while visible:
   - pending approvals: "<dot> asks to use <tool>" (the Inbox's words), the call's input exactly
     as indented JSON, the parked note when parked, and **Approve once** / **Deny**. There is no
     "always allow" on a phone;
   - open questions from dots, with their choices as buttons and a text answer;
   - runs held for Claude usage with **Run anyway** (a run held for a role shows why, without the
     button).
   Snoozed and done items are left out, as in the Inbox. Nothing else: no settings, dots, chat,
   history or files.
7. **Same decisions as the app.** The phone's actions call `Runtime::decide_approval` (with no
   rule), `Runtime::reply` and `Runtime::run_release_hold`, so account roles, the private-folder
   guard, the conflict on a second decision and the resume of a parked run are exactly the app's.
   A decided approval records `decided_via = "phone: <name>"` (migration 0043) and is announced
   with `approval_updated`. Every phone action (paired, approve, deny, answer, run anyway,
   unpaired itself) is written to `phone_actions`; Settings > Phone lists the latest.
8. **Windows firewall.** Binding a new port may make Windows ask whether Dots may use the
   network. Settings says so and says to allow private networks only. Dots never changes
   firewall rules.

## 3. Server hardening (`phone::server`)

- `Host` must be exactly the address and port the connection reached (DNS rebinding reaches
  nothing; 421 otherwise). Only GET, HEAD and POST; no preflight is answered and no CORS header
  is ever sent.
- Every POST must carry `Origin: https://<that address>:<port>` and a JSON body (403 / 415):
  with the `SameSite=Strict` cookie this is the CSRF defence.
- Every route but the page's three files and the pairing exchange (`/api/pair`,
  `/api/pair/wait`) needs a paired phone's cookie; the files hold no data.
- Response headers on everything: `Content-Security-Policy: default-src 'none'; script-src 'self';
  style-src 'self'; img-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none';
  frame-ancestors 'none'`, `X-Frame-Options: DENY`, `nosniff`, `Referrer-Policy: no-referrer`,
  `Cache-Control: no-store`, COOP/CORP same-origin. The page builds its DOM with `textContent`.
- Limits: 16 KiB bodies, answers of 8000 characters, 10 s TLS handshake, 10 s to send headers,
  20 s per request, 2 minutes per connection, 64 connections, 8 per address. The audit trail
  keeps the newest 1000 actions.
- Rate limits: 10 failures (a wrong cookie, a wrong code or token, an unknown claim) per address
  in 10 minutes, then 429 until the window ends; checked before the cookie is read. A request
  with no cookie is refused without counting: it guesses nothing, and counting it would let any
  web page the phone opens lock the phone out (the cookie stays off cross-site requests).
- The credential is looked up by its hash and compared in constant time; tokens, codes and claim
  secrets are compared in constant time.

## 4. Decision: TLS, not application-level protection

Plain HTTP on Wi-Fi can be read by anyone on the network, and a page served over HTTP can be
changed in flight, so protection added by the page itself could be removed by the same
attacker. Self-signed TLS protects every request after the first visit against anyone who only
listens, and lets the cookie be `Secure`. Everything it needs was already in Dots' dependencies
except `rcgen` (0.13, with `ring`, which Dots already had; two small new crates: `rcgen`,
`yasna`) and `qrcode` (no dependencies). The cost is the browser's warning on the first visit,
which the person accepts once, ideally after comparing the fingerprint.

## 5. Residual risks

- **An active attacker at pairing time.** Someone on the same Wi-Fi who can redirect traffic
  (ARP spoofing) when the phone first opens the page can present their own certificate. The
  browser warns for Dots' certificate too, so the warning does not tell them apart: only the
  fingerprint does, and a person may not compare it. A relay passes the 4-digit number along
  unchanged, so the number does not stop it; the dialog then shows the relay's address instead of
  the phone's, and asks the person to check the address. This is trust on first use.
- **A person who allows without checking** the name, address and number lets in whoever claimed
  the pairing first.
- **Untrusted networks.** Dots listens on the private network it was on when the switch was
  turned on, or when Dots started with it on. If the PC starts on a café's network with the
  switch on, it listens there. Every request still needs a paired phone's cookie. Turn it off
  when away from home.
- **Other private networks that can route to the PC.** Any private source address is served, not
  only the PC's own subnet: hosts on a company VLAN or behind a VPN that routes private ranges
  to the PC can reach the listener (they still need a paired phone's cookie). Addresses of
  virtual adapters (Hyper-V, WSL, some VPN clients) that are not of the tunnel type are bound too.
- **A lost or stolen phone** can answer the Inbox until it is removed in Settings > Phone (its
  screen lock is the only guard until then).
- **Denial of service** on the network: someone can end a pairing with five wrong guesses (from
  several addresses, past the per-address limit), fill the connection slots from several
  addresses, or keep a claim busy for 2 minutes.
- **The address changes** (a new DHCP lease): the cookie belongs to the old address, so the phone
  must pair again; the certificate stays.
- **Local software** on the PC with the user's rights can read the database and decrypt the
  certificate's key; that was already the case for everything else Dots keeps.

## 6. Tests

- `phone::net`: private ranges accepted; public, loopback, CGNAT, documentation, multicast,
  mapped public addresses refused; picking, order and IPv6 link-local exclusion.
- `phone::pairing`: code alphabet; a token claims once; the code claims; expiry at 5 minutes;
  five wrong guesses end it; the person's yes is needed; refused and late claims give nothing;
  one claim at a time; the rate limiter.
- `phone::server`: cookie parsing and attributes; private peers only.
- `tests/phone_test.rs` (a real runtime, the listener on 127.0.0.1): off by default; off closes
  the listener at once and keeps the certificate; security headers and no CORS on every
  response; pairing by token and by code with the PC's yes, the token single-use, only the hash
  stored; a refused phone gets nothing; wrong codes end the pairing and failures are rate
  limited, while requests without a cookie are not counted; every API route needs a paired
  phone; a phone unused for 90 days is unpaired; missing or foreign Origin, non-JSON, wrong
  Host and big bodies refused; Remove revokes at once; a phone unpairs itself; approvals,
  answers and Run anyway go through the runtime (events, conflicts, `decided_via`, the audit
  trail, no rule saved).
- UI: Settings > Phone off by default and Save turns it on; pairing shows the QR code, code and
  fingerprint and cancels; paired phones listed and removed after asking; why it is not
  listening; the phone port's checks.
