CREATE TABLE players (
    id SERIAL PRIMARY KEY,
    game_mode TEXT NOT NULL,
    cash DOUBLE PRECISION NOT NULL,
    salary DOUBLE PRECISION NOT NULL DEFAULT 0,
    expenses DOUBLE PRECISION NOT NULL DEFAULT 0,
    current_tick BIGINT NOT NULL DEFAULT 0
);

CREATE TABLE instruments (
    id SERIAL PRIMARY KEY,
    symbol TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    class TEXT NOT NULL,
    drift DOUBLE PRECISION NOT NULL,
    volatility DOUBLE PRECISION NOT NULL
);

CREATE TABLE price_history (
    instrument_id INTEGER NOT NULL REFERENCES instruments(id),
    tick BIGINT NOT NULL,
    price DOUBLE PRECISION NOT NULL,
    PRIMARY KEY (instrument_id, tick)
);

CREATE TABLE positions (
    player_id INTEGER NOT NULL REFERENCES players(id),
    instrument_id INTEGER NOT NULL REFERENCES instruments(id),
    quantity DOUBLE PRECISION NOT NULL,
    avg_cost DOUBLE PRECISION NOT NULL,
    PRIMARY KEY (player_id, instrument_id)
);

-- Single-row table tracking simulation time, used for launch catch-up.
CREATE TABLE sim_state (
    id INTEGER PRIMARY KEY DEFAULT 1,
    current_tick BIGINT NOT NULL DEFAULT 0,
    last_tick_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (id = 1)
);

INSERT INTO sim_state (id, current_tick, last_tick_at) VALUES (1, 0, now());
