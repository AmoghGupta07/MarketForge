use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Axis, Block, Borders, Cell, Chart, Dataset, Gauge, GraphType, Paragraph, Row, Table, Wrap};
use sim_core::{maker_params, AssetClass, FillKind, CAPITAL_GAINS_RATE};
use ratatui::Frame;

use crate::app::{App, ChartMode, InputMode, Tab};
use crate::ui_game;

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(10), Constraint::Length(4)])
        .split(area);

    draw_header(f, chunks[0], app);
    draw_body(f, chunks[1], app);
    draw_footer(f, chunks[2], app);
    if !ui_game::draw_modal(f, app) {
        ui_game::draw_help(f, app);
    }
}

/// Forex-sized prices need 4 decimals; everything else reads fine at 2.
pub fn fmt_price(p: f64) -> String {
    if p < 10.0 {
        format!("{p:.4}")
    } else {
        format!("{p:.2}")
    }
}

/// Whole-dollar amounts with thousands separators: 12,345 / -1,200.
pub fn fmt_money(v: f64) -> String {
    let n = v.round() as i64;
    let digits = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// Quantities can be tiny fractions (crypto), so show up to 4 decimals
/// without trailing zeros.
pub fn fmt_qty(q: f64) -> String {
    let s = format!("{q:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() { "0".to_string() } else { s.to_string() }
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    // Line 1: the five tabs, the active one highlighted.
    let mut tabs: Vec<Span> = vec![Span::raw(" ")];
    for (i, t) in Tab::ALL.iter().enumerate() {
        let style = if *t == app.tab {
            Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        tabs.push(Span::styled(format!(" {} {} ", i + 1, t.name()), style));
        tabs.push(Span::raw(" "));
    }

    // Line 2: the numbers that matter, and only the warnings that apply.
    let nw = app.net_worth();
    let day = app.player.current_tick;
    let mut stats: Vec<Span> = vec![
        Span::styled(format!(" Day {day} "), Style::default().fg(Color::DarkGray)),
        Span::raw("\u{b7} Cash "),
        Span::styled(fmt_money(app.player.cash), Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("  Net worth "),
        Span::styled(fmt_money(nw), Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(format!("  Rate {:.1}%", app.player.interest_rate * 100.0)),
        Span::styled(
            format!("  next day in {}s", app.next_auto_tick.saturating_duration_since(std::time::Instant::now()).as_secs()),
            Style::default().fg(Color::DarkGray),
        ),
        ui_game::status_span(app),
    ];
    let chip = |text: String, color: Color| Span::styled(format!(" {text} "), Style::default().fg(Color::Black).bg(color));
    if app.game.loan.is_some() {
        stats.push(chip(format!("Debt {}", fmt_money(app.game.debt())), Color::Yellow));
    }
    if !app.market.orders.is_empty() {
        stats.push(Span::raw(format!(
            "  {} open order(s), {} free",
            app.market.orders.len(),
            fmt_money(app.free_cash())
        )));
    }
    if !app.game.career.employed {
        stats.push(chip("UNEMPLOYED".to_string(), Color::Red));
    }
    if app.game.distress_months > 0 {
        stats.push(chip(format!("BILLS {}/{}", app.game.distress_months, sim_core::DISTRESS_LIMIT), Color::Red));
    }
    if app.game.has_prompt() && !app.modal_visible {
        stats.push(chip("PROMPT WAITING (Tab)".to_string(), Color::Magenta));
    }

    let header = Paragraph::new(vec![Line::from(tabs), Line::from(stats)]).block(
        Block::default().borders(Borders::ALL).title(format!(" MarketForge - {} ", app.save_name)),
    );
    f.render_widget(header, area);
}

fn draw_body(f: &mut Frame, area: Rect, app: &mut App) {
    match app.tab {
        Tab::Market => {
            // Wide terminals: list | detail. Narrow ones: stacked.
            let direction = if area.width >= 100 { Direction::Horizontal } else { Direction::Vertical };
            let parts = Layout::default()
                .direction(direction)
                .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
                .split(area);
            draw_instrument_table(f, parts[0], app);
            draw_chart(f, parts[1], app);
        }
        Tab::Portfolio => {
            let direction = if area.width >= 100 { Direction::Horizontal } else { Direction::Vertical };
            let parts = Layout::default()
                .direction(direction)
                .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
                .split(area);
            draw_portfolio_table(f, parts[0], app);
            draw_chart(f, parts[1], app);
        }
        // Everything else gets the whole screen to itself.
        _ => draw_chart(f, area, app),
    }
}

/// What an instrument pays you to hold it, in the terms that fit its type:
/// dividend yield for stocks, rent for property, current yield for bonds,
/// the annual fee for funds. Commodities, currencies and crypto pay nothing.
fn income_label(i: &sim_core::Instrument, price: f64) -> String {
    match i.class {
        AssetClass::Bond if price > 0.0 => format!("{:.1}% yld", i.face_value * i.coupon_rate / price * 100.0),
        AssetClass::RealEstate => format!("{:.1}% rent", i.income_yield * 100.0),
        AssetClass::MutualFund => format!("{:.2}% fee", i.expense_ratio * 100.0),
        AssetClass::Equity | AssetClass::IndexFund if i.income_yield > 0.0 => format!("{:.1}% div", i.income_yield * 100.0),
        _ => "no income".to_string(),
    }
}

fn draw_instrument_table(f: &mut Frame, area: Rect, app: &mut App) {
    let header = Row::new(vec!["Symbol", "Class", "Price", "Income", "Held"]).style(Style::default().add_modifier(Modifier::BOLD));
    let rows: Vec<Row> = app
        .instruments
        .iter()
        .map(|s| {
            Row::new(vec![
                Cell::from(s.instrument.symbol.clone()),
                Cell::from(s.instrument.class.short_label()),
                Cell::from(fmt_price(s.price)),
                Cell::from(income_label(&s.instrument, s.price)).style(Style::default().fg(Color::Gray)),
                Cell::from(match app.portfolio.quantity_of(s.instrument.id) {
                    q if q > 0.0 => fmt_qty(q),
                    _ => String::new(),
                })
                .style(Style::default().fg(Color::Green)),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [Constraint::Length(8), Constraint::Length(9), Constraint::Length(10), Constraint::Length(11), Constraint::Min(5)],
    )
    .header(header)
    .row_highlight_style(Style::default().bg(Color::Blue).fg(Color::White))
    .block(Block::default().borders(Borders::ALL).title("Instruments (\u{2191}/\u{2193} select, you hold the green ones)"));

    app.table_state.select(Some(app.selected));
    f.render_stateful_widget(table, area, &mut app.table_state);
}

fn draw_portfolio_table(f: &mut Frame, area: Rect, app: &App) {
    let header =
        Row::new(vec!["Symbol", "Qty", "Bought at", "Value", "Profit"]).style(Style::default().add_modifier(Modifier::BOLD));
    let rows: Vec<Row> = app
        .instruments
        .iter()
        .filter_map(|s| {
            app.portfolio.positions.get(&s.instrument.id).map(|p| {
                let value = p.quantity * s.price;
                let pnl = p.quantity * (s.price - p.avg_cost);
                let pct = if p.avg_cost > 0.0 { (s.price / p.avg_cost - 1.0) * 100.0 } else { 0.0 };
                Row::new(vec![
                    Cell::from(s.instrument.symbol.clone()),
                    Cell::from(fmt_qty(p.quantity)),
                    Cell::from(fmt_price(p.avg_cost)),
                    Cell::from(fmt_money(value)),
                    Cell::from(format!("{:+} ({:+.1}%)", pnl.round() as i64, pct))
                        .style(Style::default().fg(if pnl >= 0.0 { Color::Green } else { Color::Red })),
                ])
            })
        })
        .collect();

    if app.portfolio.positions.is_empty() {
        f.render_widget(
            Paragraph::new(vec![
                Line::from("You don't hold any investments yet."),
                Line::from(""),
                Line::from("Press 1 for the Market, pick an instrument with the"),
                Line::from("arrow keys and press b to buy. Index funds are a"),
                Line::from("calm place to start; press ? for help."),
            ])
            .block(Block::default().borders(Borders::ALL).title("Holdings")),
            area,
        );
        return;
    }
    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Length(9),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Min(14),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title("Holdings (what you paid, what it's worth, profit if sold)"));
    f.render_widget(table, area);
}

fn draw_chart(f: &mut Frame, area: Rect, app: &App) {
    match app.chart_mode {
        ChartMode::InstrumentPrice => draw_instrument_chart(f, area, app),
        ChartMode::NetWorth => draw_net_worth_chart(f, area, app),
        ChartMode::Allocation => draw_allocation(f, area, app),
        ChartMode::OrderBook => draw_order_book(f, area, app),
        ChartMode::Trades => draw_trades(f, area, app),
        ChartMode::Life => ui_game::draw_life(f, area, app),
        ChartMode::News => ui_game::draw_news(f, area, app),
        ChartMode::Deals => ui_game::draw_deals(f, area, app),
        ChartMode::Report => ui_game::draw_report(f, area, app),
        ChartMode::Economy => ui_game::draw_economy(f, area, app),
        ChartMode::Money => ui_game::draw_money(f, area, app),
        ChartMode::Transactions => ui_game::draw_transactions(f, area, app),
        ChartMode::Manual => ui_game::draw_manual(f, area, app),
    }
}

fn draw_instrument_chart(f: &mut Frame, area: Rect, app: &App) {
    let Some(selected) = app.selected_instrument() else {
        f.render_widget(Block::default().borders(Borders::ALL).title("Price History"), area);
        return;
    };

    let info = instrument_info(app, selected);
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(info.len() as u16 + 2)])
        .split(area);

    let empty = Vec::new();
    let prices = app.history.get(&selected.instrument.id).unwrap_or(&empty);

    render_line_chart(
        f,
        parts[0],
        prices,
        &format!("{} price history (w: next view)", selected.instrument.symbol),
        Color::Green,
    );

    let lines: Vec<Line> = info.into_iter().map(Line::from).collect();
    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(selected.instrument.name.clone())),
        parts[1],
    );
}

/// Class-specific facts about the selected instrument.
fn instrument_info(app: &App, s: &sim_core::InstrumentState) -> Vec<String> {
    let i = &s.instrument;
    let mut lines = Vec::new();
    if let Some(p) = app.portfolio.positions.get(&i.id) {
        let pnl = p.quantity * (s.price - p.avg_cost);
        lines.push(format!(
            "You hold {} at an average cost of {}: worth {}, {} {}",
            fmt_qty(p.quantity),
            fmt_price(p.avg_cost),
            fmt_money(p.quantity * s.price),
            if pnl >= 0.0 { "up" } else { "down" },
            fmt_money(pnl.abs())
        ));
    }
    match i.class {
        AssetClass::Bond => {
            lines.push(format!(
                "Face {:.0} | Coupon {:.2}% | Duration {:.1}y",
                i.face_value,
                i.coupon_rate * 100.0,
                i.duration
            ));
            lines.push("Price falls when the prevailing rate rises above the coupon. Coupons paid monthly.".to_string());
        }
        AssetClass::MutualFund => {
            let basket: Vec<String> = i
                .constituents
                .iter()
                .map(|c| format!("{} {:.0}%", app.symbol_of(c.instrument_id), c.weight * 100.0))
                .collect();
            lines.push(format!("Basket: {}", basket.join(", ")));
            lines.push(format!(
                "Expense ratio {:.2}%/yr. NAV tracks the basket's total return, net of fees.",
                i.expense_ratio * 100.0
            ));
        }
        AssetClass::RealEstate => {
            lines.push(format!(
                "Rent yield {:.1}%/yr (paid monthly) | Rate sensitivity {:.1}",
                i.income_yield * 100.0,
                i.rate_sensitivity
            ));
            lines.push("Growth slows when interest rates are above their long-run level. Wide spreads, thin depth.".to_string());
        }
        AssetClass::Forex => {
            lines.push(format!(
                "Long-run level {} | Volatility {:.0}%/yr",
                fmt_price(i.base_price),
                i.volatility * 100.0
            ));
            lines.push("Mean-reverting: strays from its long-run level but is pulled back. Tight spreads.".to_string());
        }
        AssetClass::Crypto => {
            lines.push(format!(
                "Drift {:.0}%/yr | Volatility {:.0}%/yr",
                i.drift * 100.0,
                i.volatility * 100.0
            ));
            lines.push("Extreme volatility; thin depth means big orders slip.".to_string());
        }
        _ => {
            lines.push(format!(
                "Drift {:.1}%/yr | Volatility {:.1}%/yr",
                i.drift * 100.0,
                i.volatility * 100.0
            ));
        }
    }
    let p = maker_params(i.class);
    lines.push(format!(
        "Market-maker: {:.2}% half-spread, {} levels x ~{:.0} each",
        p.half_spread * 100.0,
        p.levels,
        p.level_notional
    ));
    lines
}

fn draw_net_worth_chart(f: &mut Frame, area: Rect, app: &App) {
    render_line_chart(
        f,
        area,
        &app.net_worth_history,
        "Net worth over time (w: next view)",
        Color::Cyan,
    );
}

/// Cash + per-asset-class breakdown as stacked gauges - the
/// diversification snapshot, complementing NetWorth's trend-over-time
/// view with a right-now composition view.
fn draw_allocation(f: &mut Frame, area: Rect, app: &App) {
    // Only what you actually have: empty asset classes are noise.
    let slices: Vec<_> = app
        .allocation()
        .into_iter()
        .filter(|s| s.label == "Cash" || s.value.abs() >= 0.5)
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Where your money is (w: net worth over time)");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let constraints: Vec<Constraint> = slices.iter().map(|_| Constraint::Length(1)).collect();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    for (slice, row) in slices.iter().zip(rows.iter()) {
        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(Color::Magenta))
            .ratio(slice.fraction.clamp(0.0, 1.0))
            .label(format!(
                "{:<12} {:>5.1}%  {}",
                slice.label,
                (slice.fraction * 100.0).max(0.0),
                fmt_money(slice.value)
            ));
        f.render_widget(gauge, *row);
    }
}

/// The selected instrument's NPC ladder (asks on top, bids below, spread
/// in between) and the player's resting orders on it.
fn draw_order_book(f: &mut Frame, area: Rect, app: &App) {
    let Some(sel) = app.selected_instrument() else {
        f.render_widget(Block::default().borders(Borders::ALL).title("Order Book"), area);
        return;
    };
    let id = sel.instrument.id;
    let title = format!("{} order book (w: next view)", sel.instrument.symbol);

    let mut lines: Vec<Line> = Vec::new();
    if let Some(book) = app.market.books.get(&id) {
        lines.push(Line::from(Span::styled(
            format!("{:>12} {:>12}", "Price", "Size"),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        for l in book.asks.iter().rev() {
            lines.push(Line::from(Span::styled(
                format!("{:>12} {:>12}", fmt_price(l.price), fmt_qty(l.qty)),
                Style::default().fg(Color::Red),
            )));
        }
        let spread = match (book.best_bid(), book.best_ask()) {
            (Some(b), Some(a)) if sel.price > 0.0 => format!("spread {:.3}%", (a - b) / sel.price * 100.0),
            _ => "no quotes".to_string(),
        };
        lines.push(Line::from(Span::styled(
            format!("{:>12}   fair {}  ({spread})", "-----", fmt_price(sel.price)),
            Style::default().fg(Color::Yellow),
        )));
        for l in &book.bids {
            lines.push(Line::from(Span::styled(
                format!("{:>12} {:>12}", fmt_price(l.price), fmt_qty(l.qty)),
                Style::default().fg(Color::Green),
            )));
        }
    }

    lines.push(Line::from(""));
    let mine: Vec<_> = app.market.orders_for(id).collect();
    if mine.is_empty() {
        lines.push(Line::from("No open orders here. l: limit order, x: cancel orders"));
    } else {
        lines.push(Line::from(Span::styled(
            "Your open orders",
            Style::default().add_modifier(Modifier::BOLD),
        )));
        for o in mine {
            lines.push(Line::from(format!(
                "#{} {} {} @ {}  (since day {})",
                o.id,
                if o.is_buy { "BUY " } else { "SELL" },
                fmt_qty(o.quantity),
                fmt_price(o.limit_price),
                o.placed_tick
            )));
        }
    }

    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(title)),
        area,
    );
}

fn draw_trades(f: &mut Frame, area: Rect, app: &App) {
    let header = Row::new(vec!["Day", "Side", "Symbol", "Qty", "Price", "Profit", "Type"])
        .style(Style::default().add_modifier(Modifier::BOLD));
    let rows: Vec<Row> = app
        .fills
        .iter()
        .rev()
        .skip(app.log_scroll)
        .map(|t| {
            let color = if t.is_buy { Color::Green } else { Color::Red };
            Row::new(vec![
                Cell::from(t.tick.to_string()),
                Cell::from(if t.is_buy { "BUY" } else { "SELL" }).style(Style::default().fg(color)),
                Cell::from(app.symbol_of(t.instrument_id)),
                Cell::from(fmt_qty(t.quantity)),
                Cell::from(fmt_price(t.price)),
                if t.is_buy {
                    Cell::from("")
                } else {
                    Cell::from(format!("{:+.0}", t.realized_pnl))
                        .style(Style::default().fg(if t.realized_pnl >= 0.0 { Color::Green } else { Color::Red }))
                },
                Cell::from(match t.kind {
                    FillKind::Market => "market",
                    FillKind::LimitImmediate => "limit",
                    FillKind::LimitResting => "limit (resting)",
                    FillKind::Forced => "FORCED SALE",
                }),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(7),
            Constraint::Length(5),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(9),
            Constraint::Min(8),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title("Recent trades (arrows scroll; w: transactions)"));
    f.render_widget(table, area);
}

/// Shared rendering for both chart modes - same axis/bounds/empty-state
/// handling, just a different data series, title, and color.
pub fn render_line_chart(f: &mut Frame, area: Rect, values: &[f64], title: &str, color: Color) {
    let points: Vec<(f64, f64)> = values.iter().enumerate().map(|(i, v)| (i as f64, *v)).collect();

    if points.len() < 2 {
        f.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("{title} - not enough history yet")),
            area,
        );
        return;
    }

    let min_y = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let max_y = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    let pad = ((max_y - min_y) * 0.1).max(0.01);

    let datasets = vec![Dataset::default()
        .graph_type(GraphType::Line)
        .style(Style::default().fg(color))
        .data(&points)];

    let chart = Chart::new(datasets)
        .block(Block::default().borders(Borders::ALL).title(title.to_string()))
        .x_axis(Axis::default().bounds([0.0, (points.len() - 1) as f64]))
        .y_axis(
            Axis::default()
                .bounds([min_y - pad, max_y + pad])
                .labels(vec![
                    Line::from(format!("{:.2}", min_y - pad)),
                    Line::from(format!("{:.2}", max_y + pad)),
                ]),
        );

    f.render_widget(chart, area);
}

/// What you paid and what a sale would earn: shown while you type a sell
/// order. `price` is a limit price; without it the order is a market order
/// and the estimate walks the order book.
fn sell_hint(app: &App, instrument_id: i32, qty: Option<f64>, price: Option<f64>) -> String {
    let Some(pos) = app.portfolio.positions.get(&instrument_id) else {
        return "you hold none of this".to_string();
    };
    let held = pos.quantity;
    let cost = pos.avg_cost;
    let mut out = format!("you hold {} bought at {} avg", fmt_qty(held), fmt_price(cost));
    let q = qty.filter(|q| *q > 0.0).map(|q| q.min(held));
    // Estimated sale price: your limit, or the average the order book would pay.
    let est = match (q, price) {
        (_, Some(p)) => Some(p),
        (Some(q), None) => app.market.books.get(&instrument_id).and_then(|b| {
            let (filled, proceeds) = b.clone().take_sell(q, None);
            (filled > 0.0).then(|| proceeds / filled)
        }),
        (None, None) => app.market.books.get(&instrument_id).and_then(|b| b.best_bid()),
    };
    if let Some(p) = est {
        let units = q.unwrap_or(held);
        let profit = (p - cost) * units;
        let pct = if cost > 0.0 { (p / cost - 1.0) * 100.0 } else { 0.0 };
        let after = if profit > 0.0 { profit * (1.0 - CAPITAL_GAINS_RATE) } else { profit };
        out.push_str(&format!(
            " | at ~{} for {}: {} {:+.0} ({:+.1}%), ~{:+.0} after {:.0}% tax",
            fmt_price(p),
            fmt_qty(units),
            if profit >= 0.0 { "profit" } else { "loss" },
            profit,
            pct,
            after,
            CAPITAL_GAINS_RATE * 100.0
        ));
    }
    out
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let text = match &app.input_mode {
        InputMode::Normal => format!(" {} ", app.status),
        InputMode::Quantity { is_buy, instrument_id } => format!(
            " MARKET {} qty> {}_{}",
            if *is_buy { "BUY" } else { "SELL" },
            app.input_buffer,
            if *is_buy { String::new() } else { format!("   [{}]", sell_hint(app, *instrument_id, app.input_buffer.parse().ok(), None)) }
        ),
        InputMode::SkipTicks => format!(" Skip how many days> {}_ ", app.input_buffer),
        InputMode::LimitSide { .. } => " LIMIT order: press b to buy or s to sell (Esc cancels) ".to_string(),
        InputMode::LimitQuantity { is_buy, instrument_id } => format!(
            " LIMIT {} qty> {}_{}",
            if *is_buy { "BUY" } else { "SELL" },
            app.input_buffer,
            if *is_buy { String::new() } else { format!("   [{}]", sell_hint(app, *instrument_id, app.input_buffer.parse().ok(), None)) }
        ),
        InputMode::CareerMenu => {
            format!(" CAREER: 1 take a course (-{:.0}, +1 skill)   2 job hunt (when unemployed)   Esc cancel ", sim_core::TRAINING_COST)
        }
        InputMode::MoneyMenu => format!(
            " MONEY: 1 borrow (up to {:.0} more)   2 repay{}   3 deposit to savings   4 withdraw (saved {:.0})   Esc ",
            app.game.borrowing_room(&app.player),
            if app.game.loan.is_some() { format!(" (owe {:.0})", app.game.debt()) } else { String::new() },
            app.game.savings
        ),
        InputMode::Deposit => format!(" DEPOSIT amount> {}_ (Enter confirms, Esc cancels) ", app.input_buffer),
        InputMode::Withdraw => format!(" WITHDRAW amount> {}_ (savings: {:.0}) ", app.input_buffer, app.game.savings),
        InputMode::Borrow => format!(" BORROW amount> {}_ (Enter confirms, Esc cancels) ", app.input_buffer),
        InputMode::Repay => format!(" REPAY amount> {}_ (you owe {:.0}; Enter confirms) ", app.input_buffer, app.game.debt()),
        InputMode::HousingMenu => format!(
            " HOME: 1 buy a listed home   2 sell yours{}   Esc cancel ",
            if app.game.home.is_some() { String::new() } else { format!(" (you rent at {}/mo)", fmt_money(app.game.rent)) }
        ),
        InputMode::BuyHome => " Buy which home? \u{2191}\u{2193} choose, Enter confirms (or press its number), Esc cancels ".to_string(),
        InputMode::ConfirmSellHome => " Sell your home on the market (6% selling costs)? y to confirm, any other key cancels ".to_string(),
        InputMode::AssetMenu => " ASSETS: 1 buy (rental flat, storage units, solar, car)   2 sell one   3 repay a mortgage   Esc cancel ".to_string(),
        InputMode::BuyAsset => " Buy which asset? \u{2191}\u{2193} choose, Enter confirms (or press its number), Esc cancels ".to_string(),
        InputMode::BuyAssetFinance { kind } => format!(
            " {}: 1 = buy-to-let mortgage ({:.0}% down)   2 = pay cash   Esc cancels ",
            sim_core::ASSET_SPECS[*kind].name,
            sim_core::BTL_DOWN_PAYMENT * 100.0
        ),
        InputMode::SellAsset => " Sell which asset? \u{2191}\u{2193} choose, Enter confirms (or press its number), Esc cancels ".to_string(),
        InputMode::ConfirmSellAsset { index } => format!(
            " Sell the {} at market value less selling costs? y to confirm, any other key cancels ",
            app.game.assets.get(*index).map_or("asset", |a| a.name())
        ),
        InputMode::PrepayAssetPick => " Repay the mortgage on which asset? \u{2191}\u{2193} choose, Enter confirms, Esc cancels ".to_string(),
        InputMode::PrepayAssetAmount { index } => format!(" REPAY asset {} mortgage amount> {}_ (Enter confirms, Esc cancels) ", index + 1, app.input_buffer),
        InputMode::BusinessMenu => " BUSINESS: 1 buy a listed business   2 sell one of yours   3 upgrade one   Esc cancel ".to_string(),
        InputMode::BuyBusiness => " Buy which listing? \u{2191}\u{2193} choose, Enter confirms (or press its number), Esc cancels ".to_string(),
        InputMode::SellBusiness => " Sell which business? \u{2191}\u{2193} choose, Enter confirms (or press its number), Esc cancels ".to_string(),
        InputMode::UpgradeBusiness => " Upgrade which business? \u{2191}\u{2193} choose, Enter confirms (or press its number), Esc cancels ".to_string(),
        InputMode::ForwardSide { .. } => " FORWARD with your broker: b = buy (long), s = sell (short), Esc cancels ".to_string(),
        InputMode::ForwardQty { is_long, .. } => format!(
            " FORWARD {} qty> {}_ ",
            if *is_long { "LONG" } else { "SHORT" },
            app.input_buffer
        ),
        InputMode::ForwardHorizon { qty, .. } => format!(" FORWARD {}: settle in 1 = 30 days, 2 = 60, 3 = 90 ", fmt_qty(*qty)),
        InputMode::LimitPrice { is_buy, qty, instrument_id } => format!(
            " LIMIT {} {} - limit price> {}_{}",
            if *is_buy { "BUY" } else { "SELL" },
            fmt_qty(*qty),
            app.input_buffer,
            if *is_buy { String::new() } else { format!("   [{}]", sell_hint(app, *instrument_id, Some(*qty), app.input_buffer.parse().ok())) }
        ),
    };
    let hints = match app.tab {
        Tab::Market => "1-7 tabs  \u{2191}\u{2193} select  b buy  s sell  l limit  x cancel  f forward  w book  t/n time  i manual  ? help",
        Tab::Portfolio => "1-7 tabs  w chart  t/n time  m money  i manual  ? help",
        Tab::Life => "1-7 tabs  c career  w report  t/n time  i manual  ? help",
        Tab::Deals => "1-7 tabs  v business  h home  a assets  m money  w money/home  t/n time  i manual  ? help",
        Tab::World => "1-7 tabs  t/n time  i manual  ? help",
        Tab::Manual => "1-7 tabs  \u{2191}\u{2193} term  \u{2190}\u{2192} category  PgUp/PgDn jump  Esc back  t/n time  ? help",
        Tab::Logs => "1-7 tabs  \u{2191}\u{2193}/PgUp/PgDn scroll  w transactions/news/trades  t/n time  i manual  ? help",
    };
    let footer = Paragraph::new(text).wrap(Wrap { trim: true }).block(
        Block::default().borders(Borders::ALL).title(format!(" {hints} ")),
    );
    f.render_widget(footer, area);
}
