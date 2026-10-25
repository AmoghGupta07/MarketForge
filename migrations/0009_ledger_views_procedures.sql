-- =====================================================================
-- Transaction ledger, economy history, views, stored procedures, triggers
-- =====================================================================

-- ---- Tables ----------------------------------------------------------

-- Every cash movement: pay, taxes, bills, interest, trades, deals.
-- Append-only (see the immutability trigger below).
CREATE TABLE transactions (
    id BIGSERIAL PRIMARY KEY,
    player_id INTEGER NOT NULL REFERENCES players(id),
    tick BIGINT NOT NULL,
    month BIGINT GENERATED ALWAYS AS (tick / 30) STORED,
    category TEXT NOT NULL,
    description TEXT NOT NULL,
    amount DOUBLE PRECISION NOT NULL CHECK (amount <> 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX transactions_player_month_idx ON transactions (player_id, month);

-- One row per simulated month: the macro indicators.
CREATE TABLE economy_history (
    player_id INTEGER NOT NULL REFERENCES players(id),
    month BIGINT NOT NULL,
    tick BIGINT NOT NULL,
    cpi DOUBLE PRECISION NOT NULL,
    inflation DOUBLE PRECISION NOT NULL,
    gdp_growth DOUBLE PRECISION NOT NULL,
    unemployment DOUBLE PRECISION NOT NULL,
    policy_rate DOUBLE PRECISION NOT NULL,
    debt_to_gdp DOUBLE PRECISION NOT NULL,
    PRIMARY KEY (player_id, month)
);

-- Filled by the close_month() procedure.
CREATE TABLE monthly_summary (
    player_id INTEGER NOT NULL REFERENCES players(id),
    month BIGINT NOT NULL,
    income DOUBLE PRECISION NOT NULL,
    expenses DOUBLE PRECISION NOT NULL,
    taxes DOUBLE PRECISION NOT NULL,
    net DOUBLE PRECISION NOT NULL,
    closed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (player_id, month)
);

-- Maintained by a trigger on transactions.
CREATE TABLE category_totals (
    player_id INTEGER NOT NULL REFERENCES players(id),
    category TEXT NOT NULL,
    total DOUBLE PRECISION NOT NULL DEFAULT 0,
    n BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (player_id, category)
);

-- Maintained by a trigger on net_worth_history.
CREATE TABLE player_records (
    player_id INTEGER PRIMARY KEY REFERENCES players(id),
    peak_net_worth DOUBLE PRECISION NOT NULL,
    peak_tick BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE positions ADD CONSTRAINT positions_quantity_nonneg CHECK (quantity >= 0);

-- ---- Functions -------------------------------------------------------

-- Movements that swap one kind of asset for another rather than earn or
-- spend money. Excluded from income/expense reporting.
CREATE FUNCTION is_transfer(cat TEXT) RETURNS BOOLEAN
LANGUAGE sql IMMUTABLE AS $$
    SELECT cat IN ('trade', 'loan_draw', 'loan_principal', 'deposit', 'withdrawal', 'deal', 'business_invest')
$$;

-- ---- Views -----------------------------------------------------------

CREATE VIEW v_latest_prices AS
SELECT DISTINCT ON (ph.player_id, ph.instrument_id)
       ph.player_id, ph.instrument_id, i.symbol, i.class, ph.price, ph.tick
FROM price_history ph
JOIN instruments i ON i.id = ph.instrument_id
ORDER BY ph.player_id, ph.instrument_id, ph.tick DESC;

CREATE VIEW v_portfolio AS
SELECT p.player_id, lp.symbol, lp.class, p.quantity, p.avg_cost, lp.price,
       p.quantity * lp.price AS market_value,
       p.quantity * (lp.price - p.avg_cost) AS unrealized_pnl,
       CASE WHEN p.avg_cost > 0 THEN lp.price / p.avg_cost - 1 END AS return_pct
FROM positions p
JOIN v_latest_prices lp ON lp.player_id = p.player_id AND lp.instrument_id = p.instrument_id;

CREATE VIEW v_monthly_cashflow AS
SELECT player_id, month,
       COALESCE(SUM(amount) FILTER (WHERE amount > 0 AND NOT is_transfer(category)), 0) AS income,
       COALESCE(-SUM(amount) FILTER (WHERE amount < 0 AND NOT is_transfer(category) AND category <> 'tax'), 0) AS expenses,
       COALESCE(-SUM(amount) FILTER (WHERE category = 'tax'), 0) AS taxes,
       COALESCE(SUM(amount) FILTER (WHERE NOT is_transfer(category)), 0) AS net
FROM transactions
GROUP BY player_id, month;

CREATE VIEW v_net_worth_monthly AS
SELECT DISTINCT ON (player_id, tick / 30)
       player_id, tick / 30 AS month, tick, net_worth
FROM net_worth_history
ORDER BY player_id, tick / 30, tick DESC;

CREATE VIEW v_economy_latest AS
SELECT DISTINCT ON (player_id) *
FROM economy_history
ORDER BY player_id, month DESC;

CREATE VIEW v_balance_sheet AS
SELECT pl.id AS player_id, pl.save_name, pl.current_tick, pl.cash,
       COALESCE(pv.value, 0) AS investments,
       pl.cash + COALESCE(pv.value, 0) AS cash_and_investments
FROM players pl
LEFT JOIN (SELECT player_id, SUM(market_value) AS value FROM v_portfolio GROUP BY player_id) pv
       ON pv.player_id = pl.id;

-- ---- Stored procedures and functions ----------------------------------

-- Value of one save's holdings at its latest prices.
CREATE FUNCTION portfolio_value(p_player INTEGER) RETURNS DOUBLE PRECISION
LANGUAGE sql STABLE AS $$
    SELECT COALESCE(SUM(market_value), 0) FROM v_portfolio WHERE player_id = p_player
$$;

-- Aggregates one month of the ledger into monthly_summary. Safe to call
-- repeatedly (a month still in progress is simply recomputed).
CREATE PROCEDURE close_month(p_player INTEGER, p_month BIGINT)
LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO monthly_summary (player_id, month, income, expenses, taxes, net, closed_at)
    SELECT player_id, month, income, expenses, taxes, net, now()
    FROM v_monthly_cashflow
    WHERE player_id = p_player AND month = p_month
    ON CONFLICT (player_id, month) DO UPDATE
        SET income = EXCLUDED.income, expenses = EXCLUDED.expenses,
            taxes = EXCLUDED.taxes, net = EXCLUDED.net, closed_at = now();
END
$$;

-- Thins out old history: keeps full resolution for the last p_keep_recent
-- ticks and one point per p_step ticks before that.
CREATE PROCEDURE compact_history(
    p_player INTEGER, p_current_tick BIGINT,
    p_keep_recent BIGINT DEFAULT 2000, p_step BIGINT DEFAULT 30)
LANGUAGE plpgsql AS $$
BEGIN
    DELETE FROM price_history
    WHERE player_id = p_player AND tick < p_current_tick - p_keep_recent AND tick % p_step <> 0;
    DELETE FROM net_worth_history
    WHERE player_id = p_player AND tick < p_current_tick - p_keep_recent AND tick % p_step <> 0;
END
$$;

-- ---- Triggers --------------------------------------------------------

-- Keep running totals per category as transactions arrive.
CREATE FUNCTION trg_transactions_totals() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO category_totals (player_id, category, total, n)
    VALUES (NEW.player_id, NEW.category, NEW.amount, 1)
    ON CONFLICT (player_id, category)
    DO UPDATE SET total = category_totals.total + EXCLUDED.total, n = category_totals.n + 1;
    RETURN NEW;
END
$$;
CREATE TRIGGER transactions_totals AFTER INSERT ON transactions
FOR EACH ROW EXECUTE FUNCTION trg_transactions_totals();

-- The ledger is append-only: no editing history.
CREATE FUNCTION trg_transactions_immutable() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'transactions are immutable (id %)', OLD.id;
END
$$;
CREATE TRIGGER transactions_immutable BEFORE UPDATE ON transactions
FOR EACH ROW EXECUTE FUNCTION trg_transactions_immutable();

-- Track each save's best-ever net worth.
CREATE FUNCTION trg_net_worth_record() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO player_records (player_id, peak_net_worth, peak_tick)
    VALUES (NEW.player_id, NEW.net_worth, NEW.tick)
    ON CONFLICT (player_id) DO UPDATE
        SET peak_net_worth = EXCLUDED.peak_net_worth, peak_tick = EXCLUDED.peak_tick, updated_at = now()
        WHERE EXCLUDED.peak_net_worth > player_records.peak_net_worth;
    RETURN NEW;
END
$$;
CREATE TRIGGER net_worth_record AFTER INSERT OR UPDATE ON net_worth_history
FOR EACH ROW EXECUTE FUNCTION trg_net_worth_record();

-- Time only moves forward.
CREATE FUNCTION trg_players_tick_forward() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.current_tick < OLD.current_tick THEN
        RAISE EXCEPTION 'current_tick cannot go backwards (% -> %)', OLD.current_tick, NEW.current_tick;
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER players_tick_forward BEFORE UPDATE OF current_tick ON players
FOR EACH ROW EXECUTE FUNCTION trg_players_tick_forward();
