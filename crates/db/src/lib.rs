//! Thin persistence layer. Talks to Postgres via sqlx using runtime-
//! checked queries (not the `query!` macro) so the workspace compiles
//! without a live database connection - only actual gameplay needs one.

use std::collections::HashMap;

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};

pub async fn connect(database_url: &str) -> Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(database_url)
        .await?;
    Ok(pool)
}

/// Embeds the SQL files under ../../migrations into the binary at
/// compile time and applies any that haven't run yet.
pub async fn run_migrations(pool: &PgPool) -> Result<()> {
    sqlx::migrate!("../../migrations").run(pool).await?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct PlayerRow {
    pub id: i32,
    pub save_name: String,
    pub game_mode: String,
    pub cash: f64,
    pub salary: f64,
    pub expenses: f64,
    pub current_tick: i64,
    pub last_tick_at: DateTime<Utc>,
    pub interest_rate: f64,
}

#[derive(Debug, Clone)]
pub struct InstrumentRow {
    pub id: i32,
    pub symbol: String,
    pub name: String,
    pub class: String,
    pub drift: f64,
    pub volatility: f64,
    pub base_price: f64,
    pub face_value: f64,
    pub coupon_rate: f64,
    pub duration: f64,
    pub income_yield: f64,
    pub rate_sensitivity: f64,
    pub expense_ratio: f64,
    /// (constituent instrument id, weight) - mutual funds only.
    pub constituents: Vec<(i32, f64)>,
}

#[derive(Debug, Clone)]
pub struct PositionRow {
    pub instrument_id: i32,
    pub quantity: f64,
    pub avg_cost: f64,
}

pub async fn create_player(
    pool: &PgPool,
    save_name: &str,
    game_mode: &str,
    cash: f64,
    salary: f64,
    expenses: f64,
) -> Result<PlayerRow> {
    let row = sqlx::query(
        "INSERT INTO players (save_name, game_mode, cash, salary, expenses, current_tick, last_tick_at)
         VALUES ($1, $2, $3, $4, $5, 0, now())
         RETURNING id, save_name, game_mode, cash, salary, expenses, current_tick, last_tick_at, interest_rate",
    )
    .bind(save_name)
    .bind(game_mode)
    .bind(cash)
    .bind(salary)
    .bind(expenses)
    .fetch_one(pool)
    .await?;

    Ok(row_to_player_row(row))
}

pub async fn save_name_exists(pool: &PgPool, save_name: &str) -> Result<bool> {
    let row = sqlx::query("SELECT COUNT(*) AS c FROM players WHERE save_name = $1")
        .bind(save_name)
        .fetch_one(pool)
        .await?;
    let count: i64 = row.get("c");
    Ok(count > 0)
}

/// All saves, most recently played first - what the save-picker menu
/// lists on startup.
pub async fn list_players(pool: &PgPool) -> Result<Vec<PlayerRow>> {
    let rows = sqlx::query(
        "SELECT id, save_name, game_mode, cash, salary, expenses, current_tick, last_tick_at, interest_rate
         FROM players ORDER BY last_tick_at DESC",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(row_to_player_row).collect())
}

/// Deletes a save and everything that belongs to it (positions,
/// orders, trades, price and net worth history). Instrument
/// definitions are shared across saves and untouched.
pub async fn delete_player(pool: &PgPool, player_id: i32) -> Result<()> {
    let mut tx = pool.begin().await?;
    for table in [
        "positions",
        "orders",
        "trades",
        "price_history",
        "net_worth_history",
        "game_state",
        "transactions",
        "economy_history",
        "monthly_summary",
        "category_totals",
        "player_records",
    ] {
        sqlx::query(&format!("DELETE FROM {table} WHERE player_id = $1"))
            .bind(player_id)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("DELETE FROM players WHERE id = $1")
        .bind(player_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

fn row_to_player_row(row: sqlx::postgres::PgRow) -> PlayerRow {
    PlayerRow {
        id: row.get("id"),
        save_name: row.get("save_name"),
        game_mode: row.get("game_mode"),
        cash: row.get("cash"),
        salary: row.get("salary"),
        expenses: row.get("expenses"),
        current_tick: row.get("current_tick"),
        last_tick_at: row.get("last_tick_at"),
        interest_rate: row.get("interest_rate"),
    }
}

/// Persists cash, tick count, tick timing, and the prevailing interest
/// rate together - the things that change on every advance/trade.
/// Per-save, unlike the old global sim_state approach.
pub async fn save_progress(
    pool: &PgPool,
    player_id: i32,
    cash: f64,
    current_tick: i64,
    last_tick_at: DateTime<Utc>,
    interest_rate: f64,
) -> Result<()> {
    sqlx::query("UPDATE players SET cash = $1, current_tick = $2, last_tick_at = $3, interest_rate = $4 WHERE id = $5")
        .bind(cash)
        .bind(current_tick)
        .bind(last_tick_at)
        .bind(interest_rate)
        .bind(player_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// One seeded instrument definition.
struct Seed {
    symbol: &'static str,
    name: &'static str,
    class: &'static str,
    price: f64,
    drift: f64,
    vol: f64,
    // bond-only
    face_value: f64,
    coupon_rate: f64,
    duration: f64,
    // new classes
    income_yield: f64,
    rate_sensitivity: f64,
    expense_ratio: f64,
}

impl Seed {
    const fn base(symbol: &'static str, name: &'static str, class: &'static str, price: f64, drift: f64, vol: f64) -> Seed {
        Seed {
            symbol,
            name,
            class,
            price,
            drift,
            vol,
            face_value: 0.0,
            coupon_rate: 0.0,
            duration: 0.0,
            income_yield: 0.0,
            rate_sensitivity: 0.0,
            expense_ratio: 0.0,
        }
    }
}

/// Seeds any instruments that aren't in the database yet (matched by
/// symbol), so databases created by older versions pick up new
/// instruments on the next launch without touching existing ones.
/// Defines each instrument and its starting price - price *history* is
/// per-save (see seed_price_history_for_save and
/// backfill_price_history_for_save).
///
/// Bond starting prices are pre-computed at sim-core's
/// LONG_RUN_INTEREST_RATE (4%): GOVB's coupon matches that rate so it
/// starts at par; CORP's higher coupon starts above par.
pub async fn seed_instruments_if_missing(pool: &PgPool) -> Result<()> {
    let bond = |symbol, name, price, face, coupon, duration| Seed {
        face_value: face,
        coupon_rate: coupon,
        duration,
        ..Seed::base(symbol, name, "bond", price, 0.0, 0.0)
    };

    let seeds: Vec<Seed> = vec![
        Seed { income_yield: 0.005, ..Seed::base("AAPL", "Applecore Inc.", "equity", 180.0, 0.115, 0.28) },
        Seed { income_yield: 0.040, ..Seed::base("NRGX", "Nordic Energy", "equity", 65.0, 0.030, 0.35) },
        Seed { income_yield: 0.025, ..Seed::base("GROV", "Grover Foods", "equity", 42.0, 0.035, 0.22) },
        Seed { income_yield: 0.018, ..Seed::base("IDX1", "Broad Market Index Fund", "index_fund", 100.0, 0.062, 0.15) },
        Seed::base("SAVE", "High-Yield Savings", "savings", 1.0, 0.04, 0.0),
        Seed::base("WHET", "Wheat", "commodity", 6.5, 0.03, 0.30),
        Seed::base("OIL", "Crude Oil", "commodity", 78.0, 0.02, 0.40),
        Seed::base("STEE", "Steel", "commodity", 850.0, 0.03, 0.25),
        bond("GOVB", "Government Bond", 1000.0, 1000.0, 0.04, 7.0),
        bond("CORP", "Corporate Bond", 1080.0, 1000.0, 0.06, 4.0),
        // Real estate: listed property trusts. Pay rent, and their
        // drift falls when prevailing interest rates rise.
        Seed {
            income_yield: 0.035,
            rate_sensitivity: 1.5,
            ..Seed::base("PROP", "Metro Commercial Property Trust", "real_estate", 50.0, 0.05, 0.14)
        },
        Seed {
            income_yield: 0.030,
            rate_sensitivity: 1.0,
            ..Seed::base("HOME", "Harbor Residential Trust", "real_estate", 250.0, 0.045, 0.10)
        },
        // Forex: mean-reverting around the starting level, no drift.
        Seed::base("EURUSD", "Euro / US Dollar", "forex", 1.10, 0.0, 0.08),
        Seed::base("GBPUSD", "Pound / US Dollar", "forex", 1.27, 0.0, 0.09),
        Seed::base("USDJPY", "US Dollar / Yen", "forex", 150.0, 0.0, 0.10),
        // Crypto: high drift, very high volatility.
        Seed::base("BTC", "Bitcoin", "crypto", 60000.0, 0.15, 0.70),
        Seed::base("ETH", "Ethereum", "crypto", 3000.0, 0.12, 0.85),
        // Mutual funds: NAV derived from the baskets below.
        Seed { expense_ratio: 0.0075, ..Seed::base("BALF", "Balanced Growth Fund", "mutual_fund", 100.0, 0.0, 0.0) },
        Seed { expense_ratio: 0.0120, ..Seed::base("GRWF", "Aggressive Growth Fund", "mutual_fund", 100.0, 0.0, 0.0) },
        Seed { expense_ratio: 0.0090, ..Seed::base("REAL", "Real Assets Fund", "mutual_fund", 100.0, 0.0, 0.0) },
    ];

    for s in &seeds {
        sqlx::query(
            "INSERT INTO instruments
                (symbol, name, class, drift, volatility, base_price, face_value, coupon_rate, duration,
                 income_yield, rate_sensitivity, expense_ratio)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
             ON CONFLICT (symbol) DO NOTHING",
        )
        .bind(s.symbol)
        .bind(s.name)
        .bind(s.class)
        .bind(s.drift)
        .bind(s.vol)
        .bind(s.price)
        .bind(s.face_value)
        .bind(s.coupon_rate)
        .bind(s.duration)
        .bind(s.income_yield)
        .bind(s.rate_sensitivity)
        .bind(s.expense_ratio)
        .execute(pool)
        .await?;
    }

    // (fund symbol, [(constituent symbol, weight)]) - weights sum to 1.
    let baskets: &[(&str, &[(&str, f64)])] = &[
        ("BALF", &[("IDX1", 0.45), ("GOVB", 0.30), ("CORP", 0.15), ("PROP", 0.10)]),
        ("GRWF", &[("AAPL", 0.35), ("NRGX", 0.25), ("GROV", 0.15), ("IDX1", 0.25)]),
        ("REAL", &[("PROP", 0.30), ("HOME", 0.30), ("OIL", 0.20), ("STEE", 0.10), ("WHET", 0.10)]),
    ];
    for (fund, parts) in baskets {
        for (constituent, weight) in *parts {
            sqlx::query(
                "INSERT INTO fund_constituents (fund_id, constituent_id, weight)
                 SELECT f.id, c.id, $3 FROM instruments f, instruments c
                 WHERE f.symbol = $1 AND c.symbol = $2
                 ON CONFLICT DO NOTHING",
            )
            .bind(fund)
            .bind(constituent)
            .bind(weight)
            .execute(pool)
            .await?;
        }
    }

    Ok(())
}

pub async fn list_instruments(pool: &PgPool) -> Result<Vec<InstrumentRow>> {
    let rows = sqlx::query(
        "SELECT id, symbol, name, class, drift, volatility, base_price, face_value, coupon_rate, duration,
                income_yield, rate_sensitivity, expense_ratio
         FROM instruments ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    let mut constituents: HashMap<i32, Vec<(i32, f64)>> = HashMap::new();
    for r in sqlx::query("SELECT fund_id, constituent_id, weight FROM fund_constituents ORDER BY fund_id, constituent_id")
        .fetch_all(pool)
        .await?
    {
        constituents
            .entry(r.get("fund_id"))
            .or_default()
            .push((r.get("constituent_id"), r.get("weight")));
    }

    Ok(rows
        .into_iter()
        .map(|r| {
            let id: i32 = r.get("id");
            InstrumentRow {
                id,
                symbol: r.get("symbol"),
                name: r.get("name"),
                class: r.get("class"),
                drift: r.get("drift"),
                volatility: r.get("volatility"),
                base_price: r.get("base_price"),
                face_value: r.get("face_value"),
                coupon_rate: r.get("coupon_rate"),
                duration: r.get("duration"),
                income_yield: r.get("income_yield"),
                rate_sensitivity: r.get("rate_sensitivity"),
                expense_ratio: r.get("expense_ratio"),
                constituents: constituents.remove(&id).unwrap_or_default(),
            }
        })
        .collect())
}

/// Instruments added after a save was created (a newer version of the
/// game seeded them) have no price history in that save yet. Give each
/// one a starting point at the save's current tick, at its base price,
/// so the save can trade them from here on.
pub async fn backfill_price_history_for_save(pool: &PgPool, player_id: i32, current_tick: i64) -> Result<()> {
    sqlx::query(
        "INSERT INTO price_history (player_id, instrument_id, tick, price)
         SELECT $1, i.id, $2, i.base_price FROM instruments i
         WHERE NOT EXISTS (
             SELECT 1 FROM price_history ph WHERE ph.player_id = $1 AND ph.instrument_id = i.id
         )",
    )
    .bind(player_id)
    .bind(current_tick)
    .execute(pool)
    .await?;
    Ok(())
}

/// Inserts tick-0 price history for a brand new save, one row per
/// instrument at its base_price. Call once, right after creating the
/// player row.
pub async fn seed_price_history_for_save(pool: &PgPool, player_id: i32, instruments: &[InstrumentRow]) -> Result<()> {
    let mut tx = pool.begin().await?;
    for i in instruments {
        sqlx::query(
            "INSERT INTO price_history (player_id, instrument_id, tick, price) VALUES ($1, $2, 0, $3)",
        )
        .bind(player_id)
        .bind(i.id)
        .bind(i.base_price)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Current prices for one save's own price history - each save has an
/// independent price trajectory (its own RNG path from a shared
/// starting point), not a market shared across saves.
pub async fn latest_prices(pool: &PgPool, player_id: i32) -> Result<HashMap<i32, f64>> {
    let rows = sqlx::query(
        "SELECT DISTINCT ON (instrument_id) instrument_id, price
         FROM price_history
         WHERE player_id = $1
         ORDER BY instrument_id, tick DESC",
    )
    .bind(player_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| (r.get::<i32, _>("instrument_id"), r.get::<f64, _>("price")))
        .collect())
}

/// The most recent `limit` price points for one instrument within one
/// save, oldest first - what the TUI chart plots.
pub async fn recent_prices(pool: &PgPool, player_id: i32, instrument_id: i32, limit: i64) -> Result<Vec<(i64, f64)>> {
    let rows = sqlx::query(
        "SELECT tick, price FROM (
            SELECT tick, price FROM price_history
            WHERE player_id = $1 AND instrument_id = $2
            ORDER BY tick DESC
            LIMIT $3
         ) recent
         ORDER BY tick ASC",
    )
    .bind(player_id)
    .bind(instrument_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| (r.get::<i64, _>("tick"), r.get::<f64, _>("price")))
        .collect())
}

/// Rows per batched INSERT (kept well under Postgres' bind limits).
const INSERT_CHUNK: usize = 5_000;

pub async fn insert_price_points(pool: &PgPool, player_id: i32, points: &[sim_core::PricePoint]) -> Result<()> {
    let mut tx = pool.begin().await?;
    for chunk in points.chunks(INSERT_CHUNK) {
        let instrument_ids: Vec<i32> = chunk.iter().map(|p| p.instrument_id).collect();
        let ticks: Vec<i64> = chunk.iter().map(|p| p.tick).collect();
        let prices: Vec<f64> = chunk.iter().map(|p| p.price).collect();
        sqlx::query(
            "INSERT INTO price_history (player_id, instrument_id, tick, price)
             SELECT $1, * FROM UNNEST($2::int4[], $3::int8[], $4::float8[])",
        )
        .bind(player_id)
        .bind(&instrument_ids)
        .bind(&ticks)
        .bind(&prices)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn insert_net_worth_points(
    pool: &PgPool,
    player_id: i32,
    points: &[sim_core::NetWorthPoint],
) -> Result<()> {
    let mut tx = pool.begin().await?;
    for chunk in points.chunks(INSERT_CHUNK) {
        let ticks: Vec<i64> = chunk.iter().map(|p| p.tick).collect();
        let values: Vec<f64> = chunk.iter().map(|p| p.net_worth).collect();
        sqlx::query(
            "INSERT INTO net_worth_history (player_id, tick, net_worth)
             SELECT $1, * FROM UNNEST($2::int8[], $3::float8[])
             ON CONFLICT (player_id, tick) DO UPDATE SET net_worth = EXCLUDED.net_worth",
        )
        .bind(player_id)
        .bind(&ticks)
        .bind(&values)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// The most recent `limit` net worth points for one save, oldest
/// first - what the net worth chart view plots.
pub async fn recent_net_worth(pool: &PgPool, player_id: i32, limit: i64) -> Result<Vec<(i64, f64)>> {
    let rows = sqlx::query(
        "SELECT tick, net_worth FROM (
            SELECT tick, net_worth FROM net_worth_history
            WHERE player_id = $1
            ORDER BY tick DESC
            LIMIT $2
         ) recent
         ORDER BY tick ASC",
    )
    .bind(player_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| (r.get::<i64, _>("tick"), r.get::<f64, _>("net_worth")))
        .collect())
}

pub async fn load_positions(pool: &PgPool, player_id: i32) -> Result<Vec<PositionRow>> {
    let rows = sqlx::query("SELECT instrument_id, quantity, avg_cost FROM positions WHERE player_id = $1")
        .bind(player_id)
        .fetch_all(pool)
        .await?;

    Ok(rows
        .into_iter()
        .map(|r| PositionRow {
            instrument_id: r.get("instrument_id"),
            quantity: r.get("quantity"),
            avg_cost: r.get("avg_cost"),
        })
        .collect())
}

pub async fn load_orders(pool: &PgPool, player_id: i32) -> Result<Vec<sim_core::LimitOrder>> {
    let rows = sqlx::query(
        "SELECT id, instrument_id, is_buy, quantity, limit_price, placed_tick
         FROM orders WHERE player_id = $1 ORDER BY id",
    )
    .bind(player_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| sim_core::LimitOrder {
            id: r.get("id"),
            instrument_id: r.get("instrument_id"),
            is_buy: r.get("is_buy"),
            quantity: r.get("quantity"),
            limit_price: r.get("limit_price"),
            placed_tick: r.get("placed_tick"),
        })
        .collect())
}

/// Newest `limit` fills for one save, returned oldest-first.
pub async fn load_recent_fills(pool: &PgPool, player_id: i32, limit: i64) -> Result<Vec<sim_core::Fill>> {
    let rows = sqlx::query(
        "SELECT instrument_id, is_buy, quantity, price, tick, kind, realized_pnl FROM (
            SELECT id, instrument_id, is_buy, quantity, price, tick, kind, realized_pnl
            FROM trades WHERE player_id = $1 ORDER BY id DESC LIMIT $2
         ) recent ORDER BY id ASC",
    )
    .bind(player_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| sim_core::Fill {
            instrument_id: r.get("instrument_id"),
            is_buy: r.get("is_buy"),
            quantity: r.get("quantity"),
            price: r.get("price"),
            tick: r.get("tick"),
            kind: sim_core::FillKind::from_db_str(r.get::<String, _>("kind").as_str()),
            realized_pnl: r.get("realized_pnl"),
        })
        .collect())
}

/// Loads a save's game layer, or None for saves created before it
/// existed (the caller starts a fresh one).
pub async fn load_game_state(pool: &PgPool, player_id: i32) -> Result<Option<sim_core::GameState>> {
    let row = sqlx::query("SELECT state FROM game_state WHERE player_id = $1")
        .bind(player_id)
        .fetch_optional(pool)
        .await?;
    match row {
        Some(r) => Ok(Some(serde_json::from_str(&r.get::<String, _>("state"))?)),
        None => Ok(None),
    }
}

/// Persists everything a trade or a tick batch can change, atomically:
/// cash/salary/expenses/tick/rate progress, the full set of positions,
/// the full set of resting orders, the game layer, and any new fills. Positions and orders are small
/// per save, so replacing them wholesale is simpler and safer than
/// diffing.
pub async fn save_state(
    pool: &PgPool,
    player: &sim_core::Player,
    last_tick_at: DateTime<Utc>,
    portfolio: &sim_core::Portfolio,
    orders: &[sim_core::LimitOrder],
    game: &mut sim_core::GameState,
    new_fills: &[sim_core::Fill],
) -> Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "UPDATE players SET cash = $1, current_tick = $2, last_tick_at = $3, interest_rate = $4,
                salary = $6, expenses = $7
         WHERE id = $5",
    )
        .bind(player.cash)
        .bind(player.current_tick)
        .bind(last_tick_at)
        .bind(player.interest_rate)
        .bind(player.id)
        .bind(player.salary)
        .bind(player.expenses)
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        "INSERT INTO game_state (player_id, state) VALUES ($1, $2)
         ON CONFLICT (player_id) DO UPDATE SET state = EXCLUDED.state",
    )
    .bind(player.id)
    .bind(serde_json::to_string(&*game)?)
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM positions WHERE player_id = $1")
        .bind(player.id)
        .execute(&mut *tx)
        .await?;
    for pos in portfolio.positions.values() {
        if pos.quantity <= 0.0001 {
            continue;
        }
        sqlx::query("INSERT INTO positions (player_id, instrument_id, quantity, avg_cost) VALUES ($1, $2, $3, $4)")
            .bind(player.id)
            .bind(pos.instrument_id)
            .bind(pos.quantity)
            .bind(pos.avg_cost)
            .execute(&mut *tx)
            .await?;
    }

    sqlx::query("DELETE FROM orders WHERE player_id = $1")
        .bind(player.id)
        .execute(&mut *tx)
        .await?;
    for o in orders {
        sqlx::query(
            "INSERT INTO orders (player_id, id, instrument_id, is_buy, quantity, limit_price, placed_tick)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(player.id)
        .bind(o.id)
        .bind(o.instrument_id)
        .bind(o.is_buy)
        .bind(o.quantity)
        .bind(o.limit_price)
        .bind(o.placed_tick)
        .execute(&mut *tx)
        .await?;
    }

    for f in new_fills {
        sqlx::query(
            "INSERT INTO trades (player_id, instrument_id, is_buy, quantity, price, tick, kind, realized_pnl)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(player.id)
        .bind(f.instrument_id)
        .bind(f.is_buy)
        .bind(f.quantity)
        .bind(f.price)
        .bind(f.tick)
        .bind(f.kind.as_str())
        .bind(f.realized_pnl)
        .execute(&mut *tx)
        .await?;
    }

    // The transaction ledger: the game's cash movements, plus one row
    // per trade. Triggers keep running totals; close_month() rolls the
    // months up into monthly_summary.
    let mut months: std::collections::BTreeSet<i64> = std::collections::BTreeSet::new();
    for chunk in game.ledger.chunks(INSERT_CHUNK) {
        let ticks: Vec<i64> = chunk.iter().map(|e| e.tick).collect();
        let categories: Vec<String> = chunk.iter().map(|e| e.category.clone()).collect();
        let descriptions: Vec<String> = chunk.iter().map(|e| e.description.clone()).collect();
        let amounts: Vec<f64> = chunk.iter().map(|e| e.amount).collect();
        sqlx::query(
            "INSERT INTO transactions (player_id, tick, category, description, amount)
             SELECT $1, * FROM UNNEST($2::int8[], $3::text[], $4::text[], $5::float8[])",
        )
        .bind(player.id)
        .bind(&ticks)
        .bind(&categories)
        .bind(&descriptions)
        .bind(&amounts)
        .execute(&mut *tx)
        .await?;
        months.extend(ticks.iter().map(|t| t / 30));
    }
    for f in new_fills {
        let amount = if f.is_buy { -f.quantity * f.price } else { f.quantity * f.price };
        let verb = format!("{} {:.4}", if f.is_buy { "BUY" } else { "SELL" }, f.quantity);
        sqlx::query(
            "INSERT INTO transactions (player_id, tick, category, description, amount)
             SELECT $1, $2, 'trade', $3 || ' ' || i.symbol, $4 FROM instruments i WHERE i.id = $5",
        )
        .bind(player.id)
        .bind(f.tick)
        .bind(verb)
        .bind(amount)
        .bind(f.instrument_id)
        .execute(&mut *tx)
        .await?;
        months.insert(f.tick / 30);
    }
    for p in &game.new_points {
        sqlx::query(
            "INSERT INTO economy_history
                (player_id, month, tick, cpi, inflation, gdp_growth, unemployment, policy_rate, debt_to_gdp)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (player_id, month) DO UPDATE SET
                tick = EXCLUDED.tick, cpi = EXCLUDED.cpi, inflation = EXCLUDED.inflation,
                gdp_growth = EXCLUDED.gdp_growth, unemployment = EXCLUDED.unemployment,
                policy_rate = EXCLUDED.policy_rate, debt_to_gdp = EXCLUDED.debt_to_gdp",
        )
        .bind(player.id)
        .bind(p.month)
        .bind(p.tick)
        .bind(p.cpi)
        .bind(p.inflation)
        .bind(p.gdp_growth)
        .bind(p.unemployment)
        .bind(p.policy_rate)
        .bind(p.debt_to_gdp)
        .execute(&mut *tx)
        .await?;
    }
    for m in months {
        sqlx::query("CALL close_month($1, $2)").bind(player.id).bind(m).execute(&mut *tx).await?;
    }

    tx.commit().await?;
    // Only forget what was written once it is safely committed.
    game.ledger.clear();
    game.new_points.clear();
    Ok(())
}

/// One closed month from the ledger (produced by the close_month procedure).
#[derive(Debug, Clone)]
pub struct CashflowRow {
    pub month: i64,
    pub income: f64,
    pub expenses: f64,
    pub taxes: f64,
    pub net: f64,
}

pub async fn recent_cashflow(pool: &PgPool, player_id: i32, months: i64) -> Result<Vec<CashflowRow>> {
    let rows = sqlx::query(
        "SELECT month, income, expenses, taxes, net FROM monthly_summary
         WHERE player_id = $1 ORDER BY month DESC LIMIT $2",
    )
    .bind(player_id)
    .bind(months)
    .fetch_all(pool)
    .await?;
    let mut out: Vec<CashflowRow> = rows
        .into_iter()
        .map(|r| CashflowRow {
            month: r.get("month"),
            income: r.get("income"),
            expenses: r.get("expenses"),
            taxes: r.get("taxes"),
            net: r.get("net"),
        })
        .collect();
    out.reverse();
    Ok(out)
}

/// One row of the transaction ledger, for the Logs view.
#[derive(Debug, Clone)]
pub struct LedgerRow {
    pub tick: i64,
    pub category: String,
    pub description: String,
    pub amount: f64,
}

/// The newest `limit` ledger rows for a save, newest first.
pub async fn recent_transactions(pool: &PgPool, player_id: i32, limit: i64) -> Result<Vec<LedgerRow>> {
    let rows = sqlx::query(
        "SELECT tick, category, description, amount FROM transactions
         WHERE player_id = $1 ORDER BY id DESC LIMIT $2",
    )
    .bind(player_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| LedgerRow {
            tick: r.get("tick"),
            category: r.get("category"),
            description: r.get("description"),
            amount: r.get("amount"),
        })
        .collect())
}

/// Lifetime totals per ledger category (kept up to date by a trigger).
pub async fn category_totals(pool: &PgPool, player_id: i32) -> Result<Vec<(String, f64)>> {
    let rows = sqlx::query("SELECT category, total FROM category_totals WHERE player_id = $1 ORDER BY total DESC")
        .bind(player_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|r| (r.get("category"), r.get("total"))).collect())
}

/// Thins old price/net worth history (full resolution for recent ticks,
/// one point per 30 ticks before that). Calls the compact_history procedure.
pub async fn compact_history(pool: &PgPool, player_id: i32, current_tick: i64) -> Result<()> {
    sqlx::query("CALL compact_history($1, $2)")
        .bind(player_id)
        .bind(current_tick)
        .execute(pool)
        .await?;
    Ok(())
}

// Note: tick catch-up timing used to live in a global `sim_state`
// singleton row. It's now per-save (players.last_tick_at). The
// sim_state table itself is left in the schema unused rather than
// dropped, to keep migrations additive-only.
