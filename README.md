# Pan-Africa Pay

A pan-African mobile money aggregator: collect and pay out across mobile
money networks and stablecoin rails from a single API.

## What It Does

- **Collect payments** - STK-push style prompts to customer phones
- **Pay out** - send money to customer phones or wallets
- **Cross-border settlement** - USDC-backed transfers via Kotani Pay
- **One integration, many rails** - your application talks to one API;
  Pan-Africa Pay handles the network plumbing underneath

## Security

- Idempotent by design: every mutating request carries an idempotency key
  so retries can never double-charge a customer
- Monetary values handled as integer minor units - no floating-point drift
- Atomic balance updates with overdraft protection at the database level
- Provider callbacks verified and replay-guarded before state changes
- Secrets managed via environment configuration, never committed
- Append-only audit log for every financial event
- Provider credentials isolated per environment (sandbox vs production)

## Speed

- Asynchronous request handling end-to-end
- Redis-backed idempotency lookups for fast retries
- Pooled database connections with health checking
- Provider OAuth tokens cached and reused until expiry
- Continuous integration verifies format, lints, tests, and offline
  SQL checks on every change

## Getting Started

```bash
# 1. Start infrastructure (PostgreSQL + Redis + mock provider)
docker compose up -d postgres redis

# 2. Copy environment template
cp .env.example .env

# 3. Apply migrations
cargo sqlx migrate run --source crates/storage/migrations

# 4. Build and test
cargo build --workspace
cargo test --workspace
```

## API Endpoints

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/mpesa/stk/push` | Initiate an M-Pesa STK push prompt |
| `GET` | `/mpesa/stk/query/{checkout_request_id}` | Poll STK push status |
| `POST` | `/kotani/customers` | Register a mobile money customer |
| `POST` | `/kotani/deposit` | Initiate fiat -> stablecoin deposit |
| `POST` | `/kotani/withdraw` | Initiate stablecoin -> fiat payout |
| `GET` | `/kotani/deposit/status/{reference_id}` | Poll deposit status |
| `GET` | `/kotani/withdraw/status/{reference_id}` | Poll withdrawal status |
| `POST` | `/webhooks/mpesa` | Daraja STK callback |
| `POST` | `/webhooks/kotani` | Signed Kotani transaction callback |
| `POST` | `/payments/payin` | Dual-rail payin: `KES` -> M-Pesa, `USDC` -> Kotani |
| `GET` | `/payments/{id}` | Fetch a payment |
| `GET` | `/payments?user_id=...` | List a user's payments |
| `POST` | `/users` | Create a platform user |

The payin endpoint performs automatic rail routing: `KES` amounts go
through M-Pesa STK push, `USDC` amounts through a Kotani Pay deposit
(with automatic customer registration on first use). Webhooks reconcile
payments to `COMPLETED`/`FAILED` and attach the provider callback
payload for audit. A reconciliation sweeper additionally polls provider
status endpoints every minute and settles payments stuck in
`PENDING`/`PROCESSING` whose callback was missed.

Every mutating endpoint accepts an optional `Idempotency-Key` header
(URL-safe, max 128 chars): retries with the same key and body replay
the original response; a different body with the same key returns `409
IDEMPOTENCY_CONFLICT`. Records live in Redis with a PostgreSQL fallback.

## Repository Layout

```
crates/
├── domain/    # Core types, errors, events, repository contracts
├── storage/   # Database adapters and schema migrations
├── mpesa/     # M-Pesa integration (Phase 2)
├── kotani/    # Kotani Pay integration (Phase 2)
└── api/       # HTTP API server (Phase 2)
```

## Development Workflow

- Feature work on `feat/*` branches merged into `develop` via PR
- `develop` is the integration branch; `main` carries tagged releases
- Conventional commit messages
- CI enforces format, lint, tests, and offline SQL verification

## Phase Roadmap

| Phase | Scope | Status |
|-------|-------|--------|
| 1 | Foundation: workspace, domain core, storage | Done |
| 2 | Provider integrations and HTTP API | Done |
| 3 | Dual-rail flows and reconciliation | Done |
