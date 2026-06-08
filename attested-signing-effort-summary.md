# Attested-Signing Substrate — Status

Landed a full substrate for **human-in-the-loop gated blockchain signing** in Reborn: the agent can get a high-value transaction signed only behind an explicit approval gate, with what-you-see-is-what-you-sign (WYSIWYS) binding and one-shot anti-replay throughout. Multi-chain (EVM / Solana / NEAR) and a dual trust model:

- **External wallet** — WalletConnect v2, browser-injected, NEAR redirect; preferred, gives wallet-side WYSIWYS.
- **Custodial** — keys behind `ironclaw_secrets`, WebAuthn presence, KMS ship-gate for mainnet.

Deliberately openssl-free (rustls / RustCrypto).

## Shape

A stacked PR set:

> trait → canonical hash/binding → sealed grant + idempotency ledger → WebAuthn + audit → turns gate → custodial chain-signing → the 3 external providers → runtime resume/continuation → reborn webui ingress → durable PG/libSQL stores → provider config → the `request_signature` raise.

Stack map + bottom-up merge order: [#3960](https://github.com/nearai/ironclaw/pull/3960)

## Hardening

Every PR got a deep, line-by-line security review (Codex-assisted) and the findings are fixed — it caught real issues before anything touched keys:

- approve-A / sign-B byte drift
- a KMS gate that didn't actually gate
- fail-open proof resolution
- forgeable "verified-proof" tokens
- panic-on-attacker-input

A **whole-stack coherence review** then confirmed the seams compose end-to-end and the load-bearing invariants hold together: one-shot grant, exact-bytes binding, deterministic resume (no LLM re-entry), fail-closed everywhere, tenant isolation.

## Multi-tenant

Isolation is enforced down to the grant key and the key-encryption AAD — cross-tenant claim / decrypt / resolve all fail closed — and is now regression-tested ([#4054](https://github.com/nearai/ironclaw/pull/4054)).

The operational gaps (per-tenant config, key/credential lifecycle) are designed and tracked in [#4051](https://github.com/nearai/ironclaw/issues/4051). Two pieces already landed:

- External-wallet trust-registration unblocker — [#4055](https://github.com/nearai/ironclaw/pull/4055)
- KMS curve-capability fail-closed guard — [#4058](https://github.com/nearai/ironclaw/pull/4058)

## Spin-off: tool-execution audit funnel

The review surfaced that interactive tool execution bypassed the audited `ToolDispatcher` funnel (no `ActionRecord`, no channel filter). We:

- Filed the finding — [#4017](https://github.com/nearai/ironclaw/issues/4017)
- Designed a durable fix — [#4019](https://github.com/nearai/ironclaw/issues/4019)
- Shipped it: a CI ratchet that blocks new bypasses + migrated chat / scheduler / routines / bridge onto the audited path
- Plus an HTTP-error run-abort regression fix and the macOS-keychain-prompt-during-tests fix — [#4027](https://github.com/nearai/ironclaw/pull/4027)

## Where it stands

- ~24 PRs open, green where CI has run, reviews requested from **@serrrfirat** + **@henrypark133**.
- **Next:** the bottom-up merge / rebase cascade — that's what produces the integrated, fixed artifact (the un-merged tip still carries pre-fix per-PR code).
- Plus the production multi-tenant runtime (gap D, reborn-wide).

Happy to walk anyone through the stack.
