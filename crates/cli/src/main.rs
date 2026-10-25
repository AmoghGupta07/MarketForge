mod app;
mod manual;
mod ui;
mod ui_game;

use std::collections::HashMap;
use std::io::{self, Write};

use anyhow::Result;
use chrono::{DateTime, Utc};
use crossterm::event::{Event, EventStream, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use futures_util::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::time::{interval, Duration};

use sim_core::{
    fast_forward, fast_forward_with_rng, ticks_elapsed_since, AssetClass, Constituent, GameMode, GameState, Instrument,
    InstrumentState, Market, NegStep, Player, Portfolio, TradeError,
};

use app::{App, ChartMode, InputMode, Tab};
use ui::{fmt_price, fmt_qty};

/// While the app is "live," one tick corresponds to this many real
/// seconds. Also used to compute how many ticks to catch up on launch:
/// One tick (one simulated day) is this many real seconds, both while you
/// play (the dashboard ticks by itself at this rate) and when catching up
/// after time away. Override with MARKETFORGE_TICK_SECONDS.
const DEFAULT_TICK_SECONDS: u64 = 60;

/// The most time the world will catch up on when you reopen a save: one
/// real day, i.e. 1,440 ticks (about four simulated years) at the default
/// pace. Longer absences are capped so a week away doesn't fast-forward
/// decades of markets. Override with MARKETFORGE_MAX_AWAY_TICKS.
const DEFAULT_MAX_AWAY_TICKS: i64 = 1_440;

fn tick_seconds() -> u64 {
    std::env::var("MARKETFORGE_TICK_SECONDS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|s| *s >= 1)
        .unwrap_or(DEFAULT_TICK_SECONDS)
}

fn max_away_ticks() -> i64 {
    std::env::var("MARKETFORGE_MAX_AWAY_TICKS")
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|t| *t >= 0)
        .unwrap_or(DEFAULT_MAX_AWAY_TICKS)
}

/// How many recent price points the chart panel keeps per instrument.
const HISTORY_LEN: usize = 60;

#[tokio::main]
async fn main() -> Result<()> {
    // Load DATABASE_URL (and anything else) from a `.env` file in the
    // working directory or a parent, if there is one. Real environment
    // variables win over the file, and a missing file is fine.
    if let Ok(path) = dotenvy::dotenv() {
        println!("Loaded settings from {}", path.display());
    }

    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://localhost/econsim".to_string());

    println!("Connecting to {database_url} ...");
    let pool = db::connect(&database_url).await?;
    db::run_migrations(&pool).await?;
    db::seed_instruments_if_missing(&pool).await?;

    // Save selection happens on plain stdout, before the alternate
    // screen takes over - simplest way to keep one-off prompts out of
    // the TUI's event loop.
    let row = select_or_create_save(&pool).await?;
    let mut player = row_to_player(&row);
    let mut last_tick_at = row.last_tick_at;

    // Holdings, resting orders and prices are loaded up front: catch-up
    // replays ticks in which resting limit orders can fill, and the
    // player can't act while that happens.
    db::backfill_price_history_for_save(&pool, player.id, player.current_tick).await?;
    let mut portfolio = load_portfolio(&pool, player.id).await?;
    let mut instruments = load_instrument_states(&pool, player.id).await?;
    let mut orders = db::load_orders(&pool, player.id).await?;
    // Saves from before the game layer existed get a fresh one, with
    // rivals scaled to where this save's net worth is now.
    let mut game = match db::load_game_state(&pool, player.id).await? {
        Some(g) => g,
        None => {
            let prices: HashMap<i32, f64> = instruments.iter().map(|s| (s.instrument.id, s.price)).collect();
            let nw = player.cash + portfolio.market_value(&prices);
            println!("Starting the game layer for this save (career, events, rivals, deals).");
            GameState::new(&player, nw, true)
        }
    };
    game.ensure_compat(&player);
    retire_savings_holdings(&pool, &player, &mut portfolio, &mut orders, &mut game).await?;
    let mut market = Market::new(&instruments, orders);
    catch_up(
        &pool,
        &mut player,
        &mut last_tick_at,
        &mut instruments,
        &mut portfolio,
        &mut market,
        &mut game,
    )
    .await?;

    db::compact_history(&pool, player.id, player.current_tick).await?;

    let mut app = build_app(
        &pool,
        row.save_name,
        player,
        last_tick_at,
        instruments,
        portfolio,
        market,
        game,
    )
    .await?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, &pool, &mut app).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

async fn select_or_create_save(pool: &sqlx::PgPool) -> Result<db::PlayerRow> {
    loop {
        let saves = db::list_players(pool).await?;

        if saves.is_empty() {
            println!("No saves found. Let's create one.\n");
            return create_new_save(pool).await;
        }

        println!("Saves:");
        for (i, s) in saves.iter().enumerate() {
            println!(
                "  {}) {}  day {}  cash {:.2}",
                i + 1,
                s.save_name,
                s.current_tick,
                s.cash
            );
        }
        println!("  n) New save");
        println!("  d) Delete a save");
        print!("> ");
        io::stdout().flush()?;

        let mut choice = String::new();
        io::stdin().read_line(&mut choice)?;
        let choice = choice.trim();

        if choice.eq_ignore_ascii_case("n") {
            return create_new_save(pool).await;
        }

        if choice.eq_ignore_ascii_case("d") {
            print!("Delete which number? > ");
            io::stdout().flush()?;
            let mut idx_input = String::new();
            io::stdin().read_line(&mut idx_input)?;
            if let Ok(idx) = idx_input.trim().parse::<usize>() {
                if idx >= 1 && idx <= saves.len() {
                    db::delete_player(pool, saves[idx - 1].id).await?;
                    println!("Deleted '{}'.\n", saves[idx - 1].save_name);
                }
            }
            continue;
        }

        if let Ok(idx) = choice.parse::<usize>() {
            if idx >= 1 && idx <= saves.len() {
                return Ok(saves[idx - 1].clone());
            }
        }

        println!("Not a valid choice.\n");
    }
}

async fn create_new_save(pool: &sqlx::PgPool) -> Result<db::PlayerRow> {
    let save_name = loop {
        print!("Save name> ");
        io::stdout().flush()?;
        let mut name = String::new();
        io::stdin().read_line(&mut name)?;
        let name = name.trim().to_string();

        if name.is_empty() {
            println!("Name can't be empty.");
            continue;
        }
        if db::save_name_exists(pool, &name).await? {
            println!("That name's taken - pick another.");
            continue;
        }
        break name;
    };

    // Trader mode is cut for now - both the instrument set and the
    // trading mechanics (spread/costs, leverage) it'd need to feel
    // like a genuinely different game from Salaried Investor were
    // never built, so a mode picker here would just be a cosmetic
    // choice with no real difference behind it. Revisit if/when
    // Trader gets real mechanics of its own.
    let row = db::create_player(pool, &save_name, "salaried_investor", 2_000.0, 4_000.0, 3_200.0).await?;
    let instruments = db::list_instruments(pool).await?;
    db::seed_price_history_for_save(pool, row.id, &instruments).await?;
    println!("\nCreated save '{}'. Starting cash: {:.2}\n", row.save_name, row.cash);
    Ok(row)
}

fn row_to_player(row: &db::PlayerRow) -> Player {
    let game_mode = if row.game_mode == "trader" {
        GameMode::Trader
    } else {
        GameMode::SalariedInvestor
    };
    Player {
        id: row.id,
        game_mode,
        cash: row.cash,
        salary: row.salary,
        expenses: row.expenses,
        current_tick: row.current_tick,
        interest_rate: row.interest_rate,
    }
}

async fn load_portfolio(pool: &sqlx::PgPool, player_id: i32) -> Result<Portfolio> {
    let position_rows = db::load_positions(pool, player_id).await?;
    let mut portfolio = Portfolio::default();
    for p in position_rows {
        portfolio.positions.insert(
            p.instrument_id,
            sim_core::Position {
                instrument_id: p.instrument_id,
                quantity: p.quantity,
                avg_cost: p.avg_cost,
            },
        );
    }
    Ok(portfolio)
}

/// Bank savings used to be a tradable "instrument" (SAVE) with a price chart,
/// which made a bank account look like a stock. It's now only the savings
/// account. Saves that still hold the old instrument get its value moved
/// into the account, and any open orders on it are dropped.
async fn retire_savings_holdings(
    pool: &sqlx::PgPool,
    player: &Player,
    portfolio: &mut Portfolio,
    orders: &mut Vec<sim_core::LimitOrder>,
    game: &mut GameState,
) -> Result<()> {
    let ids: Vec<i32> = db::list_instruments(pool)
        .await?
        .into_iter()
        .filter(|r| r.class == "savings")
        .map(|r| r.id)
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let prices = db::latest_prices(pool, player.id).await?;
    let mut value = 0.0;
    for id in &ids {
        if let Some(p) = portfolio.positions.remove(id) {
            value += p.quantity * prices.get(id).copied().unwrap_or(1.0);
        }
    }
    orders.retain(|o| !ids.contains(&o.instrument_id));
    if game.absorb_retired_savings(value, player.current_tick) > 0.0 {
        println!("Your old High-Yield Savings holding ({value:.0}) is now in your bank savings account.");
    }
    Ok(())
}

async fn load_instrument_states(pool: &sqlx::PgPool, player_id: i32) -> Result<Vec<InstrumentState>> {
    let instruments = db::list_instruments(pool).await?;
    let prices = db::latest_prices(pool, player_id).await?;

    Ok(instruments
        .into_iter()
        // Bank savings is an account (m), not a tradable instrument.
        .filter(|i| i.class != "savings")
        .map(|i| {
            let class = AssetClass::from_db_str(&i.class);
            let price = prices.get(&i.id).copied().unwrap_or(i.base_price);
            InstrumentState {
                instrument: Instrument {
                    id: i.id,
                    symbol: i.symbol,
                    name: i.name,
                    class,
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
                price,
            }
        })
        .collect())
}

/// Fast-forwards the simulation to account for real time elapsed since
/// this save was last closed - the "catch-up" mechanic. Same
/// `fast_forward` function as live ticking and manual skip. Scoped to
/// this save's own `last_tick_at`, so multiple saves catch up
/// independently rather than sharing one global clock.
async fn catch_up(
    pool: &sqlx::PgPool,
    player: &mut Player,
    last_tick_at: &mut DateTime<Utc>,
    instruments: &mut [InstrumentState],
    portfolio: &mut Portfolio,
    market: &mut Market,
    game: &mut GameState,
) -> Result<()> {
    let now = Utc::now();
    let elapsed = ticks_elapsed_since(*last_tick_at, now, tick_seconds() as i64);
    let ticks = elapsed.min(max_away_ticks());
    if elapsed > ticks {
        println!(
            "You were away for about {} days of game time. The world only catches up on the most recent {ticks} (MARKETFORGE_MAX_AWAY_TICKS).",
            elapsed
        );
    }

    if ticks > 0 {
        println!("Catching up {ticks} day(s) since this save was last played...");
        let news_before = game.news.len();
        // While you were away your personal life is paused (no pay, bills,
        // reviews or personal events); markets, the economy and your
        // investments keep going. MARKETFORGE_OFFLINE_PAY=1 restores the
        // old behavior of living your whole life while the app is closed.
        let freeze = std::env::var("MARKETFORGE_OFFLINE_PAY").map_or(true, |v| v.trim() != "1");
        let worth = |p: &Player, pf: &Portfolio, inst: &[InstrumentState], g: &GameState| {
            let prices: HashMap<i32, f64> = inst.iter().map(|s| (s.instrument.id, s.price)).collect();
            p.cash + pf.market_value(&prices) + g.assets_value(&prices)
        };
        let before = worth(player, portfolio, instruments, game);
        game.away = freeze;
        let out = fast_forward(player, instruments, portfolio, market, game, ticks);
        game.away = false;
        if freeze {
            let after = worth(player, portfolio, instruments, game);
            println!(
                "While you were away your salary and bills were paused. The markets kept moving: net worth {:.0} -> {:.0} ({:+.1}%).",
                before,
                after,
                if before.abs() > 1.0 { (after - before) / before.abs() * 100.0 } else { 0.0 }
            );
        }
        db::insert_price_points(pool, player.id, &out.price_points).await?;
        db::insert_net_worth_points(pool, player.id, &out.net_worth_points).await?;
        *last_tick_at = now;
        db::save_state(pool, player, *last_tick_at, portfolio, &market.orders, &mut *game, &out.fills).await?;
        println!("Caught up. Current tick: {}", player.current_tick);
        if !out.fills.is_empty() {
            println!("{} resting order(s) filled while you were away.", out.fills.len());
        }
        if !out.cancelled_orders.is_empty() {
            println!("{} resting order(s) were cancelled (could no longer be funded).", out.cancelled_orders.len());
        }
        let missed = game.news.len().saturating_sub(news_before);
        if missed > 0 {
            println!("While you were away ({missed} news item(s); the last few):");
            for n in game.news.iter().rev().take(5).rev() {
                println!("  [{}] {}", n.tick, n.text);
            }
        }
        if let sim_core::GameStatus::Lost { reason, .. } = &game.status {
            println!("GAME OVER: {reason}");
        }
        println!();
    }

    Ok(())
}

async fn build_app(
    pool: &sqlx::PgPool,
    save_name: String,
    player: Player,
    last_tick_at: DateTime<Utc>,
    instruments: Vec<InstrumentState>,
    portfolio: Portfolio,
    market: Market,
    game: GameState,
) -> Result<App> {
    let mut history = HashMap::new();
    for s in &instruments {
        let recent = db::recent_prices(pool, player.id, s.instrument.id, HISTORY_LEN as i64).await?;
        history.insert(
            s.instrument.id,
            recent.into_iter().map(|(_, price)| price).collect(),
        );
    }

    let net_worth_history = db::recent_net_worth(pool, player.id, HISTORY_LEN as i64)
        .await?
        .into_iter()
        .map(|(_, nw)| nw)
        .collect();

    let fills = db::load_recent_fills(pool, player.id, app::FILLS_LEN as i64).await?;

    let cashflow = db::recent_cashflow(pool, player.id, 8).await?;
    let ledger_totals = db::category_totals(pool, player.id).await?;
    let transactions = db::recent_transactions(pool, player.id, 400).await?;

    Ok(App {
        save_name,
        player,
        last_tick_at,
        instruments,
        portfolio,
        market,
        modal_visible: game.has_prompt(),
        show_help: false,
        next_auto_tick: std::time::Instant::now() + Duration::from_secs(tick_seconds()),
        cashflow,
        ledger_totals,
        transactions,
        log_scroll: 0,
        manual_selected: 0,
        prev_tab: Tab::Market,
        game,
        fills,
        table_state: ratatui::widgets::TableState::default(),
        history,
        net_worth_history,
        chart_mode: ChartMode::InstrumentPrice,
        tab: Tab::Market,
        selected: 0,
        input_mode: InputMode::Normal,
        input_buffer: String::new(),
        pick: 0,
        status: "Welcome! Press 1-5 to switch screens, ? for help. Goal: reach financial independence.".to_string(),
        should_quit: false,
    })
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    pool: &sqlx::PgPool,
    app: &mut App,
) -> Result<()> {
    let mut events = EventStream::new();
    // One simulated day passes every tick_seconds() of play (default 60): a
    // 30-day month is half an hour. Set MARKETFORGE_TICK_SECONDS to change it.
    let mut ticker = interval(Duration::from_secs(tick_seconds()));
    ticker.tick().await; // interval fires immediately on first poll - consume that so we don't double-advance on startup
    // Redraw once a second so the "next day in Ns" countdown stays live.
    let mut redraw = interval(Duration::from_secs(1));
    redraw.tick().await;

    loop {
        terminal.draw(|f| ui::draw(f, app))?;

        // EventStream + an interval timer, combined with select! - the
        // interval fires a tick automatically every tick_seconds()
        // even if the player isn't pressing anything, while key events
        // are still handled the moment they arrive. Replaces the old
        // blocking poll()/read() pair, which had no way to also react
        // to a timer without either busy-polling or leaving the app
        // static until a keypress.
        tokio::select! {
            maybe_event = events.next() => {
                if let Some(Ok(Event::Key(key))) = maybe_event {
                    if key.kind == KeyEventKind::Press {
                        handle_key(pool, app, key.code).await?;
                    }
                }
            }
            _ = ticker.tick() => {
                app.next_auto_tick = std::time::Instant::now() + Duration::from_secs(tick_seconds());
                advance(pool, app, 1, false).await?;
            }
            _ = redraw.tick() => {}
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

async fn handle_key(pool: &sqlx::PgPool, app: &mut App, code: KeyCode) -> Result<()> {
    // Help overlay: any key closes it.
    if app.show_help {
        app.show_help = false;
        return Ok(());
    }
    // A pending decision or negotiation captures the keyboard.
    if app.game.has_prompt() && app.modal_visible {
        return handle_prompt_key(pool, app, code).await;
    }
    // Once the game is lost the save is read-only.
    if app.game.status.is_lost() {
        match code {
            KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
            KeyCode::Char('w') => app.next_view(),
            KeyCode::Char(c @ '1'..='7') => {
                app.set_tab(Tab::ALL[c as usize - '1' as usize]);
                app.log_scroll = 0;
            }
            KeyCode::Up | KeyCode::Char('k') => app.select_prev(),
            KeyCode::Down | KeyCode::Char('j') => app.select_next(),
            _ => app.status = "Game over. The save is frozen; w to browse, q to quit.".to_string(),
        }
        return Ok(());
    }
    // Trading keys bring you to the Market tab, so you see the result.
    if matches!(app.input_mode, InputMode::Normal)
        && matches!(code, KeyCode::Char('b' | 's' | 'l' | 'x' | 'f'))
        && app.tab != Tab::Market
    {
        app.set_tab(Tab::Market);
    }
    match app.input_mode {
        InputMode::Normal => match code {
            KeyCode::Char('?') => app.show_help = true,
            // The finance manual: browse terms, jump categories, Esc goes back.
            KeyCode::Up | KeyCode::Char('k') if app.tab == Tab::Manual => app.manual_move(-1),
            KeyCode::Down | KeyCode::Char('j') if app.tab == Tab::Manual => app.manual_move(1),
            KeyCode::PageUp if app.tab == Tab::Manual => app.manual_move(-8),
            KeyCode::PageDown if app.tab == Tab::Manual => app.manual_move(8),
            KeyCode::Left | KeyCode::Char('[') if app.tab == Tab::Manual => {
                app.manual_selected = manual::jump_category(app.manual_selected, false)
            }
            KeyCode::Right | KeyCode::Char(']') if app.tab == Tab::Manual => {
                app.manual_selected = manual::jump_category(app.manual_selected, true)
            }
            KeyCode::Home if app.tab == Tab::Manual => app.manual_selected = 0,
            KeyCode::Esc if app.tab == Tab::Manual => {
                let back = app.prev_tab;
                app.set_tab(back);
            }
            KeyCode::Char('i') => app.open_manual(),
            KeyCode::Up | KeyCode::Char('k') if app.tab == Tab::Logs => app.scroll_log(-1),
            KeyCode::Down | KeyCode::Char('j') if app.tab == Tab::Logs => app.scroll_log(1),
            KeyCode::PageUp if app.tab == Tab::Logs => app.scroll_log(-15),
            KeyCode::PageDown if app.tab == Tab::Logs => app.scroll_log(15),
            KeyCode::Char('h') => {
                app.set_tab(Tab::Deals);
                app.chart_mode = ChartMode::Money;
                app.input_mode = InputMode::HousingMenu;
            }
            KeyCode::Char(c @ '1'..='7') => {
                app.set_tab(Tab::ALL[c as usize - '1' as usize]);
                app.log_scroll = 0;
            }
            KeyCode::Char('a') => {
                app.set_tab(Tab::Deals);
                app.chart_mode = ChartMode::Money;
                app.input_mode = InputMode::AssetMenu;
            }
            KeyCode::Char('m') => {
                app.set_tab(Tab::Deals);
                app.chart_mode = ChartMode::Money;
                app.input_mode = InputMode::MoneyMenu;
            }
            KeyCode::Tab | KeyCode::Char('p') if app.game.has_prompt() => app.modal_visible = true,
            KeyCode::Char('c') => {
                app.set_tab(Tab::Life);
                app.input_mode = InputMode::CareerMenu;
                app.status = "Career: 1 course, 2 job hunt".to_string();
            }
            KeyCode::Char('v') => {
                app.set_tab(Tab::Deals);
                app.chart_mode = ChartMode::Deals;
                app.input_mode = InputMode::BusinessMenu;
            }
            KeyCode::Char('f') => {
                if let Some(state) = app.selected_instrument() {
                    let instrument_id = state.instrument.id;
                    app.input_mode = InputMode::ForwardSide { instrument_id };
                }
            }
            KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
            KeyCode::Up | KeyCode::Char('k') => app.select_prev(),
            KeyCode::Down | KeyCode::Char('j') => app.select_next(),
            KeyCode::Char('b') => start_trade_input(app, true),
            KeyCode::Char('s') => start_trade_input(app, false),
            KeyCode::Char('l') => {
                if let Some(state) = app.selected_instrument() {
                    let instrument_id = state.instrument.id;
                    app.input_mode = InputMode::LimitSide { instrument_id };
                    app.input_buffer.clear();
                }
            }
            KeyCode::Char('x') => cancel_orders(pool, app).await?,
            KeyCode::Char('t') => advance(pool, app, 1, false).await?,
            KeyCode::Char('w') => {
                app.next_view();
                app.log_scroll = 0;
                if app.chart_mode == ChartMode::Report {
                    refresh_ledger_views(pool, app).await?;
                }
            }
            KeyCode::Char('n') => {
                app.input_mode = InputMode::SkipTicks;
                app.input_buffer.clear();
            }
            _ => {}
        },
        InputMode::Quantity { instrument_id, is_buy } => match code {
            KeyCode::Char(c) if c.is_ascii_digit() || c == '.' => app.input_buffer.push(c),
            KeyCode::Backspace => {
                app.input_buffer.pop();
            }
            KeyCode::Enter => {
                match app.input_buffer.parse::<f64>() {
                    Ok(qty) if qty > 0.0 => {
                        execute_market_order(pool, app, instrument_id, is_buy, qty).await?;
                    }
                    Ok(_) => app.status = "Enter a positive quantity".to_string(),
                    Err(_) => app.status = "Invalid quantity".to_string(),
                }
                app.input_mode = InputMode::Normal;
                app.input_buffer.clear();
            }
            KeyCode::Esc => cancel_input(app),
            _ => {}
        },
        InputMode::LimitSide { instrument_id } => match code {
            KeyCode::Char('b') => begin_limit_quantity(app, instrument_id, true),
            KeyCode::Char('s') => begin_limit_quantity(app, instrument_id, false),
            KeyCode::Esc => cancel_input(app),
            _ => {}
        },
        InputMode::LimitQuantity { instrument_id, is_buy } => match code {
            KeyCode::Char(c) if c.is_ascii_digit() || c == '.' => app.input_buffer.push(c),
            KeyCode::Backspace => {
                app.input_buffer.pop();
            }
            KeyCode::Enter => match app.input_buffer.parse::<f64>() {
                Ok(qty) if qty > 0.0 => {
                    app.input_mode = InputMode::LimitPrice { instrument_id, is_buy, qty };
                    app.input_buffer.clear();
                    if let Some(book) = app.market.books.get(&instrument_id) {
                        app.status = format!(
                            "Best bid {} / best ask {} - type your limit price, Enter to place",
                            book.best_bid().map_or("-".to_string(), fmt_price),
                            book.best_ask().map_or("-".to_string(), fmt_price),
                        );
                    }
                }
                Ok(_) => app.status = "Enter a positive quantity".to_string(),
                Err(_) => app.status = "Invalid quantity".to_string(),
            },
            KeyCode::Esc => cancel_input(app),
            _ => {}
        },
        InputMode::LimitPrice { instrument_id, is_buy, qty } => match code {
            KeyCode::Char(c) if c.is_ascii_digit() || c == '.' => app.input_buffer.push(c),
            KeyCode::Backspace => {
                app.input_buffer.pop();
            }
            KeyCode::Enter => {
                match app.input_buffer.parse::<f64>() {
                    Ok(price) if price > 0.0 => {
                        place_limit_order(pool, app, instrument_id, is_buy, qty, price).await?;
                    }
                    Ok(_) => app.status = "Enter a positive price".to_string(),
                    Err(_) => app.status = "Invalid price".to_string(),
                }
                app.input_mode = InputMode::Normal;
                app.input_buffer.clear();
            }
            KeyCode::Esc => cancel_input(app),
            _ => {}
        },
        InputMode::CareerMenu => {
            let free = app.free_cash();
            let result = match code {
                KeyCode::Char('1') => Some(app.game.train(&mut app.player, free)),
                KeyCode::Char('2') => Some(app.game.toggle_job_hunt()),
                KeyCode::Esc => {
                    cancel_input(app);
                    None
                }
                _ => None,
            };
            if let Some(r) = result {
                app.status = r.unwrap_or_else(|e| e);
                app.input_mode = InputMode::Normal;
                persist(pool, app, &[]).await?;
            }
        }
        InputMode::MoneyMenu => match code {
            KeyCode::Char('1') => {
                app.input_mode = InputMode::Borrow;
                app.input_buffer.clear();
            }
            KeyCode::Char('2') => {
                if app.game.loan.is_none() {
                    app.status = "You don't owe anything.".to_string();
                    app.input_mode = InputMode::Normal;
                } else {
                    app.input_mode = InputMode::Repay;
                    app.input_buffer.clear();
                }
            }
            KeyCode::Char('3') => {
                app.input_mode = InputMode::Deposit;
                app.input_buffer.clear();
            }
            KeyCode::Char('4') => {
                app.input_mode = InputMode::Withdraw;
                app.input_buffer.clear();
            }
            KeyCode::Esc => cancel_input(app),
            _ => {}
        },
        InputMode::Borrow | InputMode::Repay | InputMode::Deposit | InputMode::Withdraw => match code {
            KeyCode::Char(c) if c.is_ascii_digit() || c == '.' => app.input_buffer.push(c),
            KeyCode::Backspace => {
                app.input_buffer.pop();
            }
            KeyCode::Enter => {
                let mode = app.input_mode;
                match app.input_buffer.parse::<f64>() {
                    Ok(amount) => {
                        let free = app.free_cash();
                        let result = match mode {
                            InputMode::Borrow => app.game.borrow(&mut app.player, amount),
                            InputMode::Deposit => app.game.deposit(&mut app.player, amount, free),
                            InputMode::Withdraw => app.game.withdraw(&mut app.player, amount),
                            _ => app.game.repay(&mut app.player, amount, free),
                        };
                        match result {
                            Ok(msg) => {
                                app.status = msg;
                                persist(pool, app, &[]).await?;
                            }
                            Err(e) => app.status = e,
                        }
                    }
                    Err(_) => app.status = "Enter an amount".to_string(),
                }
                app.input_mode = InputMode::Normal;
                app.input_buffer.clear();
            }
            KeyCode::Esc => cancel_input(app),
            _ => {}
        },
        InputMode::HousingMenu => match code {
            KeyCode::Char('1') => {
                if app.game.home_listings.is_empty() {
                    app.status = "Nothing is for sale right now.".to_string();
                    app.input_mode = InputMode::Normal;
                } else {
                    app.pick = 0;
                    app.input_mode = InputMode::BuyHome;
                }
            }
            KeyCode::Char('2') => {
                if app.game.home.is_none() {
                    app.status = "You don't own a home.".to_string();
                    app.input_mode = InputMode::Normal;
                } else {
                    app.input_mode = InputMode::ConfirmSellHome;
                }
            }
            KeyCode::Esc => cancel_input(app),
            _ => {}
        },
        InputMode::BuyHome => match code {
            KeyCode::Up | KeyCode::Char('k') => pick_move(app, -1),
            KeyCode::Down | KeyCode::Char('j') => pick_move(app, 1),
            _ if pick_index(code, app.pick).is_some() => {
                let index = pick_index(code, app.pick).unwrap_or(0);
                app.input_mode = InputMode::Normal;
                let free = app.free_cash();
                match app.game.start_buy_home(&app.player, free, index, &mut rand::thread_rng()) {
                    Ok(msg) => {
                        app.status = msg;
                        app.modal_visible = true;
                        app.input_buffer.clear();
                        persist(pool, app, &[]).await?;
                    }
                    Err(e) => app.status = e,
                }
            }
            KeyCode::Esc => cancel_input(app),
            _ => {}
        },
        InputMode::ConfirmSellHome => match code {
            KeyCode::Char('y') => {
                app.input_mode = InputMode::Normal;
                match app.game.sell_home(&mut app.player) {
                    Ok(msg) => {
                        app.status = msg;
                        persist(pool, app, &[]).await?;
                    }
                    Err(e) => app.status = e,
                }
            }
            _ => cancel_input(app),
        },
        InputMode::AssetMenu => match code {
            KeyCode::Char('1') => {
                app.pick = 0;
                app.input_mode = InputMode::BuyAsset;
            }
            KeyCode::Char('2') => {
                if app.game.assets.is_empty() {
                    app.status = "You don't own any assets yet.".to_string();
                    app.input_mode = InputMode::Normal;
                } else {
                    app.pick = 0;
                    app.input_mode = InputMode::SellAsset;
                }
            }
            KeyCode::Char('3') => {
                if app.game.assets.iter().any(|a| a.mortgage.is_some()) {
                    app.pick = 0;
                    app.input_mode = InputMode::PrepayAssetPick;
                } else {
                    app.status = "None of your assets has a mortgage.".to_string();
                    app.input_mode = InputMode::Normal;
                }
            }
            KeyCode::Esc => cancel_input(app),
            _ => {}
        },
        InputMode::BuyAsset => match code {
            KeyCode::Up | KeyCode::Char('k') => pick_move(app, -1),
            KeyCode::Down | KeyCode::Char('j') => pick_move(app, 1),
            _ if pick_index(code, app.pick).is_some() => {
                let kind = pick_index(code, app.pick).unwrap_or(0);
                if kind >= sim_core::ASSET_SPECS.len() {
                    app.status = "No such asset.".to_string();
                    app.input_mode = InputMode::Normal;
                } else if sim_core::ASSET_SPECS[kind].financeable {
                    app.input_mode = InputMode::BuyAssetFinance { kind };
                } else {
                    buy_asset_now(pool, app, kind, false).await?;
                }
            }
            KeyCode::Esc => cancel_input(app),
            _ => {}
        },
        InputMode::BuyAssetFinance { kind } => match code {
            KeyCode::Char('1') => buy_asset_now(pool, app, kind, true).await?,
            KeyCode::Char('2') => buy_asset_now(pool, app, kind, false).await?,
            KeyCode::Esc => cancel_input(app),
            _ => {}
        },
        InputMode::SellAsset => match code {
            KeyCode::Up | KeyCode::Char('k') => pick_move(app, -1),
            KeyCode::Down | KeyCode::Char('j') => pick_move(app, 1),
            _ if pick_index(code, app.pick).is_some() => {
                let index = pick_index(code, app.pick).unwrap_or(0);
                if index < app.game.assets.len() {
                    app.input_mode = InputMode::ConfirmSellAsset { index };
                } else {
                    app.status = "No such asset.".to_string();
                    app.input_mode = InputMode::Normal;
                }
            }
            KeyCode::Esc => cancel_input(app),
            _ => {}
        },
        InputMode::ConfirmSellAsset { index } => match code {
            KeyCode::Char('y') => {
                app.input_mode = InputMode::Normal;
                match app.game.sell_asset(&mut app.player, index) {
                    Ok(msg) => {
                        app.status = msg;
                        persist(pool, app, &[]).await?;
                    }
                    Err(e) => app.status = e,
                }
            }
            _ => cancel_input(app),
        },
        InputMode::PrepayAssetPick => match code {
            KeyCode::Up | KeyCode::Char('k') => pick_move(app, -1),
            KeyCode::Down | KeyCode::Char('j') => pick_move(app, 1),
            _ if pick_index(code, app.pick).is_some() => {
                let index = pick_index(code, app.pick).unwrap_or(0);
                if app.game.assets.get(index).is_some_and(|a| a.mortgage.is_some()) {
                    app.input_mode = InputMode::PrepayAssetAmount { index };
                    app.input_buffer.clear();
                } else {
                    app.status = "That asset has no mortgage.".to_string();
                    app.input_mode = InputMode::Normal;
                }
            }
            KeyCode::Esc => cancel_input(app),
            _ => {}
        },
        InputMode::PrepayAssetAmount { index } => match code {
            KeyCode::Char(c) if c.is_ascii_digit() || c == '.' => app.input_buffer.push(c),
            KeyCode::Backspace => {
                app.input_buffer.pop();
            }
            KeyCode::Enter => {
                match app.input_buffer.parse::<f64>() {
                    Ok(amount) => {
                        let free = app.free_cash();
                        match app.game.prepay_asset_mortgage(&mut app.player, free, index, amount) {
                            Ok(msg) => {
                                app.status = msg;
                                persist(pool, app, &[]).await?;
                            }
                            Err(e) => app.status = e,
                        }
                    }
                    Err(_) => app.status = "Enter an amount".to_string(),
                }
                app.input_mode = InputMode::Normal;
                app.input_buffer.clear();
            }
            KeyCode::Esc => cancel_input(app),
            _ => {}
        },
        InputMode::BusinessMenu => match code {
            KeyCode::Char('1') => {
                if app.game.listings.is_empty() {
                    app.status = "Nothing is for sale right now.".to_string();
                    app.input_mode = InputMode::Normal;
                } else {
                    app.pick = 0;
                    app.input_mode = InputMode::BuyBusiness;
                }
            }
            KeyCode::Char('2') | KeyCode::Char('3') => {
                if app.game.businesses.is_empty() {
                    app.status = "You don't own a business yet.".to_string();
                    app.input_mode = InputMode::Normal;
                } else {
                    app.pick = 0;
                    app.input_mode = if code == KeyCode::Char('2') { InputMode::SellBusiness } else { InputMode::UpgradeBusiness };
                }
            }
            KeyCode::Esc => cancel_input(app),
            _ => {}
        },
        InputMode::BuyBusiness | InputMode::SellBusiness | InputMode::UpgradeBusiness => match code {
            KeyCode::Up | KeyCode::Char('k') => pick_move(app, -1),
            KeyCode::Down | KeyCode::Char('j') => pick_move(app, 1),
            _ if pick_index(code, app.pick).is_some() => {
                let index = pick_index(code, app.pick).unwrap_or(0);
                let mode = app.input_mode;
                app.input_mode = InputMode::Normal;
                let free = app.free_cash();
                let mut rng = rand::thread_rng();
                let result = match mode {
                    InputMode::BuyBusiness => app.game.start_buy_business(index, &mut rng),
                    InputMode::SellBusiness => app.game.start_sell_business(index, &mut rng),
                    _ => app.game.upgrade_business(&mut app.player, index, free),
                };
                match result {
                    Ok(msg) => {
                        app.status = msg;
                        if app.game.negotiation.is_some() {
                            app.modal_visible = true;
                            app.input_buffer.clear();
                        }
                        persist(pool, app, &[]).await?;
                    }
                    Err(e) => app.status = e,
                }
            }
            KeyCode::Esc => cancel_input(app),
            _ => {}
        },
        InputMode::ForwardSide { instrument_id } => match code {
            KeyCode::Char('b') => {
                app.input_mode = InputMode::ForwardQty { instrument_id, is_long: true };
                app.input_buffer.clear();
            }
            KeyCode::Char('s') => {
                app.input_mode = InputMode::ForwardQty { instrument_id, is_long: false };
                app.input_buffer.clear();
            }
            KeyCode::Esc => cancel_input(app),
            _ => {}
        },
        InputMode::ForwardQty { instrument_id, is_long } => match code {
            KeyCode::Char(c) if c.is_ascii_digit() || c == '.' => app.input_buffer.push(c),
            KeyCode::Backspace => {
                app.input_buffer.pop();
            }
            KeyCode::Enter => match app.input_buffer.parse::<f64>() {
                Ok(qty) if qty > 0.0 => {
                    app.input_mode = InputMode::ForwardHorizon { instrument_id, is_long, qty };
                    app.input_buffer.clear();
                }
                _ => app.status = "Enter a positive quantity".to_string(),
            },
            KeyCode::Esc => cancel_input(app),
            _ => {}
        },
        InputMode::ForwardHorizon { instrument_id, is_long, qty } => match code {
            KeyCode::Char(c @ '1'..='3') => {
                let horizon = (c as i64 - '0' as i64) * 30;
                app.input_mode = InputMode::Normal;
                let free = app.free_cash();
                let Some(inst) = app.instruments.iter().find(|s| s.instrument.id == instrument_id).cloned() else {
                    return Ok(());
                };
                match app.game.start_forward(&app.player, free, &inst, is_long, qty, horizon, &mut rand::thread_rng()) {
                    Ok(msg) => {
                        app.status = msg;
                        app.modal_visible = true;
                        app.input_buffer.clear();
                        persist(pool, app, &[]).await?;
                    }
                    Err(e) => app.status = e,
                }
            }
            KeyCode::Esc => cancel_input(app),
            _ => {}
        },
        InputMode::SkipTicks => match code {
            KeyCode::Char(c) if c.is_ascii_digit() => app.input_buffer.push(c),
            KeyCode::Backspace => {
                app.input_buffer.pop();
            }
            KeyCode::Enter => {
                match app.input_buffer.parse::<i64>() {
                    Ok(n) if n > 0 => advance(pool, app, n, true).await?,
                    Ok(_) => app.status = "Enter a positive number of days".to_string(),
                    Err(_) => app.status = "Invalid number".to_string(),
                }
                app.input_mode = InputMode::Normal;
                app.input_buffer.clear();
            }
            KeyCode::Esc => cancel_input(app),
            _ => {}
        },
    }
    Ok(())
}

/// Keys while a decision or negotiation pop-up is showing.
async fn handle_prompt_key(pool: &sqlx::PgPool, app: &mut App, code: KeyCode) -> Result<()> {
    if code == KeyCode::Tab {
        app.modal_visible = false;
        app.status = "Prompt hidden. Press Tab or p to bring it back (it still has a deadline).".to_string();
        return Ok(());
    }
    let mut rng = rand::thread_rng();

    if app.game.negotiation.is_some() {
        let free = app.free_cash();
        let step = match code {
            KeyCode::Char(c) if c.is_ascii_digit() || c == '.' => {
                app.input_buffer.push(c);
                None
            }
            KeyCode::Backspace => {
                app.input_buffer.pop();
                None
            }
            KeyCode::Enter => match app.input_buffer.parse::<f64>() {
                Ok(price) => {
                    app.input_buffer.clear();
                    Some(app.game.negotiate_offer(&mut app.player, &app.instruments, free, price, &mut rng))
                }
                Err(_) => {
                    app.status = "Type a price first (or press a to accept their number)".to_string();
                    None
                }
            },
            KeyCode::Char('a') => {
                app.input_buffer.clear();
                Some(app.game.negotiate_accept(&mut app.player, &app.instruments, free, &mut rng))
            }
            KeyCode::Esc => {
                let msg = app.game.negotiate_walk(&app.player);
                app.status = msg;
                app.input_buffer.clear();
                app.modal_visible = false;
                persist(pool, app, &[]).await?;
                return Ok(());
            }
            _ => None,
        };
        match step {
            Some(Ok(NegStep::Continue(msg))) => app.status = msg,
            Some(Ok(NegStep::Done(msg))) => {
                app.status = msg;
                app.modal_visible = false;
                persist(pool, app, &[]).await?;
            }
            Some(Err(e)) => app.status = e,
            None => {}
        }
        return Ok(());
    }

    if app.game.decision.is_some() {
        if let KeyCode::Char(c @ '1'..='9') = code {
            let option = c as usize - '1' as usize;
            match app.game.choose_decision(&mut app.player, option, &mut rng) {
                Ok(msg) => {
                    app.status = msg;
                    app.modal_visible = false;
                    persist(pool, app, &[]).await?;
                }
                Err(e) => app.status = e,
            }
        }
    }
    Ok(())
}

/// A digit picks that row directly; Enter picks the highlighted one.
fn pick_index(code: KeyCode, highlighted: usize) -> Option<usize> {
    match code {
        KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => Some(c as usize - '1' as usize),
        KeyCode::Enter => Some(highlighted),
        _ => None,
    }
}

/// How many rows the current pick list has.
fn pick_len(app: &App) -> usize {
    match app.input_mode {
        InputMode::BuyHome => app.game.home_listings.len(),
        InputMode::BuyBusiness => app.game.listings.len(),
        InputMode::SellBusiness | InputMode::UpgradeBusiness => app.game.businesses.len(),
        InputMode::BuyAsset => sim_core::ASSET_SPECS.len(),
        InputMode::SellAsset | InputMode::PrepayAssetPick => app.game.assets.len(),
        _ => 0,
    }
}

/// Moves the highlight up or down, stopping at the ends.
fn pick_move(app: &mut App, delta: i32) {
    let len = pick_len(app);
    if len == 0 {
        app.pick = 0;
    } else if delta < 0 {
        app.pick = app.pick.saturating_sub(1);
    } else {
        app.pick = (app.pick + 1).min(len - 1);
    }
}

/// Buys the asset at the listed price and saves.
async fn buy_asset_now(pool: &sqlx::PgPool, app: &mut App, kind: usize, financed: bool) -> Result<()> {
    app.input_mode = InputMode::Normal;
    let free = app.free_cash();
    match app.game.buy_asset(&mut app.player, free, kind, financed) {
        Ok(msg) => {
            app.status = msg;
            persist(pool, app, &[]).await?;
        }
        Err(e) => app.status = e,
    }
    Ok(())
}

fn cancel_input(app: &mut App) {
    app.input_mode = InputMode::Normal;
    app.input_buffer.clear();
    app.status = "Cancelled".to_string();
}

fn begin_limit_quantity(app: &mut App, instrument_id: i32, is_buy: bool) {
    let symbol = app.symbol_of(instrument_id);
    app.input_mode = InputMode::LimitQuantity { instrument_id, is_buy };
    app.input_buffer.clear();
    app.status = format!(
        "Limit {} {symbol} - type quantity, Enter to continue, Esc to cancel",
        if is_buy { "buy" } else { "sell" }
    );
}

fn start_trade_input(app: &mut App, is_buy: bool) {
    if let Some(state) = app.selected_instrument() {
        let instrument_id = state.instrument.id;
        let symbol = state.instrument.symbol.clone();
        app.input_mode = InputMode::Quantity { instrument_id, is_buy };
        app.input_buffer.clear();
        app.status = format!(
            "Market {} {symbol} - type quantity, Enter to confirm, Esc to cancel",
            if is_buy { "buy" } else { "sell" }
        );
    }
}

/// Persists everything that can change on a trade or tick batch.
async fn persist(pool: &sqlx::PgPool, app: &mut App, new_fills: &[sim_core::Fill]) -> Result<()> {
    // Gains from trades are taxed at the next payday; collect them now so
    // quitting mid-month can't dodge the tax.
    app.game.absorb_realized(&mut app.portfolio);
    db::save_state(
        pool,
        &app.player,
        app.last_tick_at,
        &app.portfolio,
        &app.market.orders,
        &mut app.game,
        new_fills,
    )
    .await?;
    refresh_ledger_views(pool, app).await
}

/// Reloads the SQL-backed report data (closed months, category totals).
async fn refresh_ledger_views(pool: &sqlx::PgPool, app: &mut App) -> Result<()> {
    app.cashflow = db::recent_cashflow(pool, app.player.id, 8).await?;
    app.ledger_totals = db::category_totals(pool, app.player.id).await?;
    app.transactions = db::recent_transactions(pool, app.player.id, 400).await?;
    Ok(())
}

/// Advances the in-memory sim state directly (no need to reload from
/// the DB first - `app` already holds current prices/player), then
/// persists the result. Backs the 't' single-tick key, the 'n'
/// skip-N-ticks flow, and the live-tick interval. Resting limit orders
/// are batch-cleared inside every tick, and the game layer (bills,
/// events, deals) runs in the same pass. With `interrupt`, a skip stops
/// as soon as a decision or negotiation needs an answer.
async fn advance(pool: &sqlx::PgPool, app: &mut App, ticks: i64, interrupt: bool) -> Result<()> {
    if app.game.status.is_lost() {
        app.status = "Game over. The save is frozen; press q to quit.".to_string();
        return Ok(());
    }
    let had_prompt = app.game.has_prompt();
    let news_before = app.game.news.len();
    let out = fast_forward_with_rng(
        &mut app.player,
        &mut app.instruments,
        &mut app.portfolio,
        &mut app.market,
        &mut app.game,
        ticks,
        interrupt,
        &mut rand::thread_rng(),
    );
    db::insert_price_points(pool, app.player.id, &out.price_points).await?;
    db::insert_net_worth_points(pool, app.player.id, &out.net_worth_points).await?;
    app.last_tick_at = Utc::now();
    persist(pool, app, &out.fills).await?;

    for p in &out.price_points {
        let h = app.history.entry(p.instrument_id).or_default();
        h.push(p.price);
        if h.len() > HISTORY_LEN {
            let excess = h.len() - HISTORY_LEN;
            h.drain(0..excess);
        }
    }

    for p in &out.net_worth_points {
        app.net_worth_history.push(p.net_worth);
    }
    if app.net_worth_history.len() > HISTORY_LEN {
        let excess = app.net_worth_history.len() - HISTORY_LEN;
        app.net_worth_history.drain(0..excess);
    }

    app.record_fills(&out.fills);

    app.status = format!("Advanced to day {}", app.player.current_tick);
    if out.interrupted {
        app.status = format!("Stopped on day {}: something needs your attention", app.player.current_tick);
    }
    let new_news = app.game.news.len().saturating_sub(news_before);
    if new_news > 0 {
        if let Some(n) = app.game.news.last() {
            app.status.push_str(&format!(" | {}", n.text));
        }
        if new_news > 1 {
            app.status.push_str(&format!(" (+{} more in News)", new_news - 1));
        }
    }
    if !out.fills.is_empty() {
        let pnl: f64 = out.fills.iter().map(|f| f.realized_pnl).sum();
        let sells = out.fills.iter().filter(|f| !f.is_buy).count();
        app.status.push_str(&format!(" | {} order fill(s)", out.fills.len()));
        if sells > 0 {
            app.status.push_str(&format!(", realized {pnl:+.0} on sales"));
        }
    }
    if !out.cancelled_orders.is_empty() {
        app.status.push_str(&format!(" | {} order(s) cancelled (unfundable)", out.cancelled_orders.len()));
    }

    // A new decision or negotiation takes over the screen.
    if app.game.has_prompt() && !had_prompt {
        app.modal_visible = true;
        app.input_mode = InputMode::Normal;
        app.input_buffer.clear();
    }
    if !app.game.has_prompt() {
        app.modal_visible = false;
    }
    Ok(())
}

/// For a sale: what you originally paid and what you made on it
/// (before tax), e.g. " | bought at 112.40 avg: profit +58 (+5.1%)".
fn pnl_note(fill: &sim_core::Fill) -> String {
    if fill.is_buy || fill.quantity <= 0.0 {
        return String::new();
    }
    let basis = fill.price - fill.realized_pnl / fill.quantity;
    let pct = if basis > 0.0 { fill.realized_pnl / (basis * fill.quantity) * 100.0 } else { 0.0 };
    format!(
        " | bought at {} avg: {} {:+.0} ({:+.1}%) before tax",
        fmt_price(basis),
        if fill.realized_pnl >= 0.0 { "profit" } else { "loss" },
        fill.realized_pnl,
        pct
    )
}

/// Market order: walks the NPC ladder immediately. Can fill partially
/// when the visible depth runs out, and large orders slip.
async fn execute_market_order(
    pool: &sqlx::PgPool,
    app: &mut App,
    instrument_id: i32,
    is_buy: bool,
    qty: f64,
) -> Result<()> {
    let symbol = app.symbol_of(instrument_id);
    let fair = app
        .instruments
        .iter()
        .find(|s| s.instrument.id == instrument_id)
        .map_or(0.0, |s| s.price);

    let result = app
        .market
        .market_order(&mut app.player, &mut app.portfolio, instrument_id, is_buy, qty);
    match result {
        Ok(fill) => {
            let slip = if fair > 0.0 {
                let signed = (fill.price - fair) / fair * 100.0;
                if is_buy { signed } else { -signed }
            } else {
                0.0
            };
            let partial = if fill.quantity + 1e-9 < qty {
                format!(" (partial: only {} of {} available)", fmt_qty(fill.quantity), fmt_qty(qty))
            } else {
                String::new()
            };
            app.status = format!(
                "{} {} {symbol} @ {} avg ({slip:+.2}% vs fair){partial}{}",
                if is_buy { "Bought" } else { "Sold" },
                fmt_qty(fill.quantity),
                fmt_price(fill.price),
                pnl_note(&fill),
            );
            persist(pool, app, std::slice::from_ref(&fill)).await?;
            app.record_fills(std::slice::from_ref(&fill));
        }
        Err(TradeError::NoLiquidity) => {
            app.status = format!("No liquidity for {symbol} right now - the book is empty on that side until the next day");
        }
        Err(e) => app.status = e.to_string(),
    }
    Ok(())
}

/// Limit order: crosses what it can now, rests the remainder in the
/// book until a later tick's batch clearing fills it.
async fn place_limit_order(
    pool: &sqlx::PgPool,
    app: &mut App,
    instrument_id: i32,
    is_buy: bool,
    qty: f64,
    limit_price: f64,
) -> Result<()> {
    let symbol = app.symbol_of(instrument_id);
    let side = if is_buy { "buy" } else { "sell" };
    let result = app.market.limit_order(
        &mut app.player,
        &mut app.portfolio,
        instrument_id,
        is_buy,
        qty,
        limit_price,
    );
    match result {
        Ok(outcome) => {
            let mut parts = Vec::new();
            let mut new_fills = Vec::new();
            if let Some(fill) = outcome.fill {
                parts.push(format!(
                    "filled {} @ {} avg{}",
                    fmt_qty(fill.quantity),
                    fmt_price(fill.price),
                    pnl_note(&fill)
                ));
                new_fills.push(fill);
            }
            if let Some(order) = &outcome.resting {
                parts.push(format!(
                    "resting {} @ {} (order #{})",
                    fmt_qty(order.quantity),
                    fmt_price(order.limit_price),
                    order.id
                ));
            }
            app.status = format!("Limit {side} {symbol}: {}", parts.join(", "));
            persist(pool, app, &new_fills).await?;
            app.record_fills(&new_fills);
        }
        Err(e) => app.status = e.to_string(),
    }
    Ok(())
}

/// `x`: cancels every resting order on the selected instrument.
async fn cancel_orders(pool: &sqlx::PgPool, app: &mut App) -> Result<()> {
    let Some(state) = app.selected_instrument() else {
        return Ok(());
    };
    let instrument_id = state.instrument.id;
    let symbol = state.instrument.symbol.clone();
    let n = app.market.cancel_orders_for(instrument_id);
    if n == 0 {
        app.status = format!("No open orders on {symbol}");
    } else {
        persist(pool, app, &[]).await?;
        app.status = format!("Cancelled {n} open order(s) on {symbol}");
    }
    Ok(())
}
