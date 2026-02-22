-- Index Drop Scenario: Migration
--
-- Someone ran a query to find "unused" indexes and decided to clean up.
-- This index hasn't been used in the pg_stat_user_indexes view...
-- but that's because it was reset after the last database restart.

-- "This index hasn't been scanned in weeks, let's drop it"
DROP INDEX idx_orders_status_created;

-- The reasoning seemed sound:
-- - pg_stat_user_indexes showed idx_scans = 0
-- - The index was 45MB, seemed wasteful
-- - status and created_at have their own indexes, right?

-- What they missed:
-- - Stats were reset after a recent failover
-- - This composite index serves a very common query pattern
-- - The individual indexes can't serve (status, created_at) efficiently
