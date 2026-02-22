-- Missing Index Scenario: The Fix
--
-- Add the index that should have been in the original migration.
-- Using CONCURRENTLY to avoid locking the table in production.

CREATE INDEX CONCURRENTLY idx_orders_region ON orders(region);

-- For queries that filter by region AND status, a composite index is even better:
CREATE INDEX CONCURRENTLY idx_orders_region_status ON orders(region, status);
