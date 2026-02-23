# scry-cli

Command-line interface for [Scry](https://www.scrydata.com) — shadow database testing for PostgreSQL.

## What it does

Scry CLI manages shadow databases that mirror your production PostgreSQL instances. You can replay captured production traffic against shadows to detect query regressions, then integrate the whole workflow into CI to catch migration issues before they hit prod.

## Install

```bash
cargo install scry-cli
```

Or build from source:

```bash
git clone https://github.com/scrydata/scry-cli.git
cd scry-cli
cargo build --release
# binary is at target/release/scry
```

## Quick start

```bash
# Configure API endpoint and token
scry config set api_url https://api.scrydata.com
scry config set token <your-token>

# Register a source database, create a shadow, and replay traffic
scry source register myproject/prod
scry shadow create --source myproject/prod --database-name test-shadow
scry replay start --shadow <shadow-id>
scry analysis report <replay-id>
```

Or use the integrated CI command to test a migration end-to-end:

```bash
scry ci test-migration myproject/ci-shadow -- alembic upgrade head
```

## Commands

| Command | Description |
|---|---|
| `config` | Set and show CLI configuration |
| `health` | Check API service health |
| `source` | Register and manage source databases |
| `shadow` | Create, inspect, verify, and destroy shadow instances |
| `checkpoints` | Snapshot and restore shadow database state |
| `replay` | Start, monitor, and stop traffic replays |
| `analysis` | Generate regression reports (text, JSON, JUnit) |
| `ci` | CI/CD pipeline helpers — test-migration, replay, tunnel, detect-migrations |
| `demo` | Interactive demo walkthrough with bundled scenarios |
| `version` | Show version info |

Run `scry <command> --help` for details on any command.

## Configuration

**Config file:** `~/.scry/config.toml`

```toml
api_url = "https://api.scrydata.com"
token = "sk-..."

[profiles.staging]
api_url = "https://staging-api.scrydata.com"
token = "sk-staging-..."
```

**Environment variables** (override config file):

- `SCRY_API_URL` — API endpoint
- `SCRY_API_TOKEN` — authentication token

**Repo-level config:** `.scry.toml` in your repository root configures migration detection paths, CI defaults, and replay settings.

**Global flags:** `--profile <name>`, `--api-url`, `--token`, `--json`, `--quiet`

## Related repos

| Repo | Description |
|---|---|
| [scry-proxy](https://github.com/scrydata/scry-proxy) | PostgreSQL wire-protocol proxy that captures query traffic |
| [scry-protocol](https://github.com/scrydata/scry-protocol) | Shared protocol definitions and event types |

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT License](LICENSE), at your option.
