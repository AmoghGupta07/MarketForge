-- Each instrument gets a fixed starting price, used to seed tick 0
-- of every new save's own price history.
ALTER TABLE instruments ADD COLUMN base_price DOUBLE PRECISION;
UPDATE instruments i
SET base_price = COALESCE(
    (SELECT price FROM price_history ph WHERE ph.instrument_id = i.id ORDER BY ph.tick ASC LIMIT 1),
    100.0
);
ALTER TABLE instruments ALTER COLUMN base_price SET NOT NULL;

-- price_history moves from one shared timeline to one independent
-- timeline per save - each save is its own economy/playthrough, same
-- as portfolio and cash already are. Tick numbers from the old shared
-- model aren't meaningful once ticks are per-save-relative (that
-- mismatch is exactly what caused the duplicate-key crash), so this
-- clears existing history; each save reseeds its own tick 0 from
-- instruments.base_price and rebuilds history from there.
TRUNCATE price_history;
ALTER TABLE price_history ADD COLUMN player_id INTEGER NOT NULL REFERENCES players(id);
ALTER TABLE price_history DROP CONSTRAINT price_history_pkey;
ALTER TABLE price_history ADD PRIMARY KEY (player_id, instrument_id, tick);
