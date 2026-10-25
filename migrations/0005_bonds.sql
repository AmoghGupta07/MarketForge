-- Bonds are a new asset class with fundamentally different pricing
-- (interest-rate driven, not GBM), so instruments need fields the
-- GBM-priced classes don't use, defaulting to 0 (unused) for those.
ALTER TABLE instruments ADD COLUMN face_value DOUBLE PRECISION NOT NULL DEFAULT 0;
ALTER TABLE instruments ADD COLUMN coupon_rate DOUBLE PRECISION NOT NULL DEFAULT 0;
ALTER TABLE instruments ADD COLUMN duration DOUBLE PRECISION NOT NULL DEFAULT 0;

-- Per-save prevailing interest rate, which every bond's price and
-- coupon income reacts to. Defaults to sim-core's LONG_RUN_INTEREST_RATE.
ALTER TABLE players ADD COLUMN interest_rate DOUBLE PRECISION NOT NULL DEFAULT 0.04;
