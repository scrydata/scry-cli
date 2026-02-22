-- Missing Index Scenario: Migration
--
-- Your teammate adds a "region" column for a new feature.
-- The migration works fine in staging. What could go wrong?

-- Add the region column
ALTER TABLE orders ADD COLUMN region VARCHAR(50);

-- Backfill existing orders with region data
-- (In reality, this might come from user addresses or IP geolocation)
UPDATE orders SET region = CASE
    WHEN (id % 5) = 0 THEN 'west'
    WHEN (id % 5) = 1 THEN 'east'
    WHEN (id % 5) = 2 THEN 'central'
    WHEN (id % 5) = 3 THEN 'south'
    ELSE 'north'
END;

-- NOTE: No index added!
-- The new feature will filter by region, but there's no index to support it.
-- This will cause queries to do sequential scans on the 100K row orders table.
