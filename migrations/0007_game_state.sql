-- The game layer (career, businesses, deals, NPC relationships, rivals,
-- active events, news, pending decisions, win/lose status) is stored as
-- one JSON document per save. It is only ever read and written whole by
-- the game engine, so a document is simpler than a dozen tables.
CREATE TABLE game_state (
    player_id INTEGER PRIMARY KEY REFERENCES players(id),
    state TEXT NOT NULL
);
