-- Missing Index Scenario: Workload
--
-- Queries that work on existing source columns.
-- Region-specific queries (the ones that regress) are injected via the
-- POST /events/queries/batch API since the region column only exists
-- after migration (applied to shadow only).

-- Query 1: Orders by status (has index)
SELECT id, order_number, status, created_at
FROM orders
WHERE status = 'pending'
ORDER BY created_at DESC
LIMIT 50;

-- Query 2: User's orders (has index)
SELECT id, order_number, status, total_amount
FROM orders
WHERE user_id = 42
ORDER BY created_at DESC;
