-- New instrument attributes: income streams (rent), rate sensitivity
-- (property), and mutual fund expense ratios. All default to 0, which
-- means "unused" for the existing classes.
ALTER TABLE instruments ADD COLUMN income_yield DOUBLE PRECISION NOT NULL DEFAULT 0;
ALTER TABLE instruments ADD COLUMN rate_sensitivity DOUBLE PRECISION NOT NULL DEFAULT 0;
ALTER TABLE instruments ADD COLUMN expense_ratio DOUBLE PRECISION NOT NULL DEFAULT 0;

-- Mutual fund baskets: a fund's NAV is derived each tick from the
-- weighted returns of these constituents.
CREATE TABLE fund_constituents (
    fund_id INTEGER NOT NULL REFERENCES instruments(id),
    constituent_id INTEGER NOT NULL REFERENCES instruments(id),
    weight DOUBLE PRECISION NOT NULL,
    PRIMARY KEY (fund_id, constituent_id),
    CHECK (fund_id <> constituent_id)
);

-- Resting limit orders. quantity is the unfilled remainder. The id is
-- allocated by sim-core (per save), not by a sequence.
CREATE TABLE orders (
    player_id INTEGER NOT NULL REFERENCES players(id),
    id BIGINT NOT NULL,
    instrument_id INTEGER NOT NULL REFERENCES instruments(id),
    is_buy BOOLEAN NOT NULL,
    quantity DOUBLE PRECISION NOT NULL,
    limit_price DOUBLE PRECISION NOT NULL,
    placed_tick BIGINT NOT NULL,
    PRIMARY KEY (player_id, id)
);

-- Execution history: one row per fill (market, immediate limit, or
-- resting limit filled during batch clearing).
CREATE TABLE trades (
    id BIGSERIAL PRIMARY KEY,
    player_id INTEGER NOT NULL REFERENCES players(id),
    instrument_id INTEGER NOT NULL REFERENCES instruments(id),
    is_buy BOOLEAN NOT NULL,
    quantity DOUBLE PRECISION NOT NULL,
    price DOUBLE PRECISION NOT NULL,
    tick BIGINT NOT NULL,
    kind TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX trades_player_idx ON trades (player_id, id DESC);
