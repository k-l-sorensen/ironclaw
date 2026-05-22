# `ironclaw_oauth`

Provider-agnostic OAuth substrate for Reborn native extensions.

## Ownership

- Own provider registration, OAuth state/PKCE, brokered/direct token exchange, token persistence row shape, refresh serialization, and callback routing.
- Do not register concrete providers here. Provider crates implement `OAuthProvider` and register with `ProviderRegistry`.
- Do not wire product/UI/run-loop consumers here. Composition and later workstreams mount routers and connect resume signals.

## Guardrails

- Route token and broker HTTP through the Reborn network boundary.
- Preserve the legacy row layout: `{credential}`, `{credential}_refresh_token`, `{credential}_scopes`, `{credential}_expiry`.
- Brokered mode must not forward provider client secrets.
- Keep secrets redacted; never log or serialize token values.
- Tests should drive `OAuthFlow`/`OAuthRuntime`, not just helpers.
