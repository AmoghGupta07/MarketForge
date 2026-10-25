-- Profit or loss a sale realized against the average cost, before tax.
ALTER TABLE trades ADD COLUMN realized_pnl DOUBLE PRECISION NOT NULL DEFAULT 0;
