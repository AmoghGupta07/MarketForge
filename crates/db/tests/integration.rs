//! Integration tests against a real Postgres. They need `DATABASE_URL`
//! (any database on the server; the tests create and drop their own
//! scratch databases, so your dev data is never touched) and are
//! skipped with a message if it isn't set.

use sim_core::{fast_forward_with_rng, AssetClass, Constituent, GameMode, GameState, Instrument, InstrumentState, Market, Player, Portfolio};
use sqlx::{PgPool, Row};

fn base_url() -> Option<String> {
    std::env::var("DATABASE_URL").ok()
}

/// Drops and recreates a scratch database, returns a pool to it.
async fn scratch_db(name: &str) -> PgPool {
    let url = base_url().unwrap();
    let admin = db::connect(&url).await.unwrap();
    sqlx::query(&format!("DROP DATABASE IF EXISTS {name}")).execute(&admin).await.unwrap();
    sqlx::query(&format!("CREATE DATABASE {name}")).execute(&admin).await.unwrap();
    let (prefix, _) = url.rsplit_once('/').unwrap();
    db::connect(&format!("{prefix}/{name}")).await.unwrap()
}

fn to_states(rows: Vec<db::InstrumentRow>, prices: &std::collections::HashMap<i32, f64>) -> Vec<InstrumentState> {
    rows.into_iter()
        .map(|i| InstrumentState {
            price: prices.get(&i.id).copied().unwrap_or(i.base_price),
            instrument: Instrument {
                id: i.id,
                symbol: i.symbol,
                name: i.name,
                class: AssetClass::from_db_str(&i.class),
                base_price: i.base_price,
                drift: i.drift,
                volatility: i.volatility,
                face_value: i.face_value,
                coupon_rate: i.coupon_rate,
                duration: i.duration,
                income_yield: i.income_yield,
                rate_sensitivity: i.rate_sensitivity,
                expense_ratio: i.expense_ratio,
                constituents: i
                    .constituents
                    .into_iter()
                    .map(|(instrument_id, weight)| Constituent { instrument_id, weight })
                    .collect(),
            },
        })
        .collect()
}

#[tokio::test]
async fn fresh_database_full_roundtrip() {
    if base_url().is_none() {
        eprintln!("DATABASE_URL not set - skipping");
        return;
    }
    let pool = scratch_db("econsim_it_fresh").await;
    db::run_migrations(&pool).await.unwrap();
    db::seed_instruments_if_missing(&pool).await.unwrap();
    db::seed_instruments_if_missing(&pool).await.unwrap(); // idempotent

    let instruments = db::list_instruments(&pool).await.unwrap();
    assert_eq!(instruments.len(), 20);
    let idx = instruments.iter().find(|i| i.symbol == "IDX1").unwrap();
    assert_eq!(idx.income_yield, 0.018);
    assert_eq!(idx.drift, 0.062);
    for fund in instruments.iter().filter(|i| i.class == "mutual_fund") {
        let total: f64 = fund.constituents.iter().map(|c| c.1).sum();
        assert!((total - 1.0).abs() < 1e-9, "{} weights sum to {total}", fund.symbol);
    }

    let row = db::create_player(&pool, "it", "salaried_investor", 50_000.0, 4_000.0, 3_200.0).await.unwrap();
    db::seed_price_history_for_save(&pool, row.id, &instruments).await.unwrap();

    let mut player = Player {
        id: row.id,
        game_mode: GameMode::SalariedInvestor,
        cash: row.cash,
        salary: row.salary,
        expenses: row.expenses,
        current_tick: 0,
        interest_rate: row.interest_rate,
    };
    let prices = db::latest_prices(&pool, row.id).await.unwrap();
    let mut states = to_states(instruments.clone(), &prices);
    let mut portfolio = Portfolio::default();
    let mut market = Market::new(&states, vec![]);

    let id_of = |sym: &str| states.iter().find(|s| s.instrument.symbol == sym).unwrap().instrument.id;
    let (balf, btc, prop) = (id_of("BALF"), id_of("BTC"), id_of("PROP"));

    let mut game = GameState::new(&player, 50_000.0, true);
    let fill = market.market_order(&mut player, &mut portfolio, balf, true, 50.0).unwrap();
    let out = market.limit_order(&mut player, &mut portfolio, btc, true, 0.1, 30_000.0).unwrap();
    assert!(out.resting.is_some());
    db::save_state(&pool, &player, chrono::Utc::now(), &portfolio, &market.orders, &mut game, &[fill]).await.unwrap();

    // Run 400 ticks (crosses payday boundaries) and persist.
    let mut rng = <rand::rngs::StdRng as rand::SeedableRng>::seed_from_u64(11);
    let tick = fast_forward_with_rng(&mut player, &mut states, &mut portfolio, &mut market, &mut game, 400, false, &mut rng);
    db::insert_price_points(&pool, row.id, &tick.price_points).await.unwrap();
    db::insert_net_worth_points(&pool, row.id, &tick.net_worth_points).await.unwrap();
    db::save_state(&pool, &player, chrono::Utc::now(), &portfolio, &market.orders, &mut game, &tick.fills).await.unwrap();

    // Reload and compare.
    let reloaded = db::list_players(&pool).await.unwrap().remove(0);
    assert_eq!(reloaded.current_tick, 400);
    assert!((reloaded.cash - player.cash).abs() < 1e-6);
    let positions = db::load_positions(&pool, row.id).await.unwrap();
    assert_eq!(positions.len(), portfolio.positions.len());
    assert_eq!(reloaded.salary, player.salary);
    assert_eq!(reloaded.expenses, player.expenses);
    // The ledger was written, drained from memory, and summarized by SQL.
    assert!(game.ledger.is_empty() && game.new_points.is_empty());
    let totals = db::category_totals(&pool, row.id).await.unwrap();
    assert!(totals.iter().any(|(c, _)| c == "salary"), "{totals:?}");
    assert!(totals.iter().any(|(c, _)| c == "trade"));
    let cashflow = db::recent_cashflow(&pool, row.id, 100).await.unwrap();
    assert!(cashflow.len() >= 12, "{} months closed", cashflow.len());
    assert!(cashflow.iter().all(|m| m.taxes >= 0.0 && m.expenses >= 0.0));
    let eco: i64 = sqlx::query("SELECT COUNT(*) AS c FROM economy_history WHERE player_id = $1")
        .bind(row.id).fetch_one(&pool).await.unwrap().get("c");
    assert!(eco >= 13, "{eco} months of economy history");
    let loaded_game = db::load_game_state(&pool, row.id).await.unwrap().expect("game state saved");
    assert_eq!(serde_json::to_string(&loaded_game).unwrap(), serde_json::to_string(&game).unwrap());
    assert!(!loaded_game.news.is_empty());
    let orders = db::load_orders(&pool, row.id).await.unwrap();
    assert_eq!(orders, market.orders);
    let fills = db::load_recent_fills(&pool, row.id, 100).await.unwrap();
    assert_eq!(fills.len(), 1 + tick.fills.len());
    assert!(db::recent_net_worth(&pool, row.id, 10).await.unwrap().len() == 10);
    let latest = db::latest_prices(&pool, row.id).await.unwrap();
    assert_eq!(latest.len(), 20);
    let _ = prop;

    // Deleting a save with full history must not trip foreign keys.
    db::delete_player(&pool, row.id).await.unwrap();
    for table in ["players", "positions", "orders", "trades", "price_history", "net_worth_history", "game_state", "transactions", "economy_history", "monthly_summary", "category_totals", "player_records"] {
        let n: i64 = sqlx::query(&format!("SELECT COUNT(*) AS c FROM {table}")).fetch_one(&pool).await.unwrap().get("c");
        assert_eq!(n, 0, "{table} not empty after delete");
    }
}

#[tokio::test]
async fn upgrades_a_database_from_the_previous_version() {
    if base_url().is_none() {
        eprintln!("DATABASE_URL not set - skipping");
        return;
    }
    let pool = scratch_db("econsim_it_upgrade").await;

    // Apply only the first five migrations, as an old install would have.
    let mut old = sqlx::migrate!("../../migrations");
    let kept: Vec<_> = old.migrations.iter().filter(|m| m.version <= 5).cloned().collect();
    old.migrations = std::borrow::Cow::Owned(kept);
    old.run(&pool).await.unwrap();

    // Old-style data: one instrument, one save mid-game with history.
    sqlx::query("INSERT INTO instruments (symbol, name, class, drift, volatility, base_price) VALUES ('AAPL','Applecore Inc.','equity',0.12,0.28,180)")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO players (save_name, game_mode, cash, salary, expenses, current_tick) VALUES ('old','salaried_investor',1234,4000,3200,100)")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO price_history (player_id, instrument_id, tick, price) SELECT p.id, i.id, 100, 199.5 FROM players p, instruments i")
        .execute(&pool).await.unwrap();

    // Now the new version starts up.
    db::run_migrations(&pool).await.unwrap();
    db::seed_instruments_if_missing(&pool).await.unwrap();
    let player = db::list_players(&pool).await.unwrap().remove(0);
    db::backfill_price_history_for_save(&pool, player.id, player.current_tick).await.unwrap();

    let instruments = db::list_instruments(&pool).await.unwrap();
    assert_eq!(instruments.len(), 20);
    let prices = db::latest_prices(&pool, player.id).await.unwrap();
    assert_eq!(prices.len(), 20, "new instruments get a starting price in the old save");
    let aapl = instruments.iter().find(|i| i.symbol == "AAPL").unwrap();
    assert_eq!((aapl.income_yield, aapl.drift), (0.005, 0.115), "old rows get dividends via migration 0008");
    assert_eq!(prices[&aapl.id], 199.5, "existing price history untouched");
    let btc = instruments.iter().find(|i| i.symbol == "BTC").unwrap();
    assert_eq!(prices[&btc.id], 60_000.0);
    // An old save has no game layer yet.
    assert!(db::load_game_state(&pool, player.id).await.unwrap().is_none());
    // Running it again is a no-op.
    db::backfill_price_history_for_save(&pool, player.id, player.current_tick).await.unwrap();
    assert_eq!(db::latest_prices(&pool, player.id).await.unwrap().len(), 20);
}

/// Exercises the SQL objects directly: views, stored procedures/functions, triggers.
#[tokio::test]
async fn sql_views_procedures_and_triggers() {
    if base_url().is_none() {
        eprintln!("DATABASE_URL not set - skipping");
        return;
    }
    let pool = scratch_db("econsim_it_sql").await;
    db::run_migrations(&pool).await.unwrap();
    db::seed_instruments_if_missing(&pool).await.unwrap();
    let instruments = db::list_instruments(&pool).await.unwrap();
    let row = db::create_player(&pool, "sql", "salaried_investor", 1_000.0, 4_000.0, 3_200.0).await.unwrap();
    db::seed_price_history_for_save(&pool, row.id, &instruments).await.unwrap();
    let pid = row.id;

    // -- trigger: running totals per category
    for (tick, cat, amt) in [(5, "salary", 4000.0), (6, "tax", -200.0), (7, "living", -3200.0), (40, "salary", 4000.0), (41, "trade", -500.0)] {
        sqlx::query("INSERT INTO transactions (player_id, tick, category, description, amount) VALUES ($1,$2,$3,'t',$4)")
            .bind(pid).bind(tick as i64).bind(cat).bind(amt).execute(&pool).await.unwrap();
    }
    let totals = db::category_totals(&pool, pid).await.unwrap();
    let salary = totals.iter().find(|t| t.0 == "salary").unwrap().1;
    assert_eq!(salary, 8000.0);

    // -- trigger: the ledger is append-only
    let err = sqlx::query("UPDATE transactions SET amount = 1 WHERE player_id = $1").bind(pid).execute(&pool).await;
    assert!(err.unwrap_err().to_string().contains("immutable"));
    // -- check constraint: no zero-amount rows
    assert!(sqlx::query("INSERT INTO transactions (player_id, tick, category, description, amount) VALUES ($1,1,'x','x',0)")
        .bind(pid).execute(&pool).await.is_err());

    // -- procedure + view: month roll-up. Month 0 = ticks 0-29, month 1 = 30-59.
    sqlx::query("CALL close_month($1, 0)").bind(pid).execute(&pool).await.unwrap();
    sqlx::query("CALL close_month($1, 1)").bind(pid).execute(&pool).await.unwrap();
    sqlx::query("CALL close_month($1, 1)").bind(pid).execute(&pool).await.unwrap(); // idempotent
    let cf = db::recent_cashflow(&pool, pid, 10).await.unwrap();
    assert_eq!(cf.len(), 2);
    assert_eq!((cf[0].month, cf[0].income, cf[0].expenses, cf[0].taxes, cf[0].net), (0, 4000.0, 3200.0, 200.0, 600.0));
    // Month 1: the trade is a transfer, not an expense.
    assert_eq!((cf[1].income, cf[1].expenses, cf[1].net), (4000.0, 0.0, 4000.0));
    let v: f64 = sqlx::query("SELECT net FROM v_monthly_cashflow WHERE player_id = $1 AND month = 0")
        .bind(pid).fetch_one(&pool).await.unwrap().get("net");
    assert_eq!(v, 600.0);

    // -- housing movements are transfers, not spending; the Logs query reads the ledger
    for (tick, cat, amt) in [(160, "home", -25_000.0), (161, "mortgage_principal", -400.0), (162, "housing", -500.0), (163, "salary", 4000.0)] {
        sqlx::query("INSERT INTO transactions (player_id, tick, category, description, amount) VALUES ($1,$2,$3,'t',$4)")
            .bind(pid).bind(tick as i64).bind(cat).bind(amt).execute(&pool).await.unwrap();
    }
    sqlx::query("CALL close_month($1, 5)").bind(pid).execute(&pool).await.unwrap();
    let m5 = db::recent_cashflow(&pool, pid, 10).await.unwrap().into_iter().find(|m| m.month == 5).unwrap();
    assert_eq!((m5.income, m5.expenses, m5.net), (4000.0, 500.0, 3500.0), "only the housing cost counts as spending");
    let log = db::recent_transactions(&pool, pid, 3).await.unwrap();
    assert_eq!(log.len(), 3);
    assert_eq!((log[0].tick, log[0].category.as_str()), (163, "salary"), "newest first");

    // -- buying an asset is a transfer; its rent and running costs are income and spending
    for (tick, cat, amt) in [(190, "asset", -30_000.0), (191, "rent_income", 700.0), (192, "asset_costs", -200.0)] {
        sqlx::query("INSERT INTO transactions (player_id, tick, category, description, amount) VALUES ($1,$2,$3,'t',$4)")
            .bind(pid).bind(tick as i64).bind(cat).bind(amt).execute(&pool).await.unwrap();
    }
    sqlx::query("CALL close_month($1, 6)").bind(pid).execute(&pool).await.unwrap();
    let m6 = db::recent_cashflow(&pool, pid, 10).await.unwrap().into_iter().find(|m| m.month == 6).unwrap();
    assert_eq!((m6.income, m6.expenses, m6.net), (700.0, 200.0, 500.0), "the purchase itself is not spending");

    // -- realized profit on a sale is stored and reloaded
    sqlx::query("INSERT INTO trades (player_id, instrument_id, is_buy, quantity, price, tick, kind, realized_pnl) VALUES ($1,$2,false,5,120,9,'market',123.45)")
        .bind(pid).bind(instruments[0].id).execute(&pool).await.unwrap();
    let fills = db::load_recent_fills(&pool, pid, 10).await.unwrap();
    let sale = fills.iter().find(|f| !f.is_buy).expect("the sale");
    assert!((sale.realized_pnl - 123.45).abs() < 1e-9);

    // -- views over positions and prices, and the portfolio_value function
    let aapl = instruments.iter().find(|i| i.symbol == "AAPL").unwrap();
    sqlx::query("INSERT INTO positions (player_id, instrument_id, quantity, avg_cost) VALUES ($1,$2,10,150)")
        .bind(pid).bind(aapl.id).execute(&pool).await.unwrap();
    let r = sqlx::query("SELECT market_value, unrealized_pnl, return_pct FROM v_portfolio WHERE player_id = $1 AND symbol = 'AAPL'")
        .bind(pid).fetch_one(&pool).await.unwrap();
    assert!((r.get::<f64, _>("market_value") - 10.0 * aapl.base_price).abs() < 1e-6);
    assert!((r.get::<f64, _>("unrealized_pnl") - 10.0 * (aapl.base_price - 150.0)).abs() < 1e-6);
    let pv: f64 = sqlx::query("SELECT portfolio_value($1) AS v").bind(pid).fetch_one(&pool).await.unwrap().get("v");
    assert!((pv - 10.0 * aapl.base_price).abs() < 1e-6);
    let bs = sqlx::query("SELECT cash, investments, cash_and_investments FROM v_balance_sheet WHERE player_id = $1")
        .bind(pid).fetch_one(&pool).await.unwrap();
    assert!((bs.get::<f64, _>("cash_and_investments") - (1_000.0 + pv)).abs() < 1e-6);
    // A negative position is rejected by the constraint.
    assert!(sqlx::query("UPDATE positions SET quantity = -1 WHERE player_id = $1").bind(pid).execute(&pool).await.is_err());

    // -- trigger: peak net worth tracking
    for (tick, nw) in [(1i64, 100.0), (2, 500.0), (3, 300.0)] {
        sqlx::query("INSERT INTO net_worth_history (player_id, tick, net_worth) VALUES ($1,$2,$3)")
            .bind(pid).bind(tick).bind(nw).execute(&pool).await.unwrap();
    }
    let rec = sqlx::query("SELECT peak_net_worth, peak_tick FROM player_records WHERE player_id = $1")
        .bind(pid).fetch_one(&pool).await.unwrap();
    assert_eq!((rec.get::<f64, _>("peak_net_worth"), rec.get::<i64, _>("peak_tick")), (500.0, 2));

    // -- trigger: time can't run backwards
    sqlx::query("UPDATE players SET current_tick = 100 WHERE id = $1").bind(pid).execute(&pool).await.unwrap();
    let back = sqlx::query("UPDATE players SET current_tick = 50 WHERE id = $1").bind(pid).execute(&pool).await;
    assert!(back.unwrap_err().to_string().contains("backwards"));

    // -- economy view
    for m in 1..=3i64 {
        sqlx::query("INSERT INTO economy_history VALUES ($1,$2,$3,101,0.025,0.025,0.05,0.04,0.8)")
            .bind(pid).bind(m).bind(m * 30).execute(&pool).await.unwrap();
    }
    let latest: i64 = sqlx::query("SELECT month FROM v_economy_latest WHERE player_id = $1").bind(pid).fetch_one(&pool).await.unwrap().get("month");
    assert_eq!(latest, 3);

    // -- procedure: history compaction thins old points, keeps recent ones
    let ticks: Vec<i64> = (1..=3_000).collect();
    let prices = vec![1.0f64; ticks.len()];
    let ids = vec![aapl.id; ticks.len()];
    sqlx::query("INSERT INTO price_history (player_id, instrument_id, tick, price) SELECT $1, * FROM UNNEST($2::int4[], $3::int8[], $4::float8[]) ON CONFLICT DO NOTHING")
        .bind(pid).bind(&ids).bind(&ticks).bind(&prices).execute(&pool).await.unwrap();
    let count = |lo: i64, hi: i64| {
        let pool = pool.clone();
        async move {
            let n: i64 = sqlx::query("SELECT COUNT(*) AS c FROM price_history WHERE player_id = $1 AND instrument_id = $2 AND tick BETWEEN $3 AND $4")
                .bind(pid).bind(aapl.id).bind(lo).bind(hi).fetch_one(&pool).await.unwrap().get("c");
            n
        }
    };
    let (old_before, recent_before) = (count(1, 999).await, count(1_000, 3_000).await);
    db::compact_history(&pool, pid, 3_000).await.unwrap();
    let (old_after, recent_after) = (count(1, 999).await, count(1_000, 3_000).await);
    assert_eq!(recent_after, recent_before, "recent history untouched");
    assert!(old_after < old_before / 20 && old_after > 0, "{old_before} -> {old_after}");
    db::compact_history(&pool, pid, 3_000).await.unwrap(); // idempotent
}
