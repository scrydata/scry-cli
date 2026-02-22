-- Scry Demo: Seed Data
--
-- Generates realistic e-commerce data:
-- - 2.5K users
-- - 250 products
-- - 25K orders (weighted toward recent dates)
-- - ~75K order items

-- Seed for reproducibility
SELECT setseed(0.42);

-- Insert 2,500 users
INSERT INTO users (email, name, created_at)
SELECT
    'user' || i || '@example.com',
    'User ' || i,
    NOW() - (random() * INTERVAL '365 days')
FROM generate_series(1, 2500) AS i;

-- Insert 250 products across categories
INSERT INTO products (sku, name, description, price, stock_quantity, category, is_active)
SELECT
    'SKU-' || LPAD(i::text, 5, '0'),
    CASE (i % 10)
        WHEN 0 THEN 'Laptop'
        WHEN 1 THEN 'Keyboard'
        WHEN 2 THEN 'Monitor'
        WHEN 3 THEN 'Mouse'
        WHEN 4 THEN 'Headphones'
        WHEN 5 THEN 'Webcam'
        WHEN 6 THEN 'USB Hub'
        WHEN 7 THEN 'Desk Lamp'
        WHEN 8 THEN 'Chair'
        WHEN 9 THEN 'Desk'
    END || ' Model ' || i,
    'High quality product with excellent features. Model number ' || i,
    ROUND((random() * 900 + 10)::numeric, 2),
    FLOOR(random() * 500)::integer,
    CASE (i % 5)
        WHEN 0 THEN 'Electronics'
        WHEN 1 THEN 'Accessories'
        WHEN 2 THEN 'Peripherals'
        WHEN 3 THEN 'Furniture'
        WHEN 4 THEN 'Lighting'
    END,
    random() > 0.1  -- 90% active
FROM generate_series(1, 250) AS i;

-- Insert 25,000 orders with realistic distribution
-- More recent orders are more common (exponential decay)
INSERT INTO orders (order_number, user_id, status, total_amount, shipping_address, created_at)
SELECT
    'ORD-' || TO_CHAR(NOW() - (pow(random(), 2) * INTERVAL '180 days'), 'YYYYMMDD') || '-' || LPAD(i::text, 6, '0'),
    FLOOR(random() * 2500 + 1)::integer,
    CASE
        WHEN random() < 0.05 THEN 'pending'
        WHEN random() < 0.15 THEN 'confirmed'
        WHEN random() < 0.25 THEN 'processing'
        WHEN random() < 0.40 THEN 'shipped'
        WHEN random() < 0.90 THEN 'delivered'
        WHEN random() < 0.95 THEN 'cancelled'
        ELSE 'refunded'
    END,
    0,  -- Will be updated after order_items
    i || ' Demo Street, City ' || (i % 100) || ', State ' || (i % 50),
    NOW() - (pow(random(), 2) * INTERVAL '180 days')  -- Exponential: more recent orders
FROM generate_series(1, 25000) AS i;

-- Insert order items (average 3 items per order = ~75K items)
INSERT INTO order_items (order_id, product_id, quantity, unit_price)
SELECT
    o.id,
    FLOOR(random() * 250 + 1)::integer,
    FLOOR(random() * 4 + 1)::integer,
    ROUND((random() * 500 + 10)::numeric, 2)
FROM orders o
CROSS JOIN generate_series(1, 3) AS item_num
WHERE random() < 0.9 + (item_num * 0.03);  -- Slight variation in items per order

-- Update order totals based on items
UPDATE orders o
SET total_amount = (
    SELECT COALESCE(SUM(quantity * unit_price), 0)
    FROM order_items oi
    WHERE oi.order_id = o.id
);

-- Add some audit log entries
INSERT INTO audit_log (table_name, record_id, action, new_data, created_at)
SELECT
    'orders',
    FLOOR(random() * 25000 + 1)::integer,
    CASE WHEN random() < 0.7 THEN 'UPDATE' ELSE 'INSERT' END,
    jsonb_build_object('status', 'updated', 'timestamp', NOW()),
    NOW() - (random() * INTERVAL '30 days')
FROM generate_series(1, 1200);

-- Analyze tables for query planner
ANALYZE users;
ANALYZE products;
ANALYZE orders;
ANALYZE order_items;
ANALYZE audit_log;
