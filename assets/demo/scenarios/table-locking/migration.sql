-- Table Locking Scenario: Migration
--
-- A migration that looks innocent but locks the entire table.
-- This demonstrates what happens when you run a slow migration
-- while production queries are trying to access the table.

-- Approach 1: Large UPDATE (always locks rows progressively, blocks reads with FOR UPDATE)
-- This simulates a data backfill that takes a while
UPDATE orders
SET shipping_address = CONCAT(shipping_address, ', Updated: ', NOW()::text)
WHERE status IN ('delivered', 'shipped')
  AND created_at < NOW() - INTERVAL '30 days';

-- Note: In a real scenario, this UPDATE would touch ~60K rows and take
-- 30-60 seconds, during which queries needing these rows would wait.

-- Alternative approaches that also cause locking:
--
-- Approach 2: Adding a column with a volatile default (PG < 11)
-- ALTER TABLE orders ADD COLUMN processed_at TIMESTAMP DEFAULT NOW();
--
-- Approach 3: Adding a constraint that requires a full table scan
-- ALTER TABLE orders ADD CONSTRAINT chk_positive CHECK (total_amount >= 0);
--
-- Approach 4: Creating a regular (non-concurrent) index
-- CREATE INDEX idx_orders_slow ON orders(created_at, status, total_amount);
