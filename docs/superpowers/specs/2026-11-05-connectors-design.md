# Connectors

- Date: 2026-11-05
- Status: draft for the owner's review (the owner's decisions of 2026-11-05 are in §2; the rest are marked *builder's call*)
- Branch: `feat/connectors`
- Builds on: the MCP/skills spec (the Library, catalogs and the fingerprint guard), the env vault spec (Personal variables, Credential Manager, grants, pinned origins, `confirm.rs`), and the ChatGPT sign-in (`chatgpt.rs`: loopback callback, PKCE, CSRF state).

## 1. Goal

Anyone can connect Gmail, GitHub, Notion and the other apps that agentic apps usually connect to, without writing an MCP server config, so their dots can use them.

Success: on a new **Connectors** screen a person picks GitHub, follows the numbered steps to make a token (an **Open GitHub** button opens the right page), pastes it, ticks two dots and presses **Connect**. Dots stores the token in Credential Manager, adds a `github` MCP server to the Library, checks it, and those two dots can call GitHub's tools on their next run. For Notion the person presses **Sign in with Notion** instead, approves Dots in the browser, and the same happens with no token to copy.

## 2. Decisions (owner, 2026-11-05)

1. Two kinds of connector: **paste a key** (an API key, token or app password the person makes on the app's site), and **built-in sign in** for apps whose hosted MCP server offers OAuth that any client can register with.
2. **Every connector shows step-by-step instructions** for getting its key or signing in, with a button that opens the right page in the browser.

### 2.1 Builder's calls

1. **A connector is a Library MCP server**, not a new kind of thing. Connecting adds an ordinary `mcp_server` item named after the connector (`github`, `notion`), so runs, the gateway, catalogs, the tool review, origins and grants work as they do for any server. Advanced users still add any server by hand in the Library; the Connectors screen links there. *(builder's call)*
2. **Whether a connector is connected is read from the Library**: the item named after it whose config is the catalog's template. No new table, no migration. An item of that name with another config (the person's own) shows the connector as "name taken" and Connect is refused with a message pointing at the Library. *(builder's call)*
3. **Values live in Personal** under names Dots owns, `CONNECTOR_<ID>_<FIELD>` (`CONNECTOR_GITHUB_TOKEN`), never the person's own names, so Disconnect can remove them without breaking anything else. *(builder's call)*
4. **Personal dots only.** A connector's values are Personal variables, which an org's dot never reads; the dot picker lists Personal dots. *(builder's call)*
5. **The first tool list is accepted on Connect.** The person picked the connector from Dots' own list and was told what it lets a dot do; a later change of the list waits for review in the Inbox as for any server. *(builder's call; the alternative is a review step inside the panel)*
6. **Community stdio packages are pinned** to the version that was checked (`npx -y slack-mcp-server@1.3.0`), so a new upload cannot run without a Dots update. *(builder's call)*
7. **A navigation entry of its own** ("Connectors", between Library and Settings), with a link from Settings. *(builder's call)*

## 3. The catalog

One data file, `crates/dots-core/src/connectors/catalog.rs`. Each entry has an id (also the Library name), name, one-line description, category, icon key, **maker** (`official`: the app's own company runs or publishes it; `community`: someone else, named), auth kind (`key`, `oauth`, or `coming_soon`), the MCP server template, the runtime it needs (Node, with a minimum major version) if any, the key fields (label, variable, secret or plain, a format hint and an optional prefix check such as `ghp_`/`github_pat_`), the **instructions** (ordered steps, the page to open and its button label), and what a dot can do with it.

Every server below was checked on 2026-11-05: hosted servers answered 401 with `WWW-Authenticate` (or `200` for Hugging Face without a token, `401` with a bad one); OAuth servers publish protected-resource metadata (or, for Atlassian, authorization-server metadata at the server's origin), S256 PKCE, and a `registration_endpoint` that accepted a test registration with a `http://127.0.0.1:<port>/callback` redirect; npm packages were looked up with `npm view` and their repositories with the GitHub API.

| Id | App | Auth | Maker | Server |
|---|---|---|---|---|
| `gmail` | Gmail | key: address + app password | community (codefuturist) | `npx -y @codefuturist/email-mcp@0.6.0 stdio`, IMAP/SMTP, Node 24+ |
| `github` | GitHub | key: personal access token | official (GitHub) | `https://api.githubcopilot.com/mcp/`, `Authorization: Bearer` |
| `slack` | Slack | key: user token `xoxp-` | community (korotovsky) | `npx -y slack-mcp-server@1.3.0 --transport stdio` |
| `brave-search` | Brave Search | key: API key | official (Brave) | `npx -y @brave/brave-search-mcp-server@2.1.4 --transport stdio` |
| `huggingface` | Hugging Face | key: access token `hf_` | official (Hugging Face) | `https://huggingface.co/mcp`, `Authorization: Bearer` |
| `notion` | Notion | sign in | official (Notion) | `https://mcp.notion.com/mcp` |
| `linear` | Linear | sign in | official (Linear) | `https://mcp.linear.app/mcp` |
| `atlassian` | Jira and Confluence | sign in | official (Atlassian) | `https://mcp.atlassian.com/v1/mcp` |
| `sentry` | Sentry | sign in | official (Sentry) | `https://mcp.sentry.dev/mcp` |
| `todoist` | Todoist | sign in | official (Doist) | `https://ai.todoist.net/mcp` |
| `airtable` | Airtable | sign in | official (Airtable) | `https://mcp.airtable.com/mcp` |
| `stripe` | Stripe | sign in | official (Stripe) | `https://mcp.stripe.com` |
| `cloudflare` | Cloudflare | sign in | official (Cloudflare) | `https://mcp.cloudflare.com/mcp` |
| `google-calendar`, `google-drive` | Google Calendar, Drive | coming soon | — | Google offers no client registration for MCP: it needs a Google OAuth client registered and verified for Dots. Shown, not connectable. |

Left out after checking: Slack's own hosted server, Asana, HubSpot, Box and Figma (their authorization servers have no open client registration, or refused one), and the archived `@modelcontextprotocol/server-slack`.

## 4. Connect with a key

`connector_connect(id, values, dot_ids)` in the app; the core's `Connectors` service does the work.

1. The core checks the id, that every field has a value, the format hint (prefix), and that no other Library item has the name.
2. The app asks the Library's own question in a native dialog (`confirm::connect_confirmed`, built on `confirm_library`): for an http server "Dots will send the values of CONNECTOR_GITHUB_TOKEN to "https://api.githubcopilot.com"…", for a stdio one "Dots will run: "npx" "-y" "slack-mcp-server@1.3.0"… with env …", with the maker named first ("Slack connector, made by korotovsky (community)."). A no stores and runs nothing.
3. The values go to Personal as secrets (or plain, for an email address); a value already there is replaced.
4. The server item is added through `LibraryService::add` (literals check, publish) and its origin recorded as for the server form.
5. It is checked in Personal and the list it gives is accepted (§2.1.5). A failed check keeps the connector, with its error shown (a wrong token shows "the server answered HTTP 401").
6. The picked dots get the server in `uses.mcp_servers` through `Runtime::update_dot`, which grants it its variables (`Grants::synced`) and enforces the role rules. Dots unticked lose it.

A failure before the item is added removes the values it stored. A stdio connector needs Node: the screen asks `connector_runtime` first and shows "Node.js 24 or newer is needed" with a **Get Node.js** button (`https://nodejs.org/en/download`) instead of Connect.

**Change dots** (`connector_set_dots`) and **Test** (`library_check`) reuse the same paths. **Disconnect** (`connector_disconnect`) removes the Library item through `LibraryService::remove(confirm)` (which takes it out of every dot's `uses`; the save drops its grants), then the connector's values and sign-in state.

## 5. Sign in (MCP authorization, spec 2025-06-18)

A self-contained module, `crates/dots-core/src/mcp/oauth.rs`:

1. **Discovery.** An unauthenticated `initialize` to the server; a 401's `WWW-Authenticate: Bearer resource_metadata="…"` names the protected-resource metadata (RFC 9728). Without it, `/.well-known/oauth-protected-resource<path>`, then `/.well-known/oauth-protected-resource`; without any, the server's origin is the authorization server (the 2025-03-26 fallback Atlassian needs). The metadata's `resource` must be on the server's origin.
2. **Authorization server metadata** (RFC 8414, then OpenID discovery): `/.well-known/oauth-authorization-server<path>`, `/.well-known/openid-configuration<path>`, `<issuer>/.well-known/openid-configuration`. The `issuer` must be the server named; every endpoint https (http only on a loopback host, for tests); S256 must be offered.
3. **Dynamic client registration** (RFC 7591) with `client_name: "Dots"`, the loopback redirect, `authorization_code` + `refresh_token`, `token_endpoint_auth_method: none`. A server that answers with a `client_secret` is used as a confidential client (`client_secret_basic`, or `_post` when that is the method it named).
4. **Authorization code with PKCE S256** in the system browser. The callback listens on `127.0.0.1` on a free port (RFC 8252 §7.3), path `/callback`; `state` is random and checked once; the `resource` parameter (RFC 8707) is the server's resource on both the authorization and token requests; `scope` is the 401's `scope`, else the metadata's `scopes_supported`, else none. The attempt ends after 5 minutes, or when cancelled.
5. **Tokens.** The access token is stored as the Personal secret `CONNECTOR_<ID>_ACCESS_TOKEN`, which the item's header reads (`Authorization: Bearer ${CONNECTOR_<ID>_ACCESS_TOKEN}`), so injection, grants and redaction are the vault's for both engines (Dots opens every MCP session itself; the Claude engine reaches it through the gateway). The registration, refresh token, token endpoint, resource and expiry are one JSON record in Credential Manager under `connector/<id>/oauth/<n>`, split into 2,000-byte parts when longer. An access token longer than one secret (2,560 bytes) is refused with a clear message. Tokens are never logged; they are added to the redactor.
6. **Refresh.** Before a run opens the server, and before a Check, a token that expires within 5 minutes is refreshed (`refresh_token` grant with the `resource`). A session that fails with HTTP 401 at a run's start is refreshed and opened once more. A refresh refused with `invalid_grant` (or 400/401) marks the connector **Needs sign-in again**, and the run's notice says "notion: sign in again on the Connectors screen". A network failure keeps the old token.

## 6. The screen

`/connectors`: a short intro, a link "Add your own server in the Library", then a grid of cards grouped by category. A card shows the app's name, its maker ("by Notion · official" / "by korotovsky · community"), the line of what it lets dots do, and a status: **Not connected**, **Connected** (with "Used by …"), **Needs sign-in again**, **Error** (the check's message), or **Coming soon**.

Clicking a card opens a side panel (dialog):

- **Key connectors:** "How to get your key", the numbered steps, an **Open <site>** button; the field(s) with their hint (`Starts with github_pat_ or ghp_`); for a stdio connector the runtime note; the maker line; the dot picker (checkboxes of Personal dots); **Connect**.
- **Sign-in connectors:** the steps (what will happen), **Sign in with <App>**, the dot picker.
- **Connected:** which dots use it and **Save dots**, the tool count, **Test**, **Replace key** / **Sign in again**, and **Disconnect** (asks first).

Built from the Library's parts (`ItemRow`-style borders, `Badge`, `Button`, `Dialog`, `Field`, `ConfirmButton`, `notifyError`, `isDeclined`). Every control has a label; the status is text, not only colour.

## 7. Security notes

- No secret is stored in a Library config: templates hold `${CONNECTOR_…}` references only, and the existing literal check runs on add.
- The native confirmation is the Library's: the webview cannot connect a server or send a variable without the person's yes. It names the maker of a community connector, whose package receives the secret.
- Community packages are pinned to a checked version; npx still downloads it from npm on first use.
- OAuth: PKCE S256, single-use random state, loopback-only callback on a random port, exact `resource`, https endpoints, issuer check, no redirects followed by the token client, 5-minute limit. Tokens live in Credential Manager and are scrubbed from logs and transcripts.
- Connectors are Personal; an org's dot never reads them.

## 8. Not in this change

- Google Calendar and Drive (§3).
- Refreshing a token in the middle of a long run: a token is refreshed at the run's start when it has under 5 minutes left; a run that outlives its token fails on its next call to that server.
- Connectors for an org's dots, and several accounts of one app.
- Revoking a token at the provider on Disconnect (Dots forgets it; the person can revoke it on the app's site, which the panel says).
