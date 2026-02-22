-- Table Locking Scenario: Workload
--
-- These queries run continuously in production.
-- During the migration, they'll timeout or experience long waits.

-- Query 1: Get order details (frequent - checkout flow)
-- Impact: Blocked when migration holds locks on orders table
SELECT o.id, o.order_number, o.status, o.total_amount, o.shipping_address
FROM orders o
WHERE o.id = 12345;

-- Query 2: Update order status (critical - order processing)
-- Impact: Blocked waiting for row locks held by UPDATE
UPDATE orders
SET status = 'shipped', updated_at = NOW()
WHERE id = 12345;

-- Query 3: List user's recent orders (common - account page)
-- Impact: May be blocked depending on isolation level
SELECT id, order_number, status, total_amount, created_at
FROM orders
WHERE user_id = 42
ORDER BY created_at DESC
LIMIT 10;

-- Query 4: Insert new order (critical - checkout)
-- Impact: Generally OK, but may wait on table-level locks
INSERT INTO orders (order_number, user_id, status, total_amount, shipping_address)
VALUES ('ORD-NEW-001', 42, 'pending', 99.99, '123 New Street');

-- Query 5: Dashboard aggregation (background job)
-- Impact: Long wait while migration runs
SELECT status, COUNT(*), SUM(total_amount)
FROM orders
WHERE created_at > NOW() - INTERVAL '24 hours'
GROUP BY status;

-- Query 6: Order search (customer support)
-- Impact: Timeout during migration window
SELECT o.id, o.order_number, u.email, o.status, o.created_at
FROM orders o
JOIN users u ON o.user_id = u.id
WHERE o.order_number LIKE 'ORD-2024%'
LIMIT 20;
