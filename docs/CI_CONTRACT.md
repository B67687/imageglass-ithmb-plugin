# CI Contract (pointer)

Canonical contract lives in the Codec repo: `docs/CI_CONTRACT.md`
(single place for all three repos' exact CI commands + env).

Plugin summary: `ci.yml` on push/PR to main (+ tags) — build (3-OS matrix,
symbol verify, ABI smoke, package) → clippy + nextest 0.9.143 → deny →
gitleaks → machete; release job is tag-only, draft, current-version notes only.
Local mirror: `scripts/check-local.sh`.
