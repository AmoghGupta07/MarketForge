-- Named saves: each player row is now a distinct save slot.
ALTER TABLE players ADD COLUMN save_name TEXT;
UPDATE players SET save_name = 'save_' || id WHERE save_name IS NULL;
ALTER TABLE players ALTER COLUMN save_name SET NOT NULL;
CREATE UNIQUE INDEX players_save_name_idx ON players (save_name);

-- Tick catch-up timing moves from the old global sim_state singleton
-- to per-player, since each save needs to catch up independently
-- based on when THAT save was last touched, not any save.
ALTER TABLE players ADD COLUMN last_tick_at TIMESTAMPTZ NOT NULL DEFAULT now();
