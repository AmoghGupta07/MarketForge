//! Drawing for the game layer: the Life / News / Deals views and the
//! modal for pending decisions and negotiations.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Gauge, Paragraph, Wrap};
use ratatui::Frame;
use sim_core::{
    decision, GameStatus, NegPurpose, NewsKind, Side, BUSINESS_KINDS, FI_WITHDRAWAL_RATE, LADDER, MAX_SKILL,
    REVIEW_PERIOD_TICKS,
};

use crate::app::{App, InputMode};
use crate::manual;
use crate::ui::{fmt_money, fmt_price, fmt_qty, render_line_chart};

/// Highlights the row the cursor is on while you're choosing from a list.
fn hl<'a>(line: Line<'a>, on: bool) -> Line<'a> {
    if on {
        line.style(Style::default().add_modifier(Modifier::REVERSED))
    } else {
        line
    }
}

fn bold() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

fn kind_color(k: NewsKind) -> Color {
    match k {
        NewsKind::Market => Color::Yellow,
        NewsKind::Personal => Color::Cyan,
        NewsKind::Career => Color::Green,
        NewsKind::Deal => Color::Magenta,
        NewsKind::Social => Color::LightBlue,
        NewsKind::Goal => Color::LightRed,
    }
}

/// One-line game status for the header.
pub fn status_span(app: &App) -> Span<'static> {
    let nw = app.net_worth();
    match &app.game.status {
        GameStatus::Lost { .. } => Span::styled(" GAME OVER ".to_string(), Style::default().fg(Color::White).bg(Color::Red)),
        GameStatus::Won { .. } => Span::styled(" FINANCIALLY INDEPENDENT ".to_string(), Style::default().fg(Color::Black).bg(Color::Green)),
        GameStatus::Playing => Span::raw(format!(
            " FI {:.0}% ",
            (app.game.fi_progress(&app.player, nw) * 100.0).min(999.0)
        )),
    }
}

pub fn draw_life(f: &mut Frame, area: Rect, app: &App) {
    let g = &app.game;
    let nw = app.net_worth();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(11), Constraint::Min(6)])
        .split(area);

    // Career + goals.
    let c = &g.career;
    let mut lines = vec![Line::from(vec![
        Span::styled(c.title().to_string(), bold()),
        Span::raw(format!(
            "  salary {:.0}/mo  expenses {:.0}/mo  skill {:.1}/{MAX_SKILL:.0}",
            app.player.salary, app.player.expenses, c.skill
        )),
    ])];
    if c.employed {
        let to_review = (c.next_review_tick - app.player.current_tick).max(0);
        lines.push(Line::from(format!(
            "Next review in {to_review} days ({:.0}% promotion odds{})",
            c.promotion_chance() * 100.0,
            c.next_title().map_or(String::new(), |(t, s)| format!(" to {t}, {s:.0}/mo"))
        )));
    } else {
        lines.push(Line::from(Span::styled(
            format!(
                "UNEMPLOYED - on benefits{}. Press c to job hunt.",
                if c.job_hunting { " (job hunting)" } else { "" }
            ),
            Style::default().fg(Color::Red),
        )));
    }
    let ladder: Vec<String> = LADDER
        .iter()
        .enumerate()
        .map(|(i, (t, _))| if i == c.title_idx { format!("[{t}]") } else { t.to_string() })
        .collect();
    lines.push(Line::from(Span::styled(ladder.join(" > "), Style::default().fg(Color::DarkGray))));
    lines.push(Line::from(""));
    let costs = g.total_costs(&app.player);
    let passive = (nw - g.non_fi_wealth()) * FI_WITHDRAWAL_RATE / 12.0;
    lines.push(Line::from(format!(
        "Goal: financial independence. Your savings and investments (not your home, car or solar) at a {:.0}% withdrawal rate give {:.0}/mo vs {:.0}/mo of costs.",
        FI_WITHDRAWAL_RATE * 100.0,
        passive,
        costs
    )));
    lines.push(Line::from(format!(
        "Savings and investments needed: {:.0}   |   Your rank: #{} of {}",
        costs * 12.0 / FI_WITHDRAWAL_RATE,
        g.leaderboard(nw).iter().position(|e| e.2).unwrap_or(0) + 1,
        g.rivals.len() + 1
    )));
    if g.distress_months > 0 {
        lines.push(Line::from(Span::styled(
            format!("WARNING: can't cover bills ({}/{})", g.distress_months, sim_core::DISTRESS_LIMIT),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )));
    }
    let top = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(1)])
        .split(rows[0]);
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(Block::default().borders(Borders::ALL).title("Career & goals (w: next view)")),
        Rect { height: rows[0].height, ..rows[0] },
    );
    let ratio = g.fi_progress(&app.player, nw).clamp(0.0, 1.0);
    let _ = top;
    let gauge_area = Rect { x: rows[0].x + 1, y: rows[0].y + rows[0].height - 2, width: rows[0].width.saturating_sub(2), height: 1 };
    f.render_widget(
        Gauge::default()
            .gauge_style(Style::default().fg(Color::Green))
            .ratio(ratio)
            .label(format!("Financial independence: {:.1}%", g.fi_progress(&app.player, nw) * 100.0)),
        gauge_area,
    );

    // Leaderboard.
    let mut lb = vec![Line::from(Span::styled("Leaderboard (net worth)", bold()))];
    for (i, (name, wealth, you)) in g.leaderboard(nw).iter().enumerate() {
        let style = if *you { Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD) } else { Style::default() };
        let style_note = g
            .rivals
            .iter()
            .find(|r| &r.name == name)
            .map_or(String::new(), |r| format!("  ({})", r.style));
        lb.push(Line::from(Span::styled(
            format!("#{} {:<14} {:>12.0}{}", i + 1, name, wealth, style_note),
            style,
        )));
    }
    f.render_widget(
        Paragraph::new(lb).block(Block::default().borders(Borders::ALL).title("Rivals")),
        rows[1],
    );
}

pub fn draw_news(f: &mut Frame, area: Rect, app: &App) {
    let lines: Vec<Line> = app
        .game
        .news
        .iter()
        .rev()
        .skip(app.log_scroll)
        .take(area.height.saturating_sub(2) as usize)
        .map(|n| {
            Line::from(vec![
                Span::styled(format!("[{:>5}] ", n.tick), Style::default().fg(Color::DarkGray)),
                Span::styled(n.text.clone(), Style::default().fg(kind_color(n.kind))),
            ])
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(Block::default().borders(Borders::ALL).title("News (arrows scroll; w: trades)")),
        area,
    );
}

pub fn draw_deals(f: &mut Frame, area: Rect, app: &App) {
    let g = &app.game;
    let now = app.player.current_tick;
    let mut lines: Vec<Line> = vec![];

    lines.push(Line::from(Span::styled("Forward contracts (f on an instrument)", bold())));
    if g.contracts.is_empty() {
        lines.push(Line::from("  none"));
    }
    let prices: std::collections::HashMap<i32, f64> =
        app.instruments.iter().map(|s| (s.instrument.id, s.price)).collect();
    for c in &g.contracts {
        let spot = prices.get(&c.instrument_id).copied().unwrap_or(c.strike);
        let v = c.value_at(spot);
        lines.push(Line::from(vec![
            Span::raw(format!(
                "  {} {} {} @ {} (spot {}), settles in {}: ",
                if c.is_long { "LONG" } else { "SHORT" },
                fmt_qty(c.quantity),
                c.symbol,
                fmt_price(c.strike),
                fmt_price(spot),
                (c.settle_tick - now).max(0)
            )),
            Span::styled(format!("{v:+.0}"), Style::default().fg(if v >= 0.0 { Color::Green } else { Color::Red })),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("Your businesses (last month's profit: {:+.0})", g.last_business_income),
        bold(),
    )));
    if g.businesses.is_empty() {
        lines.push(Line::from("  none. Press v to browse listings."));
    }
    for (i, b) in g.businesses.iter().enumerate() {
        let on = matches!(app.input_mode, InputMode::SellBusiness | InputMode::UpgradeBusiness) && app.pick == i;
        lines.push(hl(Line::from(format!(
            "  {}. {} - ~{:.0}/mo at {:.0}% efficiency, worth {:.0}",
            i + 1,
            b.name,
            b.monthly_profit * b.efficiency,
            b.efficiency * 100.0,
            b.value()
        )), on));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("For sale (v, then 1 to buy)", bold())));
    for (i, l) in g.listings.iter().enumerate() {
        let on = matches!(app.input_mode, InputMode::BuyBusiness) && app.pick == i;
        lines.push(hl(Line::from(format!(
            "  {}. {} - ~{:.0}/mo profit, asking around {:.0}  [{} days left]  owner {}",
            i + 1,
            l.kind().name,
            l.monthly_profit,
            l.fair_price(),
            (l.expires_tick - now).max(0),
            g.npcs[l.kind().owner_npc].name
        )), on));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("People", bold())));
    for n in &g.npcs {
        lines.push(Line::from(format!(
            "  {:<14} {:<24} {:>3.0}/100 ({})",
            n.name,
            n.role,
            n.relationship,
            n.mood()
        )));
    }
    let _ = BUSINESS_KINDS.len();

    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title("Businesses, contracts & people (w: money & home)")),
        area,
    );
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h }
}

/// The pop-up for a pending decision or negotiation. Returns true if it
/// drew something.
pub fn draw_modal(f: &mut Frame, app: &App) -> bool {
    if !app.modal_visible {
        return false;
    }
    let g = &app.game;
    let now = app.player.current_tick;

    if let Some(n) = &g.negotiation {
        let area = centered(f.area(), 78, 20);
        f.render_widget(Clear, area);
        let mut lines: Vec<Line> = vec![];
        let npc = &g.npcs[n.npc];
        lines.push(Line::from(vec![
            Span::styled(format!("{} ", npc.name), bold()),
            Span::raw(format!("({}, {} - relationship {:.0})", npc.role, npc.mood(), npc.relationship)),
        ]));
        lines.push(Line::from(""));
        for entry in n.log.iter().rev().take(6).rev() {
            lines.push(Line::from(entry.clone()));
        }
        lines.push(Line::from(""));
        let verb = match n.neg.side {
            Side::PlayerBuys => "asking",
            Side::PlayerSells => "offering",
        };
        lines.push(Line::from(vec![
            Span::raw(format!("{} is {verb}: ", npc.name)),
            Span::styled(format!("{:.4}", n.neg.counter), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw(format!("   (fair value ~{:.4})   patience: {}", n.neg.fair, "*".repeat(n.neg.patience as usize))),
        ]));
        if let NegPurpose::Salary { .. } = n.purpose {
            if let Some(dl) = n.deadline {
                lines.push(Line::from(format!("The review closes in {} days; their offer stands if you do nothing.", (dl - now).max(0))));
            }
        }
        lines.push(Line::from(""));
        lines.push(Line::from(format!("Your offer> {}_", app.input_buffer)));
        lines.push(Line::from(Span::styled(
            "type a price + Enter: counter-offer   a: accept their number   Esc: walk away   Tab: hide",
            Style::default().fg(Color::DarkGray),
        )));
        f.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: true })
                .block(Block::default().borders(Borders::ALL).title(format!(" {} ", n.title))),
            area,
        );
        return true;
    }

    if let Some(p) = &g.decision {
        if let Some(def) = decision(&p.key) {
            let area = centered(f.area(), 78, (def.options.len() as u16) + 9);
            f.render_widget(Clear, area);
            let mut lines = vec![Line::from(def.body), Line::from("")];
            for (i, o) in def.options.iter().enumerate() {
                lines.push(Line::from(vec![
                    Span::styled(format!(" {} ", i + 1), Style::default().fg(Color::Black).bg(Color::Cyan)),
                    Span::raw(format!(" {}{}", o.label, if i == def.default_option { "  (default)" } else { "" })),
                ]));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!(
                    "Press a number to choose. Decide within {} days or the default happens. Tab: hide",
                    (p.deadline_tick - now).max(0)
                ),
                Style::default().fg(Color::DarkGray),
            )));
            f.render_widget(
                Paragraph::new(lines)
                    .wrap(Wrap { trim: true })
                    .alignment(Alignment::Left)
                    .block(Block::default().borders(Borders::ALL).title(format!(" {} ", def.title))),
                area,
            );
            return true;
        }
    }
    let _ = REVIEW_PERIOD_TICKS;
    false
}

pub fn draw_report(f: &mut Frame, area: Rect, app: &App) {
    let g = &app.game;
    let s = &g.stats;
    let nw = app.net_worth();
    let now = app.player.current_tick;
    let years = now as f64 / 360.0;
    let mut lines: Vec<Line> = vec![];

    let status = match &g.status {
        GameStatus::Playing => "In progress".to_string(),
        GameStatus::Won { tick } => format!("WON: financial independence on day {tick}"),
        GameStatus::Lost { tick, reason } => format!("LOST on day {tick}: {reason}"),
    };
    lines.push(Line::from(vec![Span::styled("Status: ", bold()), Span::raw(status)]));
    lines.push(Line::from(format!("Time played: {now} days (~{years:.1} simulated years)")));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Net worth", bold())));
    lines.push(Line::from(format!(
        "  now {:.0}   peak {:.0} (day {})   {}",
        nw,
        s.peak_net_worth.max(nw),
        s.peak_tick,
        if s.peak_net_worth > nw * 1.05 {
            format!("down {:.0}% from the peak", (1.0 - nw / s.peak_net_worth) * 100.0)
        } else {
            "at or near the peak".to_string()
        }
    )));
    let lb = g.leaderboard(nw);
    lines.push(Line::from(format!(
        "  rank #{} of {}{}",
        lb.iter().position(|e| e.2).unwrap_or(0) + 1,
        lb.len(),
        lb.first().map_or(String::new(), |t| if t.2 { "  (you lead the field)".into() } else { format!("  (leader: {} with {:.0})", t.0, t.1) })
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Money in and out (since the game layer started)", bold())));
    let row = |label: &str, v: f64, color: Color| {
        Line::from(vec![Span::raw(format!("  {label:<26}")), Span::styled(format!("{v:>12.0}"), Style::default().fg(color))])
    };
    lines.push(row("Salary & benefits", s.salary_income, Color::Green));
    lines.push(row("Business profit", s.business_income, if s.business_income >= 0.0 { Color::Green } else { Color::Red }));
    lines.push(row("Living expenses", -s.expenses_paid, Color::Red));
    lines.push(row("Loan interest", -s.interest_paid, Color::Red));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Activity", bold())));
    lines.push(Line::from(format!(
        "  {} decision(s) made   {} deal(s) done   {} forced sale(s)   {} trade(s) on record",
        s.decisions_made,
        s.deals_done,
        s.forced_sales,
        app.fills.len()
    )));
    lines.push(Line::from(format!(
        "  career: {} (skill {:.1})   debt: {:.0}   businesses: {}",
        g.career.title(),
        g.career.skill,
        g.debt(),
        g.businesses.len()
    )));
    lines.push(Line::from(format!(
        "  taxes paid so far: {:.0}   investment income: {:.0}   savings: {:.0}   home costs paid: {:.0}",
        s.taxes_paid, s.investment_income, g.savings, s.housing_costs
    )));
    lines.push(Line::from(format!(
        "  rent received: {:.0}   asset upkeep, running costs and mortgage interest: {:.0}",
        s.rental_income, s.asset_costs
    )));

    // From the SQL ledger: months closed by the close_month() procedure.
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Monthly cash flow (SQL ledger, last closed months)", bold())));
    lines.push(Line::from(Span::styled(
        format!("  {:>6} {:>10} {:>10} {:>9} {:>10}", "month", "income", "expenses", "taxes", "net"),
        Style::default().fg(Color::DarkGray),
    )));
    for m in app.cashflow.iter().rev().take(6).rev() {
        lines.push(Line::from(vec![
            Span::raw(format!("  {:>6} {:>10.0} {:>10.0} {:>9.0} ", m.month, m.income, m.expenses, m.taxes)),
            Span::styled(format!("{:>10.0}", m.net), Style::default().fg(if m.net >= 0.0 { Color::Green } else { Color::Red })),
        ]));
    }
    if app.cashflow.is_empty() {
        lines.push(Line::from("  (nothing closed yet)"));
    }
    if !app.ledger_totals.is_empty() {
        let parts: Vec<String> = app
            .ledger_totals
            .iter()
            .filter(|(c, _)| !matches!(c.as_str(), "trade" | "loan_draw" | "loan_principal" | "deposit" | "withdrawal"))
            .map(|(c, t)| format!("{c} {t:+.0}"))
            .collect();
        lines.push(Line::from(Span::styled(format!("  lifetime: {}", parts.join("  ")), Style::default().fg(Color::DarkGray))));
    }
    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: true }).block(Block::default().borders(Borders::ALL).title("Run report (w: next view)")),
        area,
    );
}

/// Macro indicators and their history.
pub fn draw_economy(f: &mut Frame, area: Rect, app: &App) {
    let e = &app.game.economy;
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(18), Constraint::Min(6)])
        .split(area);
    let yoy = e.inflation_over(12);
    let bar = |v: f64, lo: f64, hi: f64| {
        let n = (((v - lo) / (hi - lo)).clamp(0.0, 1.0) * 20.0) as usize;
        format!("[{}{}]", "#".repeat(n), ".".repeat(20 - n))
    };
    let mut lines = vec![
        Line::from(Span::styled("Economic indicators (updated monthly)", bold())),
        Line::from(format!(
            "  Inflation        {:>6.2}%  {}  target {:.1}%{}",
            e.inflation * 100.0,
            bar(e.inflation, -0.02, 0.10),
            sim_core::INFLATION_TARGET * 100.0,
            yoy.map_or(String::new(), |y| format!("   past 12 months: {:+.1}%", y * 100.0))
        )),
        Line::from(format!("  Price level CPI  {:>6.1}   (100 at the start of the game)", e.cpi)),
        Line::from(format!("  GDP growth       {:>6.2}%  {}  trend {:.1}%", e.gdp_growth * 100.0, bar(e.gdp_growth, -0.06, 0.08), sim_core::TREND_GROWTH * 100.0)),
        Line::from(format!("  Unemployment     {:>6.2}%  {}", e.unemployment * 100.0, bar(e.unemployment, 0.02, 0.12))),
        Line::from(format!("  Policy rate      {:>6.2}%  (central bank, follows inflation and growth)", app.player.interest_rate * 100.0)),
        Line::from(format!("  Home prices      {:>6.1}   ({:+.1}% since the start; high rates push them down)", e.home_price_index, e.home_price_index - 100.0)),
        Line::from(""),
        Line::from(Span::styled("Government", bold())),
        Line::from(format!(
            "  Tax stance x{:.2}   debt {:.0}% of GDP   revenue {:.1}% / spending {:.1}% of GDP",
            e.govt.tax_mult,
            e.govt.debt_to_gdp * 100.0,
            e.govt.revenue * 100.0,
            e.govt.spending * 100.0
        )),
        Line::from(format!(
            "  Your income tax: {:.0}/mo on {:.0} salary. Investment income {:.0}%, capital gains {:.0}%, business profit {:.0}%.",
            sim_core::monthly_withholding(app.player.salary, e.govt.tax_mult),
            app.player.salary,
            sim_core::INVESTMENT_INCOME_RATE * 100.0,
            sim_core::CAPITAL_GAINS_RATE * 100.0,
            sim_core::BUSINESS_PROFIT_RATE * 100.0
        )),
        Line::from(""),
        Line::from(Span::styled("Commodity markets (supply vs demand; a shortage pushes the price up)", bold())),
    ];
    for c in &e.commodities {
        let imb = c.imbalance();
        lines.push(Line::from(vec![
            Span::raw(format!("  {:<5} supply {:.2}  demand {:.2}  ", c.symbol, c.supply, c.demand)),
            Span::styled(
                format!(
                    "{}",
                    if imb > 0.03 { "SHORTAGE" } else if imb < -0.03 { "GLUT" } else { "balanced" }
                ),
                Style::default().fg(if imb > 0.03 { Color::Red } else if imb < -0.03 { Color::Cyan } else { Color::Green }),
            ),
        ]));
    }
    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title("Economy (w: next view)")),
        rows[0],
    );

    let series = |pick: fn(&sim_core::MonthPoint) -> f64| -> Vec<f64> { e.history.iter().map(pick).collect() };
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(34), Constraint::Percentage(33), Constraint::Percentage(33)])
        .split(rows[1]);
    render_line_chart(f, cols[0], &series(|p| p.inflation * 100.0), "Inflation %", Color::Red);
    render_line_chart(f, cols[1], &series(|p| p.unemployment * 100.0), "Unemployment %", Color::Yellow);
    render_line_chart(f, cols[2], &series(|p| p.policy_rate * 100.0), "Policy rate %", Color::Cyan);
}

/// The `?` help overlay.
pub fn draw_help(f: &mut Frame, app: &App) {
    if !app.show_help {
        return;
    }
    let area = centered(f.area(), 96, 40);
    f.render_widget(Clear, area);
    let h = |t: &str| Line::from(Span::styled(t.to_string(), bold()));
    let lines = vec![
        h("GOAL"),
        Line::from("Reach financial independence: net worth that covers your expenses at a 5% withdrawal"),
        Line::from("rate (20x annual expenses), and outrun your rivals. Go bankrupt and the game is over."),
        Line::from(""),
        h("SCREENS (press 1-7 to jump, w cycles the views inside one)"),
        Line::from("  1 Market (prices, order book, trades)   2 Portfolio (holdings, allocation, net worth)"),
        Line::from("  3 Life (career, goals, rivals, report)  4 Deals (businesses, contracts; w: money & home)"),
        Line::from("  5 World (economy indicators)   6 Logs (every transaction, news, trades; arrows scroll)"),
        Line::from("  7 Manual: a finance glossary with worked examples. Press i on any screen to open the entry"),
        Line::from("    for what you are looking at (for an instrument, its asset class)."),
        Line::from(""),
        h("TRADING (on the Market tab)"),
        Line::from("  Up/Down or j/k  select instrument      b / s   market buy / sell"),
        Line::from("  l   limit order (side, qty, price)      x       cancel your open orders here"),
        Line::from("  f   forward contract (negotiated)       Market orders pay the spread and slip."),
        Line::from(""),
        h("LIFE"),
        Line::from("  c   career: take a course (+skill), job hunt when unemployed"),
        Line::from("  m   money: borrow (rate + 4%), repay, or use the savings account (safe, earns interest)"),
        Line::from("  v   businesses: buy (negotiate), sell, upgrade. They pay a noisy monthly profit."),
        Line::from("  h   home: buy one (20% down + 3% costs, mortgage at rate + 2.5%) or sell yours"),
        Line::from("  a   assets: rental flat, storage units, solar panels, a car (rent is taxed; sales add capital gains)"),
        Line::from(""),
        h("TIME"),
        Line::from("  t   advance 1 day    n   skip N days (stops if something needs you)"),
        Line::from("  Days also pass by themselves (one per minute of play); the game catches up when you reload."),
        Line::from("  Every 30 days: payday (salary less tax, bills, loan, dividends) and the monthly economy update."),
        Line::from(""),
        h("PROMPTS"),
        Line::from("  Decisions and negotiations pop up. Number = choose. In a negotiation, type a"),
        Line::from("  price + Enter to counter, a = accept their number, Esc = walk away,"),
        Line::from("  Tab = hide (Tab or p brings it back). Ignored decisions pick the default."),
        Line::from(""),
        Line::from(Span::styled("Press any key to close this help.", Style::default().fg(Color::DarkGray))),
    ];
    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Help ")),
        area,
    );
}

/// Savings, the loan, and housing in one place.
pub fn draw_money(f: &mut Frame, area: Rect, app: &App) {
    let g = &app.game;
    let now = app.player.current_tick;
    let hpi = g.economy.home_price_index;
    let mut lines: Vec<Line> = vec![];

    lines.push(Line::from(Span::styled("Housing (h)", bold())));
    match &g.home {
        Some(h) => {
            let value = h.value(hpi);
            lines.push(Line::from(vec![
                Span::styled(format!("  You own a {}", h.name), Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(format!(" (bought day {} for {})", h.bought_tick, fmt_money(h.price_paid))),
            ]));
            lines.push(Line::from(format!(
                "  Worth {}   mortgage {} at {:.2}%   equity {}",
                fmt_money(value),
                fmt_money(h.mortgage.principal),
                h.mortgage.annual_rate * 100.0,
                fmt_money(value - h.mortgage.principal)
            )));
            lines.push(Line::from(format!(
                "  Costs {}/mo ({} mortgage + {} upkeep). Renting the same would be about {}/mo.",
                fmt_money(h.monthly_cost(hpi)),
                fmt_money(h.mortgage.payment),
                fmt_money(h.monthly_upkeep(hpi)),
                fmt_money(g.rent)
            )));
            if value < h.mortgage.principal {
                lines.push(Line::from(Span::styled("  Underwater: you owe more than it's worth.", Style::default().fg(Color::Red))));
            }
        }
        None => {
            lines.push(Line::from(format!(
                "  You rent for {}/mo. Home prices index {:.0} ({:+.0}% since the start). Mortgage rate {:.2}%.",
                fmt_money(g.rent),
                hpi,
                hpi - 100.0,
                (app.player.interest_rate + sim_core::MORTGAGE_SPREAD) * 100.0
            )));
            lines.push(Line::from(Span::styled("  For sale (h, then 1 to buy and negotiate):", bold())));
            let rate = app.player.interest_rate + sim_core::MORTGAGE_SPREAD;
            for (i, l) in g.home_listings.iter().enumerate() {
                let need = l.price * (sim_core::DOWN_PAYMENT + sim_core::CLOSING_COST);
                let pmt = sim_core::mortgage_payment(l.price * (1.0 - sim_core::DOWN_PAYMENT), rate);
                let upkeep = l.price * sim_core::UPKEEP_RATE / 12.0;
                let cash_ok = need <= app.free_cash();
                let income_ok = pmt <= sim_core::MAX_PAYMENT_TO_INCOME * app.player.salary;
                let verdict = match (cash_ok, income_ok) {
                    (true, true) => Span::styled("affordable", Style::default().fg(Color::Green)),
                    (false, true) => Span::styled(format!("need {} more cash", fmt_money(need - app.free_cash())), Style::default().fg(Color::Yellow)),
                    (_, false) => Span::styled("payment too high for your income", Style::default().fg(Color::Red)),
                };
                let on = matches!(app.input_mode, InputMode::BuyHome) && app.pick == i;
                lines.push(hl(Line::from(vec![
                    Span::raw(format!(
                        "  {}. {:<15} asking {:>9}  needs {:>8} up front, then {}/mo (+{} upkeep)  ",
                        i + 1,
                        l.name(),
                        fmt_money(l.price),
                        fmt_money(need),
                        fmt_money(pmt),
                        fmt_money(upkeep)
                    )),
                    verdict,
                    Span::styled(format!("  [{} days left]", (l.expires_tick - now).max(0)), Style::default().fg(Color::DarkGray)),
                ]), on));
            }
            lines.push(Line::from(Span::styled(
                format!(
                    "  Agent: {} ({}). Rent is {:.0}% of what you spend now.",
                    g.npcs[sim_core::NPC_REALTOR].name,
                    g.npcs[sim_core::NPC_REALTOR].mood(),
                    app.game.rent / (app.player.expenses + app.game.rent).max(1.0) * 100.0
                ),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }


    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Assets: investment property & utilities (a)", bold())));
    let cpi = g.economy.cpi;
    if g.assets.is_empty() {
        lines.push(Line::from("  You don't own any yet."));
    }
    for (i, a) in g.assets.iter().enumerate() {
        let s = a.spec();
        let running = fmt_money(a.monthly_running_cost(cpi));
        let detail = if s.is_investment() {
            format!("rent ~{}/mo, costs {}/mo", fmt_money(a.monthly_rent()), running)
        } else if s.monthly_saving > 0.0 {
            format!("saves {}/mo, costs {}/mo", fmt_money(a.monthly_saving(cpi)), running)
        } else {
            format!("costs {running}/mo to run")
        };
        let debt = match &a.mortgage {
            Some(m) => format!("  mortgage {} at {:.2}% ({}/mo)", fmt_money(m.principal), m.annual_rate * 100.0, fmt_money(m.payment)),
            None => String::new(),
        };
        let on = matches!(app.input_mode, InputMode::SellAsset | InputMode::PrepayAssetPick) && app.pick == i;
        lines.push(hl(Line::from(format!(
            "  {}. {:<14} worth {:>9}  paid {:>9}{}  {}",
            i + 1,
            s.name,
            fmt_money(a.value),
            fmt_money(a.price_paid),
            debt,
            detail
        )), on));
    }
    lines.push(Line::from(Span::styled(
        format!("  For sale at the listed price (a, then 1; no haggling). Buy-to-let mortgage at {:.2}%:", (app.player.interest_rate + sim_core::BTL_SPREAD) * 100.0),
        bold(),
    )));
    for (k, s) in sim_core::ASSET_SPECS.iter().enumerate() {
        let price = sim_core::asset_price(k, hpi, cpi);
        let (with_loan, _) = sim_core::purchase_terms(k, price, true);
        let (cash_only, _) = sim_core::purchase_terms(k, price, false);
        let cheapest = with_loan.min(cash_only);
        let verdict = if cheapest <= app.free_cash() {
            Span::styled("affordable", Style::default().fg(Color::Green))
        } else {
            Span::styled(format!("need {} more cash", fmt_money(cheapest - app.free_cash())), Style::default().fg(Color::Yellow))
        };
        let terms = if s.financeable {
            format!("{} up front with a mortgage, or {} cash", fmt_money(with_loan), fmt_money(cash_only))
        } else {
            format!("{} cash", fmt_money(cash_only))
        };
        let on = matches!(app.input_mode, InputMode::BuyAsset) && app.pick == k;
        lines.push(hl(
            Line::from(vec![Span::raw(format!("  {}. {:<14} {:>9}  {}  ", k + 1, s.name, fmt_money(price), terms)), verdict]),
            on,
        ));
        lines.push(Line::from(Span::styled(format!("       {}", s.blurb), Style::default().fg(Color::DarkGray))));
    }
    lines.push(Line::from(Span::styled(
        "  Rental profit (rent - upkeep - mortgage interest) is taxed at 15%; selling at a profit adds to your capital gains. Solar and cars lose value and don't count toward financial independence.",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Bank savings account (m, then 3 to deposit, 4 to withdraw)", bold())));
    let rate = (app.player.interest_rate - sim_core::SAVINGS_SPREAD).max(0.0);
    lines.push(Line::from(vec![
        Span::raw("  Balance "),
        Span::styled(fmt_money(g.savings), Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(format!(
            "   rate {:.2}% a year (policy rate {:.1}% - {:.1}%)   interest earned so far {} (after tax)",
            rate * 100.0,
            app.player.interest_rate * 100.0,
            sim_core::SAVINGS_SPREAD * 100.0,
            fmt_money(g.stats.savings_interest)
        )),
    ]));
    let (shown, label) = if g.savings > 0.0 { (g.savings, "Left alone") } else { (10_000.0, "For example, 10,000 left alone") };
    lines.push(Line::from(format!(
        "  Next payday: about +{} after tax.  {label} for 10 years at today's rate it compounds to about {}.",
        fmt_money(g.next_savings_interest(app.player.interest_rate)),
        fmt_money(sim_core::GameState::project_savings(shown, app.player.interest_rate, 10.0))
    )));
    lines.push(Line::from(Span::styled(
        "  A bank account, not an investment: no price, no spread, safe from market swings. Used first when bills can't be paid.",
        Style::default().fg(Color::DarkGray),
    )));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Loan (m for the money menu)", bold())));
    match &g.loan {
        Some(l) => lines.push(Line::from(format!(
            "  owe {:.0} at {:.2}%: ~{:.0}/mo interest + {:.0}/mo repayment",
            l.principal,
            l.annual_rate * 100.0,
            l.principal * l.annual_rate / 12.0,
            l.principal * sim_core::LOAN_MIN_PAYMENT_RATE
        ))),
        None => lines.push(Line::from(format!(
            "  none. You could borrow up to {:.0} (at the prevailing rate + {:.0}%).",
            g.borrowing_room(&app.player),
            sim_core::LOAN_SPREAD * 100.0
        ))),
    }

    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: true }).block(Block::default().borders(Borders::ALL).title("Money & home (w: businesses & contracts)")),
        area,
    );
}

/// The cash ledger (SQL `transactions` table), newest first.
pub fn draw_transactions(f: &mut Frame, area: Rect, app: &App) {
    let transfer = |c: &str| matches!(c, "trade" | "loan_draw" | "loan_principal" | "deposit" | "withdrawal" | "deal" | "business_invest" | "home" | "mortgage_principal" | "asset");
    let visible = area.height.saturating_sub(4) as usize;
    let rows: Vec<Line> = app
        .transactions
        .iter()
        .skip(app.log_scroll)
        .take(visible)
        .map(|t| {
            let dim = transfer(&t.category);
            let color = if dim { Color::DarkGray } else if t.amount >= 0.0 { Color::Green } else { Color::Red };
            Line::from(vec![
                Span::styled(format!(" {:>6}  ", t.tick), Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{:<18}", t.category), Style::default().fg(if dim { Color::DarkGray } else { Color::Cyan })),
                Span::raw(format!("{:<46}", t.description.chars().take(45).collect::<String>())),
                Span::styled(format!("{:>12}", format!("{:+.2}", t.amount)), Style::default().fg(color)),
            ])
        })
        .collect();
    let mut lines = vec![Line::from(Span::styled(
        format!(" {:>6}  {:<18}{:<46}{:>12}", "day", "category", "description", "amount"),
        bold(),
    ))];
    if rows.is_empty() {
        lines.push(Line::from(" Nothing recorded yet. Advance time (t) to see paydays."));
    }
    lines.extend(rows);
    let total = app.transactions.len();
    let title = format!(
        "Transactions - {}-{} of {} (grey = moving money between assets, not income or spending; w: news)",
        (app.log_scroll + 1).min(total),
        (app.log_scroll + visible).min(total),
        total
    );
    f.render_widget(Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(title)), area);
}

/// The finance manual: terms on the left, the selected entry on the right.
pub fn draw_manual(f: &mut Frame, area: Rect, app: &App) {
    let parts = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(38), Constraint::Min(30)])
        .split(area);

    // Left: categories with their terms; the selected one highlighted.
    let mut lines: Vec<Line> = vec![];
    let mut selected_line = 0usize;
    let mut last_cat = "";
    for (i, e) in manual::ENTRIES.iter().enumerate() {
        if e.category != last_cat {
            if !lines.is_empty() {
                lines.push(Line::from(""));
            }
            lines.push(Line::from(Span::styled(e.category.to_uppercase(), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))));
            last_cat = e.category;
        }
        if i == app.manual_selected {
            selected_line = lines.len();
            lines.push(Line::from(Span::styled(
                format!(" {:<35}", e.term.chars().take(35).collect::<String>()),
                Style::default().fg(Color::Black).bg(Color::Cyan),
            )));
        } else {
            lines.push(Line::from(format!("  {}", e.term.chars().take(35).collect::<String>())));
        }
    }
    let height = parts[0].height.saturating_sub(2) as usize;
    let offset = selected_line.saturating_sub(height / 2).min(lines.len().saturating_sub(height));
    f.render_widget(
        Paragraph::new(lines)
            .scroll((offset as u16, 0))
            .block(Block::default().borders(Borders::ALL).title(" Finance manual ")),
        parts[0],
    );

    // Right: the entry.
    let e = &manual::ENTRIES[app.manual_selected.min(manual::ENTRIES.len() - 1)];
    let heading = |t: &str, c: Color| Line::from(Span::styled(t.to_string(), Style::default().fg(c).add_modifier(Modifier::BOLD)));
    let detail = vec![
        heading("What it is", Color::Cyan),
        Line::from(e.what),
        Line::from(""),
        heading("Example", Color::Green),
        Line::from(e.example),
        Line::from(""),
        heading("Why it matters, and what happens when it changes", Color::Yellow),
        Line::from(e.matters),
        Line::from(""),
        heading("See it in the game", Color::Magenta),
        Line::from(Span::styled(e.where_in_game, Style::default().fg(Color::Gray))),
    ];
    f.render_widget(
        Paragraph::new(detail)
            .wrap(Wrap { trim: true })
            .block(Block::default().borders(Borders::ALL).title(format!(" {} - {} ", e.category, e.term))),
        parts[1],
    );
}
