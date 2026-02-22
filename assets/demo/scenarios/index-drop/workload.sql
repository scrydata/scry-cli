-- Index Drop Scenario: Workload
--
-- These queries rely on the idx_orders_status_created composite index.
-- After dropping it, they'll fall back to less efficient plans.

-- Query 1: Pending orders dashboard (runs every 30 seconds)
-- Before: 2ms (index scan on status, created_at)
-- After: 85ms (bitmap heap scan or seq scan)
SELECT id, order_number, user_id, total_amount, created_at
FROM orders
WHERE status = 'pending'
  AND created_at > NOW() - INTERVAL '24 hours'
ORDER BY created_at DESC;

-- Query 2: Processing queue (order fulfillment system)
-- Before: 3ms (index scan)
-- After: 120ms (seq scan with filter)
SELECT id, order_number, shipping_address
FROM orders
WHERE status = 'confirmed'
  AND created_at > NOW() - INTERVAL '7 days'
ORDER BY created_at ASC
LIMIT 100;

-- Query 3: Status transition report (hourly background job)
-- Before: 15ms (index scan + aggregate)
-- After: 250ms (seq scan + aggregate)
SELECT
    DATE_TRUNC('hour', created_at) as hour,
    status,
    COUNT(*) as order_count
FROM orders
WHERE created_at > NOW() - INTERVAL '24 hours'
GROUP BY DATE_TRUNC('hour', created_at), status
ORDER BY hour DESC, status;

-- Query 4: SLA monitoring (runs every minute)
-- Before: 4ms (index scan, very selective)
-- After: 95ms (seq scan)
SELECT COUNT(*) as stuck_orders
FROM orders
WHERE status = 'processing'
  AND created_at < NOW() - INTERVAL '2 hours';

-- Query 5: Customer support lookup
-- Before: 2ms
-- After: 60ms
SELECT o.*, u.email
FROM orders o
JOIN users u ON o.user_id = u.id
WHERE o.status = 'shipped'
  AND o.created_at > NOW() - INTERVAL '3 days'
  AND o.shipping_address ILIKE '%california%';

-- These queries run frequently:
-- - Query 1: 2/minute = 2,880/day
-- - Query 2: 10/minute = 14,400/day
-- - Query 4: 1/minute = 1,440/day
-- Total: ~18,720 queries/day relying on this "unused" index
