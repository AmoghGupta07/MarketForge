CREATE TABLE net_worth_history (
    player_id INTEGER NOT NULL REFERENCES players(id),
    tick BIGINT NOT NULL,
    net_worth DOUBLE PRECISION NOT NULL,
    PRIMARY KEY (player_id, tick)
);
