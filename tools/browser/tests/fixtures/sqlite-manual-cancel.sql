-- Browser UI regression: execute each numbered section separately.
-- 1. Establish committed baseline and an uncommitted edit (Run all).
CREATE TABLE cancel_probe (value INTEGER);
INSERT INTO cancel_probe VALUES (7);
BEGIN;
UPDATE cancel_probe SET value = 8;

-- 2. Run this statement, then click Cancel before the two-second deadline.
-- Elapsed time must be materially below 2000ms to distinguish manual cancel.
WITH RECURSIVE n(x) AS (
  VALUES(1) UNION ALL SELECT x + 1 FROM n WHERE x < 1000000000
)
UPDATE cancel_probe SET value = (SELECT sum(x) FROM n);

-- 3. Expect 7, no open transaction indicator, and a usable connection.
SELECT value FROM cancel_probe;
