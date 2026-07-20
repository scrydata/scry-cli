# test-migration DATABASE_URL fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `scry ci test-migration` (and `scry ci tunnel <cmd>`) hand the spawned migration command a `DATABASE_URL` that connects cleanly through the opened tunnel — `127.0.0.1` (not `localhost`) with the shadow's real credentials — so a psql-based migration exits 0 with no interactive password prompt.

**Architecture:** The bug is a hardcoded `postgres://localhost:<port>/postgres` string in `tunnel()` (ci.rs:487) and `test_migration()` (ci.rs:671): no credentials, and `localhost` resolves to `::1` first (the tunnel binds `127.0.0.1` only) → refused IPv6 attempt then passwordless IPv4 failure. Fix: fetch the same `GET /api/v1/shadows/{id}/connection` response `scry ci connect` uses (which always carries `username`/`password`/`database`), then build the URL against the *local tunnel* endpoint `127.0.0.1:<local_port>` while preserving those credentials. A new `ConnectionResponse::tunnel_connection_string(local_port)` method is the unit-testable seam.

**Tech Stack:** Rust, tokio, existing `ApiClient`/`ConnectionResponse` in `scry-cli`. Live receipt uses the demo docker-compose stack in `scry-platform`.

## Global Constraints

- Repo: `scrydata/scry-cli` for the code fix; `scrydata/scry-platform` for the receipt. Issue #82 is tracked in `scrydata/scry-platform`.
- Lints: workspace denies `unwrap_used`/`expect_used`/`panic` in non-test code; tests may use `unwrap()`.
- No new crate dependencies (no `url` crate available); build the URL with `format!`, matching the existing `connection_string()` style (no percent-encoding — consistent with current behavior).
- Do not change the interactive (no-command) tunnel path's protocol; only the spawned-command `DATABASE_URL` and user-facing bind hint.

---

### Task 1: `tunnel_connection_string` method + wire into test_migration and tunnel

**Files:**
- Modify: `src/commands/ci.rs` — add method on `ConnectionResponse` (near :208-223), replace hardcoded URLs at :487 and :671, fetch `ConnectionResponse` in both call sites, update the "Tunnel open at" hint (:457).
- Test: `src/commands/ci.rs` `#[cfg(test)] mod tests` (near :1361) — unit tests for the new method.

**Interfaces:**
- Consumes: existing `ConnectionResponse { host, port, database, username, password: Option<String>, connection_url: Option<String> }` and `ApiClient::get`.
- Produces: `fn tunnel_connection_string(&self, local_port: u16) -> String`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn tunnel_connection_string_uses_loopback_and_credentials() {
    let json = r#"{"host":"shadow-host","port":5433,"database":"appdb","username":"scry",
        "password":"s3cret","connection_url":"postgres://scry:s3cret@shadow-host:5433/appdb"}"#;
    let resp: ConnectionResponse = serde_json::from_str(json).unwrap();
    // Points at the LOCAL tunnel bind (127.0.0.1:<local_port>), NOT the shadow's real host/port,
    // but keeps the shadow's user/password/database.
    assert_eq!(
        resp.tunnel_connection_string(15432),
        "postgres://scry:s3cret@127.0.0.1:15432/appdb"
    );
}

#[test]
fn tunnel_connection_string_without_password_omits_credentials_section() {
    let json = r#"{"host":"h","port":5433,"database":"appdb","username":"scry"}"#;
    let resp: ConnectionResponse = serde_json::from_str(json).unwrap();
    assert_eq!(
        resp.tunnel_connection_string(15500),
        "postgres://scry@127.0.0.1:15500/appdb"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p scry-cli tunnel_connection_string`
Expected: FAIL — `no method named tunnel_connection_string`.

- [ ] **Step 3: Implement the method**

Add inside `impl ConnectionResponse` (after `connection_string`):

```rust
/// Connection URL for a migration command running against a LOCAL tunnel
/// bound on `127.0.0.1:<local_port>`. Keeps the shadow's credentials
/// (username/password/database) but replaces host/port with the tunnel's
/// actual bind — `localhost` is avoided because it resolves to `::1` first
/// and the tunnel listens on `127.0.0.1` only.
fn tunnel_connection_string(&self, local_port: u16) -> String {
    match &self.password {
        Some(pw) => format!(
            "postgres://{}:{}@127.0.0.1:{}/{}",
            self.username, pw, local_port, self.database
        ),
        None => format!(
            "postgres://{}@127.0.0.1:{}/{}",
            self.username, local_port, self.database
        ),
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p scry-cli tunnel_connection_string`
Expected: PASS (2 tests).

- [ ] **Step 5: Wire into `test_migration` (replace :671)**

Replace `let connection_url = format!("postgres://localhost:{}/postgres", port);` with a fetch of the connection info (unlock on error, matching the surrounding pattern) and build via the new method:

```rust
let conn_path = format!("/api/v1/shadows/{}/connection", shadow_id);
let conn_info: ConnectionResponse = match client.get(&conn_path).await {
    Ok(c) => c,
    Err(e) => {
        let _: Result<serde_json::Value, _> = client.delete(&unlock_path).await;
        return Err(e);
    }
};
let connection_url = conn_info.tunnel_connection_string(port);
```

- [ ] **Step 6: Wire into `tunnel` command (replace :487)**

In the `else` (has-command) branch, before building `connection_url`, fetch and build:

```rust
let conn_path = format!("/api/v1/shadows/{}/connection", shadow_id);
let conn_info: ConnectionResponse = client.get(&conn_path).await?;
let connection_url = conn_info.tunnel_connection_string(port);
```

Also update the bind hint at :457 from `localhost` to `127.0.0.1`:

```rust
eprintln!("Tunnel open at 127.0.0.1:{}", port);
```

- [ ] **Step 7: Build + full test suite**

Run: `cargo build && cargo test -p scry-cli`
Expected: PASS, no new warnings; clippy clean (`cargo clippy --all-targets -- -D warnings`).

- [ ] **Step 8: Commit**

```bash
git add src/commands/ci.rs docs/superpowers/plans/2026-07-19-test-migration-database-url.md
git commit -m "fix(ci): build tunnel DATABASE_URL with 127.0.0.1 + shadow creds (#82)"
```

---

### Task 2: Live psql happy-path receipt (scry-platform)

Worktree: `/home/gmcquillan/src/scry-platform/.claude/worktrees/sdlc+82-test-migration-receipt` (branch `sdlc/82-test-migration-receipt`, from origin/main). Uses the Task 1 binary built at `/home/gmcquillan/src/scry-cli.worktrees/sdlc-82-test-migration-database-url/target/release/scry`.

**Files:**
- Modify: `docs/receipts/t14/test-migration-happy-path.txt` — replace the TCP-reachability workaround with a genuine psql migration exiting 0, one run as a host process and one in a `--network host` container, verbatim.

- [ ] **Step 1:** Build the fixed CLI release binary in the scry-cli worktree (`cargo build --release`).
- [ ] **Step 2:** Bring up the demo stack (`cd demo && docker-compose up -d`); wait for a Ready shadow (`scry ci wait-ready ...`); ensure a checkpoint exists (`scry ci checkpoint ...`).
- [ ] **Step 3:** Run `scry ci test-migration <shadow> --skip-replay -- psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f <safe migration>` as a HOST process; capture verbatim output; confirm exit 0 and no `Password for user` prompt.
- [ ] **Step 4:** Run the same via a `--network host` container wrapping psql (`docker run --rm --network host -e DATABASE_URL postgres:15 sh -c 'psql "$DATABASE_URL" ...'`); capture verbatim; confirm exit 0.
- [ ] **Step 5:** Rewrite `docs/receipts/t14/test-migration-happy-path.txt` with both verbatim runs and a note that the prior exit-12 was a genuine product papercut (missing creds + `localhost`/`::1`) now fixed in scry-cli (#82); tear down the demo stack.
- [ ] **Step 6: Commit** in the platform worktree:

```bash
git add docs/receipts/t14/test-migration-happy-path.txt
git commit -m "docs(tier-0): real psql test-migration happy-path receipt, exit 0 (#82)"
```

---

## Self-Review

- **Spec coverage:** AC1 → Task 1 Steps 3/5/6 (127.0.0.1 + creds in both call sites). AC2 → Task 2 (live psql exit 0, host + `--network host`, no prompt). AC3 → Task 1 Steps 1-4 (unit tests on the URL builder). Out-of-scope (#79 CI wiring, #80 harness) untouched.
- **Placeholder scan:** none — all code shown.
- **Type consistency:** `tunnel_connection_string(&self, local_port: u16) -> String` used identically in Steps 5/6; `ConnectionResponse` fields match the existing struct (:196-206).
