# Sign in with ChatGPT

Sage has an identity-only ChatGPT account section in **Settings > Account**.
It opens the system browser, verifies the returned identity in Rust, and shows
the account name and email. Sign out ends the Sage session; Cancel sign-in stops
an unfinished attempt.

## Registration and configuration

OpenAI currently requires registration for identity-only Sign in with ChatGPT.
Request Sage's own OAuth **public client**, with token-endpoint authentication
method **none**, and register this exact default redirect URI:

```text
http://127.0.0.1:47322/auth/openai/callback
```

See [OpenAI's identity integration guide](https://developers.openai.com/siwc/website)
and [client registration](https://developers.openai.com/siwc/request-client-id).

The publisher sets these environment variables when compiling Sage:

```powershell
$env:DOTS_OPENAI_CLIENT_ID = '<the registered Dots client ID>'
$env:DOTS_OPENAI_REDIRECT_URI = 'http://127.0.0.1:47322/auth/openai/callback'
bun run tauri build
```

For local development, set the same variables before starting Sage. Runtime
environment values override compiled values. The redirect variable is optional
when using the default above. A custom registered callback must use HTTP,
127.0.0.1, an explicit nonzero port, and no credentials, query or fragment.

Never embed a client secret in a desktop application. Do not reuse the Codex,
ChatGPT, or another application's client ID. Without configuration, the account
section explains the missing registration and disables sign-in. The current
checkout has no registered client ID.

## Session and scope

The application requests only `openid profile email`. This identifies the user;
it does not enable GPT model inference or charge the user's ChatGPT plan.
Sage retains the verified issuer/client/subject mapping and available display
name/email in memory for eight hours, until sign-out, or until Sage exits.
It does not save the authorization code, PKCE verifier, ID token, access token,
or refresh token. Signing out does not sign the system browser out of ChatGPT.

Sage remains a local single-user desktop app. This optional account session is
not an operating-system lock or a multi-user data partition.

## Verification

The automated suite exercises a local OpenID Connect provider with RSA-signed
ID tokens: PKCE, exact redirect, state/nonce, signature, issuer, audience,
expiration, future issue time, empty subject, cancellation, replay rejection,
browser failures, port conflicts, and UI transitions.

Live OpenAI sign-in must be checked after registration using the configured
build and a consenting account. Confirm the account display, cancellation,
retry, and sign-out. No live sign-in has been claimed for this checkout.
