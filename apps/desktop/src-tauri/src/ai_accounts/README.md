# Native subscription authentication

Account data and credentials belong to Nero. The module does not inspect another app's tokens, cookies, configuration or environment API keys. Its public snapshots contain account labels and state; authorization URLs, access tokens and refresh tokens stay in Rust. This module does not create API keys, select a paid API route or silently change providers.

Browser callbacks share the native OAuth status page with Calendar. A validated callback only starts its progress state; success is emitted after token validation and credential persistence finish. Cancellation, rejected callbacks and abandoned attempts show failure. The self-contained page strips the callback query from browser history, prevents caching and attempts to close only after success, retaining a manual-close fallback. It never renders callback codes, tokens or account details.

ChatGPT uses the documented OSS/local direct SIWC registration, initially `dynamic_agent_client`, with the actual application name Nero and a stable per-host identifier. The returned issued client ID is retained before exchanging the authorization code. Code exchange and refresh use that issued ID. RSA/RS256 ID tokens are verified against the published OpenAI JWKS before issuer, audience, expiry, nonce and selected-account identity are accepted. Scopes returned by the token endpoint determine plan access. A sign-in without direct plan permission remains visible but cannot invoke a model.

Claude direct OAuth and token invocation are disabled. Previously stored Claude credentials are retained but are not presented as ready or used for inference. The official Claude Code transport is documented in `../claude_runner/README.md`.

Credentials use Nero's dedicated macOS Keychain/Windows Credential Manager service. A verified chunk generation and a single committed index preserve a complete rotating credential set; individual chunks fit the Windows credential blob limit. Public status polling uses an in-memory projection or a persisted, secret-free account list. After restart, persisted accounts have unknown authentication state until an explicit action resolves credentials. Concurrent session requests for the same account share an in-flight transaction; later transactions read credentials under the operation and cross-process locks. Credential access errors are latched until an explicit retry. Refreshes serialize in-process and across processes. Cancellation stops the caller, while an already-started bounded refresh transaction finishes storing any accepted replacement credential set. Cancelled or superseded browser attempts cannot commit an active account. Temporary network errors preserve the saved grant; confirmed terminal refresh failures clear unusable credentials while keeping registration/host mapping.

Before quitting or installing an update, the desktop lifecycle blocks new requests and retains the guard returned by `prepare_shutdown`. This cancels browser sign-in and drains in-flight credential operations with a deadline derived from the HTTP request timeout. The guard rejects queued account operations until the process exits or the transition is abandoned. Dropping the guard restores account access; a drain timeout leaves the running credential transaction intact and prevents a successful shutdown acknowledgment.

ChatGPT sign-out attempts the revocation endpoint published in OpenID discovery and reports whether revocation was confirmed. Legacy Claude credentials are retained; this module does not log the user out of Claude Code.

Protocol sources:
- https://developers.openai.com/siwc/token-sharing-open-source/sign-in
- https://developers.openai.com/siwc/token-sharing-open-source/profiles-and-sessions
- https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference
- https://developers.openai.com/siwc/token-sharing-open-source/errors-and-recovery
- https://github.com/earendil-works/pi/blob/c9f4c7b9fcd0f3e6b5bbc35c7c9b6cda35580eb8/packages/ai/src/auth/oauth/anthropic.ts

The Pi MIT notice is retained in PI-LICENSE.txt. Synthetic verification fixtures include only a public RSA test key and an offline-signed fake ID token. No private test key, actual account token or user account data is included.

Live credential validation on macOS must use a stable signed application identity. Rebuilding an unsigned test executable changes its Keychain identity and can trigger repeated permission prompts; synthetic tests use the in-memory credential store. Never weaken Keychain access controls for a test.
