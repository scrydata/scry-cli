-- Table Locking Scenario: The Fix
--
-- Instead of one large UPDATE, batch it to avoid long-held locks.

-- Option 1: Batch UPDATE with smaller chunks
DO $$
DECLARE
    batch_size INT := 1000;
    rows_updated INT;
BEGIN
    LOOP
        UPDATE orders
        SET shipping_address = CONCAT(shipping_address, ', Updated: ', NOW()::text)
        WHERE id IN (
            SELECT id FROM orders
            WHERE status IN ('delivered', 'shipped')
              AND created_at < NOW() - INTERVAL '30 days'
              AND shipping_address NOT LIKE '%Updated:%'
            LIMIT batch_size
            FOR UPDATE SKIP LOCKED
        );

        GET DIAGNOSTICS rows_updated = ROW_COUNT;
        EXIT WHEN rows_updated = 0;

        -- Brief pause to let other transactions through
        PERFORM pg_sleep(0.1);
        COMMIT;
    END LOOP;
END $$;

-- Option 2: For adding indexes, use CONCURRENTLY
-- CREATE INDEX CONCURRENTLY idx_orders_new ON orders(...);

-- Option 3: For adding constraints, use NOT VALID + VALIDATE
-- ALTER TABLE orders ADD CONSTRAINT chk_positive CHECK (total_amount >= 0) NOT VALID;
-- ALTER TABLE orders VALIDATE CONSTRAINT chk_positive;  -- runs in background
