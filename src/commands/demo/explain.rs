//! Architectural explanation text for `--explain` mode.
//!
//! Each function returns (title, body) for display between walkthrough steps.

pub fn services() -> (&'static str, &'static str) {
    (
        "The Demo Stack",
        "\
The demo runs 5 services locally:

  source-postgres  Your \"production\" database (100K orders)
  nats             JetStream message broker for event journals
  scry-platform    The replay engine (receives events, manages shadows)
  scry-backfill    CDC replication (source -> shadow via logical replication)
  scry-proxy       Query capture (transparent proxy between app and DB)

In production, scry-proxy sits in front of your existing database.
Your app connects to the proxy instead of directly to PostgreSQL.
The proxy captures every query and forwards it to scry-platform.",
    )
}

pub fn shadow_sync() -> (&'static str, &'static str) {
    (
        "Shadow Databases & Journals",
        "\
A shadow database is an isolated PostgreSQL copy for testing migrations.

Key concept: scry-platform uses JOURNALS, not queues.
Unlike a traditional message queue that deletes messages after consumption,
a journal retains events by age/size — like a Kafka topic or a WAL.

Why this matters:
  - Each shadow tracks its own position in the journal
  - You can reset a shadow to an earlier position and replay from there
  - Multiple shadows can consume the same events independently
  - Repeatable A/B testing becomes possible

The shadow state machine:
  Creating -> Syncing -> Ready <-> Replaying
      |          |         |          |
    Failed    Failed   Destroying   Failed",
    )
}

pub fn query_capture() -> (&'static str, &'static str) {
    (
        "Query Capture via scry-proxy",
        "\
scry-proxy is a transparent PostgreSQL wire-protocol proxy.
It intercepts the Postgres frontend/backend protocol messages:

  App  --(Parse/Bind/Execute)-->  scry-proxy  -->  PostgreSQL
                                      |
                                 captures query
                                 text + bind params
                                      |
                                      v
                                scry-platform
                                (query journal)

What gets captured:
  - SQL text from Parse messages
  - Bind parameters (values your app sends)
  - Execution timing from the source database

The captured queries are stored in a NATS JetStream journal,
retained for 7 days by default. When you run a replay, the
replay engine reads from this journal.",
    )
}

pub fn migration_apply() -> (&'static str, &'static str) {
    (
        "Applying Migrations to Shadows",
        "\
The migration is applied ONLY to the shadow — the source database
is never modified. This is the core safety guarantee.

In the demo, we apply SQL directly to the shadow container.
In CI/CD, you would use the tunnel:

  scry ci tunnel prod-db/ci-main -- alembic upgrade head

The tunnel creates an HTTP/2 bidirectional stream that carries
PostgreSQL wire protocol. Your migration tool connects to a local
port, and scry-platform forwards the traffic to the shadow.

After the migration, the shadow has a different schema than the
source. Now we can replay captured queries and see what changed.",
    )
}

pub fn replay() -> (&'static str, &'static str) {
    (
        "Query Replay & Comparison",
        "\
The replay engine reads queries from the journal and executes
each one against the shadow database (with the migration applied).

For each query, it records:
  - Execution time on the shadow
  - Query plan (via EXPLAIN ANALYZE)
  - Any errors

Then it compares against the original timing from the source:
  - p50, p95, p99 latency comparisons
  - Query plan changes (Seq Scan vs Index Scan)
  - New errors that didn't exist before

Replay modes:
  fast_forward  Execute as fast as possible (default for CI)
  timed         Replay at original speed (1x) or scaled (2x, 0.5x)

A query is flagged as a regression when the shadow timing
significantly exceeds the source baseline.",
    )
}

pub fn results() -> (&'static str, &'static str) {
    (
        "Regression Detection",
        "\
The analysis engine uses HDR histograms to compare latency
distributions, not just averages. This catches tail latency
regressions that mean-based comparisons miss.

Classification:
  Regression   Shadow p50 > 2x source p50, or p99 > 3x source p99
  Improvement  Shadow significantly faster than source
  No change    Within normal variance

For each regression, the engine identifies the likely cause
by comparing EXPLAIN ANALYZE plans:
  - Seq Scan replacing Index Scan = missing index
  - Lock waits = blocking migration
  - Timeout = table-level locks

In CI/CD, you can fail the pipeline on regressions:
  scry ci replay prod-db/ci-main --fail-on-regression",
    )
}

pub fn fix_and_verify() -> (&'static str, &'static str) {
    (
        "Iterating on Fixes",
        "\
The checkpoint system lets you iterate quickly:

  1. Create a checkpoint before the migration
  2. Apply migration, replay, find regressions
  3. Reset to the checkpoint
  4. Apply migration + fix, replay again
  5. Repeat until all regressions are resolved

In CI/CD:
  scry ci checkpoint prod-db/ci-main --name pre-migration
  scry ci test-migration prod-db/ci-main -- alembic upgrade head
  # If regressions detected:
  scry ci reset prod-db/ci-main --to pre-migration --wait
  # Apply fix and try again

The journal retains all captured queries, so every replay
uses the same workload — true A/B comparison.",
    )
}

pub fn wrapup() -> (&'static str, &'static str) {
    (
        "From Demo to Production",
        "\
In production, the components run as services:

  scry-proxy     Deploy alongside your app (sidecar or standalone)
  scry-backfill  One instance per source database
  scry-platform  Central service managing shadows and replays

The CI/CD integration uses scry-cli:
  - scry ci wait-ready     Block until shadow is synced
  - scry ci test-migration Full migration test in one command
  - scry ci replay         Run replay with regression gating

Exit codes are designed for CI:
  0 = success, 5 = regressions detected, 6 = drift exceeded

The shadow approach means you test against real data volumes
and real query patterns — not synthetic benchmarks.",
    )
}
