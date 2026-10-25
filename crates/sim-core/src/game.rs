//! Everything that makes this a game rather than a price chart: random
//! events and news, life decisions, a career, businesses, NPC deals and
//! rivals, and win/lose conditions. Pure logic like the rest of
//! sim-core; the whole `GameState` serializes to JSON for persistence.

use std::collections::HashMap;

use rand::Rng;
use rand_distr::{Distribution, Normal};
use serde::{Deserialize, Serialize};

use crate::assets::OwnedAsset;
use crate::career::*;
use crate::economy::*;
use crate::engine::{MAX_INTEREST_RATE, MIN_INTEREST_RATE, TICK_DT_YEARS};
use crate::events::*;
use crate::housing::*;
use crate::instrument::{AssetClass, Instrument, InstrumentState};
use crate::market::{Fill, FillKind, Market};
use crate::negotiation::{Negotiation, Response, Side};
use crate::player::{GameMode, Player, PAY_PERIOD_TICKS};
use crate::portfolio::Portfolio;
use crate::social::*;

/// Net worth milestones announced as you pass them.
pub const MILESTONES: &[f64] = &[10_000.0, 25_000.0, 50_000.0, 100_000.0, 250_000.0, 500_000.0, 1_000_000.0];
/// Financial independence: net worth x this / 12 covers monthly expenses
/// (a 5% rule, i.e. net worth of 20x annual expenses).
pub const FI_WITHDRAWAL_RATE: f64 = 0.05;
pub const MARKET_EVENT_CHANCE: f64 = 1.0 / 50.0;
pub const PERSONAL_EVENT_CHANCE: f64 = 1.0 / 60.0;
pub const DECISION_CHANCE: f64 = 1.0 / 80.0;
/// Consecutive paydays you can fail to cover your bills before you're
/// bankrupt.
pub const DISTRESS_LIMIT: u32 = 3;
// Everything time-limited is measured in game days (one tick = one day),
// independent of how fast the clock runs in real time: business listings last
// 120 days (homes 180), new ones appear about every 60 days, and you have 20
// days to answer a salary review or a decision.
pub const LISTING_REFRESH_TICKS: i64 = 60;
pub const LISTING_LIFETIME_TICKS: i64 = 120;
pub const REVIEW_WINDOW_TICKS: i64 = 20;
/// Share of paper value you get if bills force a business sale.
pub const FIRE_SALE_FACTOR: f64 = 0.70;
/// Share of a promotion's raise that your expenses grow by.
pub const LIFESTYLE_CREEP: f64 = 0.30;
/// Cash you must hold, as a fraction of notional, to open a forward.
pub const FORWARD_MARGIN: f64 = 0.10;
/// Loans cost the prevailing interest rate plus this spread.
pub const LOAN_SPREAD: f64 = 0.04;
/// Total debt allowed, as a multiple of your monthly salary.
pub const MAX_LOAN_MULTIPLE: f64 = 6.0;
/// Share of the principal repaid automatically each payday (on top of interest).
pub const LOAN_MIN_PAYMENT_RATE: f64 = 0.02;
const NEWS_CAP: usize = 120;

/// A personal loan. Interest and a minimum repayment come out at every
/// payday; if you can't cover them, the usual shortfall rules apply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Loan {
    pub principal: f64,
    /// Annual rate, fixed when the money is borrowed (blended if you borrow more).
    pub annual_rate: f64,
}

/// Bank savings pay the policy rate minus this spread.
pub const SAVINGS_SPREAD: f64 = 0.005;

/// One cash movement, for the SQL transaction ledger.
#[derive(Debug, Clone, PartialEq)]
pub struct LedgerEntry {
    pub tick: i64,
    pub category: String,
    pub description: String,
    pub amount: f64,
}

/// Running totals for the run report.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stats {
    /// Bank savings interest received, after tax.
    pub savings_interest: f64,
    pub housing_costs: f64,
    /// Rent received from investment property.
    pub rental_income: f64,
    /// Upkeep, running costs and mortgage interest on your assets.
    pub asset_costs: f64,
    pub home_purchases: u32,
    pub taxes_paid: f64,
    pub investment_income: f64,
    pub peak_net_worth: f64,
    pub peak_tick: i64,
    pub salary_income: f64,
    pub business_income: f64,
    pub expenses_paid: f64,
    pub interest_paid: f64,
    pub forced_sales: u32,
    pub decisions_made: u32,
    pub deals_done: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GameStatus {
    Playing,
    /// Financial independence reached. You can keep playing.
    Won { tick: i64 },
    /// Game over: the save is frozen.
    Lost { tick: i64, reason: String },
}

impl GameStatus {
    pub fn is_lost(&self) -> bool {
        matches!(self, GameStatus::Lost { .. })
    }
    pub fn is_won(&self) -> bool {
        matches!(self, GameStatus::Won { .. })
    }
}

/// A forward contract with the broker, cash-settled at `settle_tick`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Forward {
    pub id: u64,
    pub instrument_id: i32,
    pub symbol: String,
    /// True: you profit if the price ends above `strike`.
    pub is_long: bool,
    pub quantity: f64,
    pub strike: f64,
    pub settle_tick: i64,
}

impl Forward {
    pub fn value_at(&self, spot: f64) -> f64 {
        let diff = if self.is_long { spot - self.strike } else { self.strike - spot };
        diff * self.quantity
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NegPurpose {
    Forward { instrument_id: i32, symbol: String, is_long: bool, quantity: f64, horizon: i64 },
    BuyBusiness { listing_id: u64 },
    BuyHome { listing_id: u64 },
    SellBusiness { business_id: u64 },
    /// Annual review. `promote`: a promotion comes with the offer.
    Salary { promote: bool },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveNegotiation {
    pub npc: usize,
    pub purpose: NegPurpose,
    pub neg: Negotiation,
    /// If you don't finish by then, the NPC's standing offer is taken
    /// (only used for salary reviews).
    pub deadline: Option<i64>,
    pub title: String,
    pub log: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NegStep {
    Continue(String),
    Done(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameState {
    pub status: GameStatus,
    pub career: Career,
    pub businesses: Vec<Business>,
    pub listings: Vec<Listing>,
    pub contracts: Vec<Forward>,
    pub npcs: Vec<Npc>,
    pub rivals: Vec<Rival>,
    pub effects: Vec<ActiveEffect>,
    pub scheduled: Vec<ScheduledEvent>,
    pub scheduled_cash: Vec<ScheduledCash>,
    pub temp_expenses: Vec<TempExpense>,
    pub decision: Option<PendingDecision>,
    pub negotiation: Option<ActiveNegotiation>,
    pub news: Vec<NewsItem>,
    pub distress_months: u32,
    pub milestones_hit: usize,
    pub player_rank: usize,
    pub last_business_income: f64,
    #[serde(default)]
    pub loan: Option<Loan>,
    /// The macro economy: inflation, growth, unemployment, government.
    #[serde(default)]
    pub economy: Economy,
    /// Bank savings account balance (earns interest, safe from the market).
    #[serde(default)]
    pub savings: f64,
    /// Gains realized by sales this month, awaiting capital gains tax.
    #[serde(default)]
    pub pending_gains: f64,
    /// Net losses carried forward to offset future gains (<= 0).
    #[serde(default)]
    pub loss_carry: f64,
    /// Cash movements not yet written to the SQL ledger.
    #[serde(skip)]
    pub ledger: Vec<LedgerEntry>,
    /// Monthly economic snapshots not yet written to SQL.
    /// Monthly rent (a share of your expenses while you rent).
    #[serde(default)]
    pub rent: f64,
    #[serde(default)]
    pub home: Option<Home>,
    #[serde(default)]
    pub home_listings: Vec<HomeListing>,
    /// Investment property, solar panels, a car: see `assets.rs`.
    #[serde(default)]
    pub assets: Vec<OwnedAsset>,
    #[serde(skip)]
    pub new_points: Vec<MonthPoint>,
    /// True while replaying time you were away (launch catch-up). Your
    /// personal life is paused: no pay, bills, reviews or personal events.
    /// Markets, the economy and your investments keep running.
    #[serde(skip)]
    pub away: bool,
    #[serde(default)]
    pub stats: Stats,
    pub next_id: u64,
    pub next_listing_tick: i64,
    /// False in unit tests that need a deterministic economy.
    pub events_enabled: bool,
}

impl GameState {
    pub fn new(player: &Player, net_worth: f64, events_enabled: bool) -> GameState {
        let mut g = GameState {
            status: GameStatus::Playing,
            career: Career::new(player.salary, player.current_tick),
            businesses: vec![],
            listings: vec![],
            contracts: vec![],
            npcs: default_cast(),
            rivals: default_rivals(net_worth),
            effects: vec![],
            scheduled: vec![],
            scheduled_cash: vec![],
            temp_expenses: vec![],
            decision: None,
            negotiation: None,
            news: vec![],
            distress_months: 0,
            milestones_hit: MILESTONES.iter().take_while(|m| net_worth >= **m).count(),
            player_rank: 0,
            last_business_income: 0.0,
            loan: None,
            economy: Economy::new(),
            savings: 0.0,
            pending_gains: 0.0,
            loss_carry: 0.0,
            ledger: vec![],
            rent: player.expenses * RENT_SHARE,
            home: None,
            home_listings: vec![],
            assets: vec![],
            new_points: vec![],
            away: false,
            stats: Stats { peak_net_worth: net_worth, peak_tick: player.current_tick, ..Stats::default() },
            next_id: 1,
            next_listing_tick: player.current_tick,
            events_enabled,
        };
        g.player_rank = g.rank(net_worth);
        g.refresh_listings(player.current_tick, &mut rand::thread_rng());
        g.push_news(
            player.current_tick,
            NewsKind::Goal,
            format!(
                "Goal: reach financial independence (net worth covering your expenses at {:.0}% a year) and outrun your rivals.",
                FI_WITHDRAWAL_RATE * 100.0
            ),
        );
        g
    }

    /// A game with random events switched off (tests).
    pub fn quiet(player: &Player) -> GameState {
        GameState::new(player, 2_000.0, false)
    }

    pub fn push_news(&mut self, tick: i64, kind: NewsKind, text: impl Into<String>) {
        self.news.push(NewsItem { tick, kind, text: text.into() });
        if self.news.len() > NEWS_CAP {
            let excess = self.news.len() - NEWS_CAP;
            self.news.drain(0..excess);
        }
    }

    pub(crate) fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Writes a cash movement to the ledger (zero amounts are skipped).
    pub fn record(&mut self, tick: i64, category: &str, description: impl Into<String>, amount: f64) {
        if amount.abs() < 1e-9 {
            return;
        }
        self.ledger.push(LedgerEntry { tick, category: category.to_string(), description: description.into(), amount });
    }

    /// Dividends, coupons and rent arrive taxed at source.
    pub fn pay_investment_income(&mut self, player: &mut Player, gross: f64) {
        if gross <= 0.0 {
            return;
        }
        let tax = gross * INVESTMENT_INCOME_RATE;
        player.cash += gross - tax;
        let now = player.current_tick;
        self.record(now, "dividends", "Dividends, coupons and rent", gross);
        self.record(now, "tax", "Tax on investment income", -tax);
        self.stats.investment_income += gross;
        self.stats.taxes_paid += tax;
    }

    /// Moves gains realized by trading into the tax bucket. Call before
    /// persisting so a quit mid-month can't dodge the tax.
    pub fn absorb_realized(&mut self, portfolio: &mut Portfolio) {
        self.pending_gains += portfolio.realized_gains;
        portfolio.realized_gains = 0.0;
    }

    fn cond_ok(&self, c: Cond) -> bool {
        match c {
            Cond::Always => true,
            Cond::Employed => self.career.employed,
            Cond::Renting => self.home.is_none(),
            Cond::Owning => self.home.is_some(),
        }
    }

    /// Saves from older versions have a smaller cast and no rent figure.
    pub fn ensure_compat(&mut self, player: &Player) {
        let cast = default_cast();
        while self.npcs.len() < cast.len() {
            self.npcs.push(cast[self.npcs.len()].clone());
        }
        if self.rent <= 0.0 && self.home.is_none() {
            self.rent = player.expenses * RENT_SHARE;
        }
    }

    pub fn home_equity(&self) -> f64 {
        self.home.as_ref().map_or(0.0, |h| h.equity(self.economy.home_price_index))
    }

    /// What owning costs this month (mortgage payment plus upkeep). Zero
    /// while you rent, because rent is already part of your expenses.
    pub fn housing_cost(&self) -> f64 {
        self.home.as_ref().map_or(0.0, |h| h.monthly_cost(self.economy.home_price_index))
    }

    /// Everything you spend each month on living and housing.
    pub fn total_costs(&self, player: &Player) -> f64 {
        player.expenses + self.housing_cost() + self.lifestyle_net_cost()
    }

    /// Progress toward financial independence. Home equity and lifestyle
    /// assets (car, solar) don't count: you can't live off them.
    pub fn fi_progress(&self, player: &Player, net_worth: f64) -> f64 {
        self.fi_ratio(net_worth - self.non_fi_wealth(), self.total_costs(player))
    }

    pub fn has_prompt(&self) -> bool {
        self.decision.is_some() || self.negotiation.is_some()
    }

    // ------------------------------------------------ market modifiers

    pub fn drift_shift(&self, i: &Instrument) -> f64 {
        self.effects
            .iter()
            .flat_map(|e| e.drift.iter())
            .filter(|(t, _)| t.matches(i))
            .map(|(_, d)| d)
            .sum()
    }

    pub fn vol_mult(&self, i: &Instrument) -> f64 {
        self.effects
            .iter()
            .flat_map(|e| e.vol.iter())
            .filter(|(t, _)| t.matches(i))
            .map(|(_, m)| m)
            .product()
    }

    /// Economy-wide multiplier on business profits.
    pub fn econ_factor(&self) -> f64 {
        self.effects.iter().map(|e| e.econ).product()
    }

    // ------------------------------------------------------ net worth

    /// Businesses at paper value plus forward contracts marked to market,
    /// plus your bank savings, minus any loan you owe.
    pub fn assets_value(&self, prices: &HashMap<i32, f64>) -> f64 {
        let biz: f64 = self.businesses.iter().map(|b| b.value()).sum();
        let fwd: f64 = self
            .contracts
            .iter()
            .map(|c| c.value_at(prices.get(&c.instrument_id).copied().unwrap_or(c.strike)))
            .sum();
        biz + fwd + self.savings + self.home_equity() + self.assets_equity() - self.loan.as_ref().map_or(0.0, |l| l.principal)
    }

    pub fn fi_ratio(&self, net_worth: f64, monthly_expenses: f64) -> f64 {
        (net_worth * FI_WITHDRAWAL_RATE / 12.0) / monthly_expenses.max(1.0)
    }

    /// (name, wealth, is_player), richest first.
    pub fn leaderboard(&self, player_net_worth: f64) -> Vec<(String, f64, bool)> {
        let mut v: Vec<(String, f64, bool)> =
            self.rivals.iter().map(|r| (r.name.clone(), r.wealth, false)).collect();
        v.push(("You".to_string(), player_net_worth, true));
        v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        v
    }

    fn rank(&self, player_net_worth: f64) -> usize {
        self.leaderboard(player_net_worth).iter().position(|e| e.2).unwrap_or(0) + 1
    }

    // ------------------------------------------------------- tick hooks

    /// Start of a tick, before prices move: expire old effects and fire
    /// market events that are due (their price jumps apply now, so the
    /// regular price step and any fund NAV see them).
    pub fn pre_step(&mut self, player: &mut Player, instruments: &mut [InstrumentState]) {
        let now = player.current_tick;
        self.effects.retain(|e| e.ends_tick > now);

        let (due, later): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.scheduled).into_iter().partition(|s| s.fire_tick <= now);
        self.scheduled = later;

        for ev in due {
            let Some(def) = market_event(&ev.key) else { continue };
            self.economy.apply_event(def.key);
            player.interest_rate = (player.interest_rate + def.rate_shock).clamp(MIN_INTEREST_RATE, MAX_INTEREST_RATE);
            for (target, frac) in def.shocks {
                let target = target.owned();
                for s in instruments.iter_mut().filter(|s| target.matches(&s.instrument)) {
                    s.price = (s.price * (1.0 + frac)).max(0.01);
                }
            }
            if def.duration > 0 {
                self.effects.push(ActiveEffect {
                    label: def.key.to_string(),
                    ends_tick: now + def.duration,
                    drift: def.drift.iter().map(|(t, d)| (t.owned(), *d)).collect(),
                    vol: def.vol.iter().map(|(t, m)| (t.owned(), *m)).collect(),
                    econ: def.econ,
                });
            }
            self.push_news(now, NewsKind::Market, def.headline);
        }
    }

    /// End of a tick, after prices moved and resting orders cleared:
    /// income, bills, deals settling, career, rivals, random events.
    #[allow(clippy::too_many_arguments)]
    pub fn post_step<R: Rng>(
        &mut self,
        player: &mut Player,
        instruments: &[InstrumentState],
        portfolio: &mut Portfolio,
        market: &mut Market,
        index_return: f64,
        rng: &mut R,
        fills: &mut Vec<Fill>,
    ) {
        if self.status.is_lost() {
            return;
        }
        self.ensure_compat(player);
        let now = player.current_tick;

        for r in &mut self.rivals {
            r.step(index_return, TICK_DT_YEARS, rng);
        }

        // Time away doesn't use up a payment plan you weren't charged for.
        if self.away {
            for t in &mut self.temp_expenses {
                t.ends_tick += 1;
            }
        }

        // Delayed cash (payouts, repayments).
        let (due, later): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.scheduled_cash).into_iter().partition(|c| c.due_tick <= now);
        self.scheduled_cash = later;
        for c in due {
            player.cash += c.amount;
            self.record(now, "windfall", c.label.clone(), c.amount);
            self.push_news(now, NewsKind::Personal, format!("{}: {:+.2}", c.label, c.amount));
        }

        // Back to work?
        if !self.career.employed {
            if self.career.job_hunting {
                if let Some(t) = self.career.rehire_tick.as_mut() {
                    *t -= 1;
                }
            }
            if self.career.rehire_tick.map_or(true, |t| now >= t) {
                self.career.employed = true;
                self.career.rehire_tick = None;
                self.career.job_hunting = false;
                self.push_news(now, NewsKind::Career, "You found a new job. Your paychecks are back.");
            }
        }

        // Forward contracts settle.
        let (due, later): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.contracts).into_iter().partition(|c| c.settle_tick <= now);
        self.contracts = later;
        for c in due {
            let spot = instruments
                .iter()
                .find(|s| s.instrument.id == c.instrument_id)
                .map_or(c.strike, |s| s.price);
            let payoff = c.value_at(spot);
            player.cash += payoff;
            self.record(now, "forward", format!("Forward settled: {}", c.symbol), payoff);
            self.push_news(
                now,
                NewsKind::Deal,
                format!(
                    "Forward settled: {} {} {} at {:.2} vs {:.2} now: {:+.2}",
                    if c.is_long { "long" } else { "short" },
                    c.quantity,
                    c.symbol,
                    c.strike,
                    spot,
                    payoff
                ),
            );
        }

        if now % PAY_PERIOD_TICKS == 0 {
            self.payday(player, instruments, portfolio, market, rng, fills);
            if self.status.is_lost() {
                return;
            }
        }

        if now >= self.next_listing_tick {
            self.refresh_listings(now, rng);
        }

        if self.events_enabled {
            self.roll_events(player, rng);
        }

        // Prompts that ran out of time resolve themselves (but not while
        // you're away: they wait for you).
        if let Some(d) = self.decision.as_ref().filter(|_| !self.away) {
            if now >= d.deadline_tick {
                if let Some(def) = decision(&d.key) {
                    self.push_news(now, NewsKind::Personal, "You didn't decide in time, so the default was chosen.");
                    self.resolve_decision(player, def.default_option, rng);
                } else {
                    self.decision = None;
                }
            }
        }
        if !self.away && self.negotiation.as_ref().and_then(|n| n.deadline).is_some_and(|dl| now >= dl) {
            if let Some(n) = self.negotiation.take() {
                let price = n.neg.accept_counter();
                let msg = self.finalize(n, price, player, instruments);
                self.push_news(now, NewsKind::Career, format!("The review wrapped up without you. {msg}"));
            }
        }
    }

    /// After net worth is known for the tick: milestones, rank, win.
    pub fn check_goals(&mut self, player: &Player, net_worth: f64) {
        if self.status.is_lost() {
            return;
        }
        let now = player.current_tick;
        if net_worth > self.stats.peak_net_worth {
            self.stats.peak_net_worth = net_worth;
            self.stats.peak_tick = now;
        }
        while self.milestones_hit < MILESTONES.len() && net_worth >= MILESTONES[self.milestones_hit] {
            let m = MILESTONES[self.milestones_hit];
            self.milestones_hit += 1;
            self.push_news(now, NewsKind::Goal, format!("Milestone: net worth passes ${m:.0}"));
        }
        if now % PAY_PERIOD_TICKS == 0 {
            let rank = self.rank(net_worth);
            if rank != self.player_rank {
                let verb = if rank < self.player_rank { "moved up" } else { "slipped" };
                let leaders = self.leaderboard(net_worth);
                let text = format!("You {verb} to #{rank} of {} (ahead of: {}).", leaders.len(), {
                    let behind: Vec<_> = leaders.iter().skip(rank).map(|e| e.0.clone()).collect();
                    if behind.is_empty() { "nobody".to_string() } else { behind.join(", ") }
                });
                self.push_news(now, NewsKind::Social, text);
                self.player_rank = rank;
            }
        }
        if self.status == GameStatus::Playing && self.fi_progress(player, net_worth) >= 1.0 {
            self.status = GameStatus::Won { tick: now };
            self.push_news(
                now,
                NewsKind::Goal,
                "FINANCIAL INDEPENDENCE! Your wealth now covers your expenses. You can keep playing.",
            );
        }
    }

    // ----------------------------------------------------------- payday

    fn payday<R: Rng>(
        &mut self,
        player: &mut Player,
        instruments: &[InstrumentState],
        portfolio: &mut Portfolio,
        market: &mut Market,
        rng: &mut R,
        fills: &mut Vec<Fill>,
    ) {
        let now = player.current_tick;
        self.absorb_realized(portfolio);

        // The monthly macro update: growth, unemployment, inflation, the
        // central bank, commodity supply/demand, the government budget.
        let ratio = |symbol: &str| {
            instruments
                .iter()
                .find(|s| s.instrument.symbol == symbol)
                .map_or(1.0, |s| s.price / s.instrument.base_price)
        };
        let (rate, news) = self.economy.monthly(
            now / PAY_PERIOD_TICKS,
            now,
            self.econ_factor(),
            player.interest_rate,
            ratio,
            rng,
        );
        player.interest_rate = rate.clamp(MIN_INTEREST_RATE, MAX_INTEREST_RATE);
        for n in news {
            self.push_news(now, NewsKind::Market, n);
        }
        if let Some(p) = self.economy.history.last() {
            self.new_points.push(p.clone());
        }
        // Asset values move with the market and the clock even while you're away.
        self.revalue_assets();

        let tax_mult = self.economy.govt.tax_mult;
        if player.game_mode == GameMode::SalariedInvestor && !self.away {
            if self.career.employed {
                let tax = monthly_withholding(player.salary, tax_mult);
                player.cash += player.salary - tax;
                self.record(now, "salary", "Salary", player.salary);
                self.record(now, "tax", "Income tax withheld", -tax);
                self.stats.salary_income += player.salary;
                self.stats.taxes_paid += tax;
            } else {
                let benefit = player.salary * BENEFIT_RATE;
                player.cash += benefit;
                self.record(now, "benefit", "Unemployment benefit", benefit);
                self.stats.salary_income += benefit;
            }
            player.cash -= player.expenses;
            self.record(now, "living", "Living expenses", -player.expenses);
            self.stats.expenses_paid += player.expenses;

            let temps: Vec<(String, f64)> = self.temp_expenses.iter().map(|t| (t.label.clone(), t.amount)).collect();
            for (label, amount) in temps {
                player.cash -= amount;
                self.record(now, if amount >= 0.0 { "expense" } else { "other_income" }, label, -amount);
            }
            self.temp_expenses.retain(|t| t.ends_tick > now);

            if let Some(loan) = self.loan.as_mut() {
                let interest = loan.principal * loan.annual_rate / 12.0;
                let repay = (loan.principal * LOAN_MIN_PAYMENT_RATE).min(loan.principal);
                player.cash -= interest + repay;
                loan.principal -= repay;
                let paid_off = loan.principal < 1.0;
                self.stats.interest_paid += interest;
                self.record(now, "interest", "Loan interest", -interest);
                self.record(now, "loan_principal", "Loan repayment", -repay);
                if paid_off {
                    self.loan = None;
                    self.push_news(now, NewsKind::Personal, "You've paid off your loan.");
                }
            }

            // Owning: mortgage and upkeep (rent was dropped from your expenses
            // when you bought).
            let hpi = self.economy.home_price_index;
            if let Some(h) = self.home.as_mut() {
                let interest = h.mortgage.principal * h.mortgage.annual_rate / 12.0;
                let principal_part = (h.mortgage.payment - interest).clamp(0.0, h.mortgage.principal);
                let upkeep = h.monthly_upkeep(hpi);
                player.cash -= interest + principal_part + upkeep;
                h.mortgage.principal -= principal_part;
                let cleared = h.mortgage.principal < 1.0;
                self.stats.housing_costs += interest + upkeep;
                self.record(now, "housing", "Mortgage interest and upkeep", -(interest + upkeep));
                self.record(now, "mortgage_principal", "Mortgage principal", -principal_part);
                if cleared {
                    self.home.as_mut().expect("home exists").mortgage.principal = 0.0;
                    self.push_news(now, NewsKind::Personal, "You've paid off your mortgage. The home is yours outright.");
                }
            }

            // Rent, upkeep, mortgages and bill savings on your other assets.
            self.assets_payday(player, rng);
        }

        // Bank savings interest, paid out to your account, taxed.
        if self.savings > 0.0 {
            let rate = (player.interest_rate - SAVINGS_SPREAD).max(0.0);
            let gross = self.savings * rate / 12.0;
            let tax = gross * INVESTMENT_INCOME_RATE;
            player.cash += gross - tax;
            self.record(now, "interest_income", "Savings account interest", gross);
            self.record(now, "tax", "Tax on savings interest", -tax);
            self.stats.savings_interest += gross - tax;
            self.stats.investment_income += gross;
            self.stats.taxes_paid += tax;
        }

        // Businesses report their month.
        let econ = self.econ_factor();
        let normal = Normal::new(0.0, 1.0).expect("valid normal");
        let mut income = 0.0;
        for b in &self.businesses {
            let z: f64 = normal.sample(rng);
            let base = b.monthly_profit * b.efficiency;
            income += (base * econ * (1.0 + b.kind().volatility * z)).max(-0.4 * base);
        }
        let biz_tax = income.max(0.0) * BUSINESS_PROFIT_RATE;
        self.last_business_income = income;
        self.stats.business_income += income;
        self.stats.taxes_paid += biz_tax;
        player.cash += income - biz_tax;
        self.record(now, "business", "Business profit", income);
        self.record(now, "tax", "Tax on business profit", -biz_tax);

        // Capital gains tax on this month's realized gains; losses carry forward.
        let net = self.pending_gains + self.loss_carry;
        self.pending_gains = 0.0;
        if net > 0.0 {
            let tax = net * CAPITAL_GAINS_RATE;
            self.loss_carry = 0.0;
            player.cash -= tax;
            self.stats.taxes_paid += tax;
            self.record(now, "tax", "Capital gains tax", -tax);
        } else {
            self.loss_carry = net;
        }

        // Can you pay your bills? (Not checked while you're away.)
        if player.cash < 0.0 && !self.away {
            self.cover_shortfall(player, instruments, portfolio, market, fills);
        }
        if self.away {
            // Your life is paused: no strikes while you're gone.
        } else if player.cash < 0.0 {
            self.distress_months += 1;
            if self.distress_months >= DISTRESS_LIMIT {
                self.status = GameStatus::Lost {
                    tick: now,
                    reason: "Bankrupt: you couldn't cover your bills and had nothing left to sell.".to_string(),
                };
                self.push_news(now, NewsKind::Goal, "GAME OVER: you're bankrupt.");
                return;
            }
            self.push_news(
                now,
                NewsKind::Goal,
                format!(
                    "You can't cover your bills ({}/{}). Raise cash before the next payday!",
                    self.distress_months, DISTRESS_LIMIT
                ),
            );
        } else {
            self.distress_months = 0;
        }

        if !self.away {
            for r in &mut self.rivals {
                r.payday();
            }
        }

        // Inflation erodes your purchasing power: your cost of living
        // follows the price level every month.
        if player.game_mode == GameMode::SalariedInvestor && !self.away {
            let f = 1.0 + self.economy.inflation.max(-0.02) / 12.0;
            // Rent follows prices too, even while you own (what you'd pay to rent again).
            // Your expenses never include rent while you own, so scale only the rest.
            player.expenses *= f;
            self.rent *= f;
        }
        if now % REVIEW_PERIOD_TICKS == 0 {
            if !self.away {
                for r in &mut self.rivals {
                    r.annual_raise();
                }
            }
            if let Some(yoy) = self.economy.inflation_over(12) {
                self.push_news(
                    now,
                    NewsKind::Market,
                    format!(
                        "Year in review: prices rose {:.1}%, unemployment is {:.1}%, GDP growth {:.1}%.",
                        yoy * 100.0,
                        self.economy.unemployment * 100.0,
                        self.economy.gdp_growth * 100.0
                    ),
                );
            }
        }
        if self.career.employed
            && now >= self.career.next_review_tick
            && self.negotiation.is_none()
            && !self.away
            && player.game_mode == GameMode::SalariedInvestor
        {
            self.start_review(player, rng);
        }
    }

    /// Sells just enough to get cash back to zero: cancels resting buy
    /// orders (they reserve cash), then dumps holdings largest first,
    /// then businesses at a fire-sale discount.
    fn cover_shortfall(
        &mut self,
        player: &mut Player,
        instruments: &[InstrumentState],
        portfolio: &mut Portfolio,
        market: &mut Market,
        fills: &mut Vec<Fill>,
    ) {
        let now = player.current_tick;
        let mut sold: Vec<String> = vec![];
        if player.cash < 0.0 && self.savings > 0.0 {
            let take = (-player.cash).min(self.savings);
            player.cash += take;
            self.savings -= take;
            self.record(now, "withdrawal", "Savings used to cover bills", take);
            sold.push(format!("{take:.0} from savings"));
        }
        if player.cash >= 0.0 {
            self.push_news(now, NewsKind::Goal, format!("Covered the bills from your {}.", sold.join(", ")));
            return;
        }
        market.orders.retain(|o| !o.is_buy);

        let mut held: Vec<(i32, f64, f64)> = portfolio
            .positions
            .values()
            .filter_map(|p| {
                let price = instruments.iter().find(|s| s.instrument.id == p.instrument_id)?.price;
                Some((p.instrument_id, p.quantity, price))
            })
            .collect();
        held.sort_by(|a, b| (b.1 * b.2).partial_cmp(&(a.1 * a.2)).unwrap_or(std::cmp::Ordering::Equal));

        for (id, qty, price) in held {
            if player.cash >= 0.0 {
                break;
            }
            market.cancel_orders_for(id);
            // Sell enough to cover the hole, with a margin for the spread.
            let want = ((-player.cash) / price * 1.02).min(qty);
            if let Ok(mut fill) = market.market_order(player, portfolio, id, false, want) {
                fill.kind = FillKind::Forced;
                self.stats.forced_sales += 1;
                if let Some(s) = instruments.iter().find(|s| s.instrument.id == id) {
                    sold.push(format!("{:.4} {}", fill.quantity, s.instrument.symbol));
                }
                fills.push(fill);
            }
        }

        while player.cash < 0.0 && !self.businesses.is_empty() {
            let b = self.businesses.remove(0);
            let proceeds = b.value() * FIRE_SALE_FACTOR;
            player.cash += proceeds;
            self.record(now, "deal", format!("Fire sale: {}", b.name), proceeds);
            sold.push(format!("{} (fire sale, {proceeds:.0})", b.name));
        }

        if player.cash < 0.0 {
            sold.extend(self.forced_asset_sales(player));
        }

        if player.cash < 0.0 {
            if let Some(h) = self.home.take() {
                let proceeds = h.value(self.economy.home_price_index) * FORCED_SALE_FACTOR - h.mortgage.principal;
                player.cash += proceeds;
                player.expenses += self.rent;
                self.record(now, "home", "Forced sale of your home", proceeds);
                sold.push(format!("your home (forced sale, {proceeds:+.0} after the mortgage)"));
            }
        }

        if !sold.is_empty() {
            self.push_news(now, NewsKind::Goal, format!("Forced to sell to cover bills: {}", sold.join(", ")));
        }
    }

    // ----------------------------------------------------------- events

    fn roll_events<R: Rng>(&mut self, player: &mut Player, rng: &mut R) {
        let now = player.current_tick;

        if rng.gen_bool(MARKET_EVENT_CHANCE) {
            let pool: Vec<&MarketEventDef> = MARKET_EVENTS
                .iter()
                .filter(|d| !self.scheduled.iter().any(|s| s.key == d.key))
                .filter(|d| !self.effects.iter().any(|e| e.label == d.key))
                .collect();
            if let Some(def) = pick_weighted(&pool, |d| d.weight, rng) {
                // Days between the first rumor and the event itself.
                let lead = rng.gen_range(4..=14);
                self.scheduled.push(ScheduledEvent { key: def.key.to_string(), fire_tick: now + lead });
                let broker = &self.npcs[NPC_BROKER];
                let p = ((broker.relationship - 40.0) / 100.0).clamp(0.0, 0.6);
                if p > 0.0 && rng.gen_bool(p) {
                    let text = format!(
                        "{} (broker) calls: \"I'm hearing {}. Take that for what it's worth.\"",
                        broker.name, def.tip
                    );
                    self.push_news(now, NewsKind::Social, text);
                }
            }
        }

        // Brokers sometimes pass along rumors that turn out to be nothing.
        let broker = &self.npcs[NPC_BROKER];
        if broker.relationship >= 55.0 && rng.gen_bool(0.006) {
            let def = &MARKET_EVENTS[rng.gen_range(0..MARKET_EVENTS.len())];
            let text = format!(
                "{} (broker) calls: \"Word is {}. Could be nothing.\"",
                broker.name, def.tip
            );
            self.push_news(now, NewsKind::Social, text);
        }

        if !self.away && rng.gen_bool(PERSONAL_EVENT_CHANCE) {
            let pool: Vec<&PersonalEventDef> = PERSONAL_EVENTS.iter().filter(|d| self.cond_ok(d.cond)).collect();
            // Layoffs follow the labor market.
            let u = self.economy.unemployment / 0.05;
            if let Some(def) = pick_weighted(&pool, |d| if d.key == "layoff" { d.weight * u } else { d.weight }, rng) {
                for e in def.effects {
                    self.apply_effect(*e, player, rng);
                }
                self.push_news(now, NewsKind::Personal, def.headline);
            }
        }

        if !self.away && !self.has_prompt() && rng.gen_bool(DECISION_CHANCE) {
            let pool: Vec<&DecisionDef> = DECISIONS.iter().filter(|d| self.cond_ok(d.cond)).collect();
            if let Some(def) = pick_weighted(&pool, |d| d.weight, rng) {
                self.decision = Some(PendingDecision {
                    key: def.key.to_string(),
                    deadline_tick: now + DECISION_WINDOW_TICKS,
                });
                self.push_news(now, NewsKind::Personal, format!("Decision needed: {}", def.title));
            }
        }
    }

    pub fn apply_effect<R: Rng>(&mut self, effect: Effect, player: &mut Player, rng: &mut R) {
        let now = player.current_tick;
        match effect {
            Effect::Cash(a) => {
                player.cash += a;
                self.record(now, if a >= 0.0 { "windfall" } else { "event_expense" }, "Life event", a);
            }
            Effect::CashSalaryFrac(f) => {
                player.cash += f * player.salary;
                self.record(now, "windfall", "Bonus", f * player.salary);
            }
            Effect::TaxMult(x) => {
                self.economy.govt.tax_mult = (self.economy.govt.tax_mult + x).clamp(0.7, 1.4);
            }
            Effect::ExpenseDelta(d) => player.expenses = (player.expenses + d).max(0.0),
            Effect::RentDelta(d) => {
                self.rent = (self.rent + d).max(0.0);
                if self.home.is_none() {
                    player.expenses = (player.expenses + d).max(0.0);
                }
            }
            Effect::TempExpense { amount, ticks, label } => {
                self.temp_expenses.push(TempExpense { ends_tick: now + ticks, amount, label: label.to_string() })
            }
            Effect::CashLater { amount, ticks, label } => {
                self.scheduled_cash.push(ScheduledCash { due_tick: now + ticks, amount, label: label.to_string() })
            }
            Effect::Skill(d) => self.career.skill = (self.career.skill + d).clamp(0.0, MAX_SKILL),
            Effect::SalaryPct(p) => player.salary *= 1.0 + p,
            Effect::LoseJob { min_ticks, max_ticks } => {
                self.career.employed = false;
                self.career.job_hunting = false;
                self.career.rehire_tick = Some(now + rng.gen_range(min_ticks..=max_ticks));
            }
        }
    }

    /// Resolves the pending decision with the chosen option; returns the
    /// outcome text.
    pub fn resolve_decision<R: Rng>(&mut self, player: &mut Player, option: usize, rng: &mut R) -> String {
        let Some(pending) = self.decision.take() else {
            return "Nothing to decide.".to_string();
        };
        let Some(def) = decision(&pending.key) else {
            return String::new();
        };
        let opt = &def.options[option.min(def.options.len() - 1)];
        let mut roll: f64 = rng.gen_range(0.0..1.0);
        let mut chosen = &opt.outcomes[opt.outcomes.len() - 1].1;
        for (p, outcome) in opt.outcomes {
            if roll < *p {
                chosen = outcome;
                break;
            }
            roll -= p;
        }
        for e in chosen.effects {
            self.apply_effect(*e, player, rng);
        }
        self.stats.decisions_made += 1;
        let text = format!("{}: {}", def.title, chosen.text);
        self.push_news(player.current_tick, NewsKind::Personal, text.clone());
        chosen.text.to_string()
    }

    /// Player picks an option (0-based) for the pending decision.
    pub fn choose_decision<R: Rng>(&mut self, player: &mut Player, option: usize, rng: &mut R) -> Result<String, String> {
        let Some(p) = &self.decision else {
            return Err("No decision is pending.".to_string());
        };
        let n = decision(&p.key).map_or(0, |d| d.options.len());
        if option >= n {
            return Err("No such option.".to_string());
        }
        Ok(self.resolve_decision(player, option, rng))
    }

    // ------------------------------------------------------------ career

    fn start_review<R: Rng>(&mut self, player: &Player, rng: &mut R) {
        let now = player.current_tick;
        let promote = rng.gen_bool(self.career.promotion_chance());
        let proposal = match (promote, self.career.next_title()) {
            (true, Some((_, s))) => s.max(player.salary * 1.05),
            _ => player.salary * (1.0 + self.economy.inflation.max(0.0) + 0.01),
        };
        let boss_name = self.npcs[NPC_BOSS].name.clone();
        let rel_adj = (self.npcs[NPC_BOSS].relationship - 50.0) / 50.0 * 0.03;
        let reservation = proposal * (1.04 + rel_adj + self.career.skill * 0.004).clamp(1.01, 1.25);
        let neg = Negotiation::custom(Side::PlayerSells, proposal * 1.08, reservation, proposal, 3);
        let title = if promote {
            format!("Annual review: promotion to {}", self.career.next_title().map_or("", |t| t.0))
        } else {
            "Annual review: cost-of-living raise".to_string()
        };
        self.career.next_review_tick = now + REVIEW_PERIOD_TICKS;
        self.push_news(
            now,
            NewsKind::Career,
            format!("{}. {} offers {:.0}/mo. You can negotiate.", title, boss_name, proposal),
        );
        self.negotiation = Some(ActiveNegotiation {
            npc: NPC_BOSS,
            purpose: NegPurpose::Salary { promote },
            neg,
            deadline: Some(now + REVIEW_WINDOW_TICKS),
            title,
            log: vec![format!("{} offers {:.0}/mo.", boss_name, proposal)],
        });
    }

    pub fn train(&mut self, player: &mut Player, free_cash: f64) -> Result<String, String> {
        let now = player.current_tick;
        if self.status.is_lost() {
            return Err("The game is over.".to_string());
        }
        if now - self.career.last_training_tick < TRAINING_COOLDOWN_TICKS {
            let wait = TRAINING_COOLDOWN_TICKS - (now - self.career.last_training_tick);
            return Err(format!("You're still digesting the last course. Try again in {wait} days."));
        }
        if self.career.skill >= MAX_SKILL {
            return Err("You've maxed out your skills.".to_string());
        }
        if free_cash < TRAINING_COST {
            return Err(format!("Training costs {TRAINING_COST:.0}; you have {free_cash:.2} free."));
        }
        player.cash -= TRAINING_COST;
        self.record(now, "training", "Training course", -TRAINING_COST);
        self.career.skill = (self.career.skill + 1.0).min(MAX_SKILL);
        self.career.last_training_tick = now;
        let msg = format!("You take a course (-{TRAINING_COST:.0}). Skill is now {:.1}.", self.career.skill);
        self.push_news(now, NewsKind::Career, msg.clone());
        Ok(msg)
    }

    pub fn toggle_job_hunt(&mut self) -> Result<String, String> {
        if self.career.employed {
            return Err("You have a job.".to_string());
        }
        self.career.job_hunting = !self.career.job_hunting;
        Ok(if self.career.job_hunting {
            "You start job hunting: offers will come about twice as fast.".to_string()
        } else {
            "You stop job hunting.".to_string()
        })
    }

    // -------------------------------------------------------------- loans

    pub fn debt(&self) -> f64 {
        self.loan.as_ref().map_or(0.0, |l| l.principal)
    }

    /// The most you could borrow right now.
    pub fn borrowing_room(&self, player: &Player) -> f64 {
        (player.salary * MAX_LOAN_MULTIPLE - self.debt()).max(0.0)
    }

    pub fn borrow(&mut self, player: &mut Player, amount: f64) -> Result<String, String> {
        if self.status.is_lost() {
            return Err("The game is over.".to_string());
        }
        if !(amount.is_finite() && amount > 0.0) {
            return Err("Enter a positive amount.".to_string());
        }
        if !self.career.employed {
            return Err("No bank will lend to you while you're unemployed.".to_string());
        }
        let room = self.borrowing_room(player);
        if amount > room + 1e-9 {
            return Err(format!("The bank will lend you at most {room:.0} more (6 months of salary in total)."));
        }
        let rate = player.interest_rate + LOAN_SPREAD;
        match self.loan.as_mut() {
            Some(l) => {
                let total = l.principal + amount;
                l.annual_rate = (l.principal * l.annual_rate + amount * rate) / total;
                l.principal = total;
            }
            None => self.loan = Some(Loan { principal: amount, annual_rate: rate }),
        }
        player.cash += amount;
        self.record(player.current_tick, "loan_draw", "Loan drawn", amount);
        let l = self.loan.as_ref().expect("loan exists");
        let msg = format!(
            "You borrow {amount:.0} at {:.2}%. Interest plus 2% of the balance comes out every payday.",
            l.annual_rate * 100.0
        );
        self.push_news(player.current_tick, NewsKind::Personal, msg.clone());
        Ok(msg)
    }

    pub fn repay(&mut self, player: &mut Player, amount: f64, free_cash: f64) -> Result<String, String> {
        let Some(loan) = self.loan.as_mut() else {
            return Err("You don't owe anything.".to_string());
        };
        if !(amount.is_finite() && amount > 0.0) {
            return Err("Enter a positive amount.".to_string());
        }
        let pay = amount.min(loan.principal);
        if pay > free_cash + 1e-9 {
            return Err(format!("You only have {free_cash:.2} free."));
        }
        player.cash -= pay;
        loan.principal -= pay;
        let tick = player.current_tick;
        let remaining = loan.principal;
        let msg = if remaining < 1.0 {
            self.loan = None;
            format!("Loan repaid in full ({pay:.0}).")
        } else {
            format!("You repay {pay:.0}. You still owe {remaining:.0}.")
        };
        self.record(tick, "loan_principal", "Loan repayment", -pay);
        self.push_news(player.current_tick, NewsKind::Personal, msg.clone());
        Ok(msg)
    }

    // ----------------------------------------------------------- savings

    /// What `balance` in the bank grows to after `years` if left alone at a
    /// constant policy rate: interest paid monthly at (rate - spread), taxed
    /// at 15% and compounded.
    pub fn project_savings(balance: f64, policy_rate: f64, years: f64) -> f64 {
        let net_monthly = (policy_rate - SAVINGS_SPREAD).max(0.0) / 12.0 * (1.0 - INVESTMENT_INCOME_RATE);
        balance * (1.0 + net_monthly).powf(12.0 * years)
    }

    /// Interest you would receive at the next payday, after tax.
    pub fn next_savings_interest(&self, policy_rate: f64) -> f64 {
        self.savings * (policy_rate - SAVINGS_SPREAD).max(0.0) / 12.0 * (1.0 - INVESTMENT_INCOME_RATE)
    }

    /// Moves the value of any holdings of the retired SAVE instrument into
    /// the bank account. Returns the amount moved.
    pub fn absorb_retired_savings(&mut self, value: f64, now: i64) -> f64 {
        if value < 0.5 {
            return 0.0;
        }
        self.savings += value;
        self.push_news(
            now,
            NewsKind::Personal,
            format!("The old High-Yield Savings fund is now your bank savings account: {value:.0} moved over."),
        );
        value
    }

    pub fn deposit(&mut self, player: &mut Player, amount: f64, free_cash: f64) -> Result<String, String> {
        if self.status.is_lost() {
            return Err("The game is over.".to_string());
        }
        if !(amount.is_finite() && amount > 0.0) {
            return Err("Enter a positive amount.".to_string());
        }
        if amount > free_cash + 1e-9 {
            return Err(format!("You only have {free_cash:.2} free."));
        }
        player.cash -= amount;
        self.savings += amount;
        self.record(player.current_tick, "deposit", "Deposit to savings", -amount);
        Ok(format!(
            "Deposited {amount:.0}. It earns {:.2}% (policy rate minus {:.1}%), safe from the market.",
            (player.interest_rate - SAVINGS_SPREAD).max(0.0) * 100.0,
            SAVINGS_SPREAD * 100.0
        ))
    }

    pub fn withdraw(&mut self, player: &mut Player, amount: f64) -> Result<String, String> {
        if !(amount.is_finite() && amount > 0.0) {
            return Err("Enter a positive amount.".to_string());
        }
        if self.savings <= 0.0 {
            return Err("Your savings account is empty.".to_string());
        }
        let take = amount.min(self.savings);
        self.savings -= take;
        player.cash += take;
        self.record(player.current_tick, "withdrawal", "Withdrawal from savings", take);
        Ok(format!("Withdrew {take:.0}. Savings balance: {:.0}.", self.savings))
    }

    // -------------------------------------------------------- businesses

    fn refresh_listings<R: Rng>(&mut self, now: i64, rng: &mut R) {
        self.listings.retain(|l| l.expires_tick > now);
        while self.listings.len() < 3 {
            let kind = rng.gen_range(0..BUSINESS_KINDS.len());
            let id = self.alloc_id();
            self.listings.push(Listing {
                id,
                kind,
                monthly_profit: BUSINESS_KINDS[kind].base_profit * rng.gen_range(0.8..1.2),
                expires_tick: now + LISTING_LIFETIME_TICKS,
            });
        }
        self.home_listings.retain(|l| l.expires_tick > now);
        while self.home_listings.len() < 3 {
            let kind = rng.gen_range(0..HOME_KINDS.len());
            let id = self.alloc_id();
            let price = HOME_KINDS[kind].1 * self.economy.home_price_index / 100.0 * rng.gen_range(0.92..1.08);
            self.home_listings.push(HomeListing { id, kind, price, expires_tick: now + HOME_LISTING_LIFETIME_TICKS });
        }
        self.next_listing_tick = now + LISTING_REFRESH_TICKS;
    }

    pub fn upgrade_business(&mut self, player: &mut Player, index: usize, free_cash: f64) -> Result<String, String> {
        let now = player.current_tick;
        let Some(b) = self.businesses.get_mut(index) else {
            return Err("No such business.".to_string());
        };
        if b.efficiency >= MAX_EFFICIENCY - 1e-9 {
            return Err(format!("{} is already as efficient as it gets.", b.name));
        }
        let cost = b.upgrade_cost();
        if free_cash < cost {
            return Err(format!("Upgrading costs {cost:.0}; you have {free_cash:.2} free."));
        }
        player.cash -= cost;
        let name = b.name.clone();
        b.efficiency = (b.efficiency + UPGRADE_EFFICIENCY_GAIN).min(MAX_EFFICIENCY);
        self.record(now, "business_invest", format!("Upgrade {name}"), -cost);
        let b = &self.businesses[index];
        let msg = format!("You invest {cost:.0} in {}. Efficiency now {:.0}%.", b.name, b.efficiency * 100.0);
        self.push_news(now, NewsKind::Deal, msg.clone());
        Ok(msg)
    }

    // ----------------------------------------------------------- housing

    pub fn start_buy_home<R: Rng>(&mut self, player: &Player, free_cash: f64, index: usize, rng: &mut R) -> Result<String, String> {
        self.check_can_start()?;
        self.ensure_compat(player);
        if self.home.is_some() {
            return Err("You already own a home. Sell it first.".to_string());
        }
        if !self.career.employed {
            return Err("No bank will give you a mortgage while you're out of work.".to_string());
        }
        let Some(l) = self.home_listings.get(index) else {
            return Err("No such listing.".to_string());
        };
        // Haggling can't get the price below its fair value, so if you can't
        // afford that, there's no point sitting down.
        self.check_buy_limits(&NegPurpose::BuyHome { listing_id: l.id }, l.price, free_cash, player)?;
        let neg = Negotiation::open(Side::PlayerBuys, l.price, self.npcs[NPC_REALTOR].relationship, rng);
        let opening = format!("{} asks {:.0} for the {}.", self.npcs[NPC_REALTOR].name, neg.counter, l.name());
        self.negotiation = Some(ActiveNegotiation {
            npc: NPC_REALTOR,
            purpose: NegPurpose::BuyHome { listing_id: l.id },
            neg,
            deadline: None,
            title: format!("Buy a home: {}", l.name()),
            log: vec![opening.clone()],
        });
        Ok(opening)
    }

    /// Sells your home on the market (6% selling costs) and goes back to renting.
    pub fn sell_home(&mut self, player: &mut Player) -> Result<String, String> {
        if self.status.is_lost() {
            return Err("The game is over.".to_string());
        }
        let Some(h) = self.home.as_ref() else {
            return Err("You don't own a home.".to_string());
        };
        let value = h.value(self.economy.home_price_index);
        let proceeds = value * (1.0 - SELLING_COST) - h.mortgage.principal;
        if player.cash + proceeds < 0.0 {
            return Err(format!("The sale wouldn't cover your mortgage: you'd need another {:.0} in cash.", -(player.cash + proceeds)));
        }
        self.home = None;
        player.cash += proceeds;
        player.expenses += self.rent;
        let now = player.current_tick;
        self.record(now, "home", "Sold your home", proceeds);
        let msg = format!("You sell your home for {value:.0}, clear the mortgage and net {proceeds:.0}. You're renting again at {:.0}/mo.", self.rent);
        self.push_news(now, NewsKind::Deal, msg.clone());
        Ok(msg)
    }

    // ------------------------------------------------------ negotiations

    /// Can you actually pay this price?
    fn check_buy_limits(&self, purpose: &NegPurpose, price: f64, free_cash: f64, player: &Player) -> Result<(), String> {
        match purpose {
            NegPurpose::BuyBusiness { .. } if price > free_cash + 1e-9 => Err(format!("You only have {free_cash:.2} free.")),
            NegPurpose::BuyHome { .. } => {
                let need = price * (DOWN_PAYMENT + CLOSING_COST);
                if need > free_cash + 1e-9 {
                    return Err(format!(
                        "At {price:.0} you need {need:.0} in cash ({:.0}% down plus {:.0}% closing costs); you have {free_cash:.0} free.",
                        DOWN_PAYMENT * 100.0,
                        CLOSING_COST * 100.0
                    ));
                }
                let rate = player.interest_rate + MORTGAGE_SPREAD;
                let pmt = mortgage_payment(price * (1.0 - DOWN_PAYMENT), rate);
                if pmt > MAX_PAYMENT_TO_INCOME * player.salary {
                    return Err(format!(
                        "The bank won't lend that: the payment of {pmt:.0}/mo is over {:.0}% of your salary.",
                        MAX_PAYMENT_TO_INCOME * 100.0
                    ));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn check_can_start(&self) -> Result<(), String> {
        if self.status.is_lost() {
            return Err("The game is over.".to_string());
        }
        if self.has_prompt() {
            return Err("Finish what's pending first.".to_string());
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn start_forward<R: Rng>(
        &mut self,
        player: &Player,
        free_cash: f64,
        inst: &InstrumentState,
        is_long: bool,
        quantity: f64,
        horizon: i64,
        rng: &mut R,
    ) -> Result<String, String> {
        self.check_can_start()?;
        if matches!(inst.instrument.class, AssetClass::Savings) {
            return Err("Savings can't be traded forward.".to_string());
        }
        if !(quantity.is_finite() && quantity > 0.0) {
            return Err("Invalid quantity.".to_string());
        }
        let margin = FORWARD_MARGIN * quantity * inst.price;
        if free_cash < margin {
            return Err(format!("Forwards need {margin:.2} of free cash as margin; you have {free_cash:.2}."));
        }
        let fair = inst.price * (1.0 + player.interest_rate * horizon as f64 / 365.0);
        let side = if is_long { Side::PlayerBuys } else { Side::PlayerSells };
        let neg = Negotiation::open(side, fair, self.npcs[NPC_BROKER].relationship, rng);
        let title = format!(
            "Forward: {} {} {} in {} days",
            if is_long { "buy" } else { "sell" },
            quantity,
            inst.instrument.symbol,
            horizon
        );
        let opening = format!(
            "{} quotes a strike of {:.4} (spot {:.4}).",
            self.npcs[NPC_BROKER].name, neg.counter, inst.price
        );
        self.negotiation = Some(ActiveNegotiation {
            npc: NPC_BROKER,
            purpose: NegPurpose::Forward {
                instrument_id: inst.instrument.id,
                symbol: inst.instrument.symbol.clone(),
                is_long,
                quantity,
                horizon,
            },
            neg,
            deadline: None,
            title,
            log: vec![opening.clone()],
        });
        Ok(opening)
    }

    pub fn start_buy_business<R: Rng>(&mut self, index: usize, rng: &mut R) -> Result<String, String> {
        self.check_can_start()?;
        let Some(l) = self.listings.get(index) else {
            return Err("No such listing.".to_string());
        };
        let npc = l.kind().owner_npc;
        let neg = Negotiation::open(Side::PlayerBuys, l.fair_price(), self.npcs[npc].relationship, rng);
        let opening = format!("{} asks {:.0} for the {}.", self.npcs[npc].name, neg.counter, l.kind().name);
        self.negotiation = Some(ActiveNegotiation {
            npc,
            purpose: NegPurpose::BuyBusiness { listing_id: l.id },
            neg,
            deadline: None,
            title: format!("Buy {}", l.kind().name),
            log: vec![opening.clone()],
        });
        Ok(opening)
    }

    pub fn start_sell_business<R: Rng>(&mut self, index: usize, rng: &mut R) -> Result<String, String> {
        self.check_can_start()?;
        let Some(b) = self.businesses.get(index) else {
            return Err("No such business.".to_string());
        };
        let npc = b.kind().owner_npc;
        let neg = Negotiation::open(Side::PlayerSells, b.value(), self.npcs[npc].relationship, rng);
        let opening = format!("{} offers {:.0} for your {}.", self.npcs[npc].name, neg.counter, b.name);
        self.negotiation = Some(ActiveNegotiation {
            npc,
            purpose: NegPurpose::SellBusiness { business_id: b.id },
            neg,
            deadline: None,
            title: format!("Sell {}", b.name),
            log: vec![opening.clone()],
        });
        Ok(opening)
    }

    /// Makes an offer in the active negotiation.
    pub fn negotiate_offer<R: Rng>(
        &mut self,
        player: &mut Player,
        instruments: &[InstrumentState],
        free_cash: f64,
        price: f64,
        rng: &mut R,
    ) -> Result<NegStep, String> {
        let _ = rng;
        if !(price.is_finite() && price > 0.0) {
            return Err("Enter a positive price.".to_string());
        }
        let Some(mut n) = self.negotiation.take() else {
            return Err("No negotiation in progress.".to_string());
        };
        if let Err(e) = self.check_buy_limits(&n.purpose, price, free_cash, player) {
            self.negotiation = Some(n);
            return Err(e);
        }
        let now = player.current_tick;
        let npc_name = self.npcs[n.npc].name.clone();
        match n.neg.offer(price) {
            Response::Accepted { price } => Ok(NegStep::Done(self.finalize(n, price, player, instruments))),
            Response::Counter { price: counter, patience, insulted } => {
                let msg = format!(
                    "{}{} counters at {:.4}. (patience {})",
                    if insulted { "That's insulting! " } else { "" },
                    npc_name,
                    counter,
                    patience
                );
                if insulted {
                    self.npcs[n.npc].adjust(-2.0);
                }
                n.log.push(format!("You: {price:.4}"));
                n.log.push(msg.clone());
                self.negotiation = Some(n);
                Ok(NegStep::Continue(msg))
            }
            Response::WalkedAway => {
                self.npcs[n.npc].adjust(-6.0);
                let msg = format!("{npc_name} loses patience and walks away. The relationship suffers.");
                self.push_news(now, NewsKind::Deal, msg.clone());
                Ok(NegStep::Done(msg))
            }
        }
    }

    pub fn negotiate_accept<R: Rng>(
        &mut self,
        player: &mut Player,
        instruments: &[InstrumentState],
        free_cash: f64,
        rng: &mut R,
    ) -> Result<NegStep, String> {
        let Some(n) = &self.negotiation else {
            return Err("No negotiation in progress.".to_string());
        };
        let price = n.neg.accept_counter();
        self.negotiate_offer_at(player, instruments, free_cash, price, rng)
    }

    /// Taking the NPC's own standing offer always succeeds.
    fn negotiate_offer_at<R: Rng>(
        &mut self,
        player: &mut Player,
        instruments: &[InstrumentState],
        free_cash: f64,
        price: f64,
        _rng: &mut R,
    ) -> Result<NegStep, String> {
        let Some(n) = self.negotiation.take() else {
            return Err("No negotiation in progress.".to_string());
        };
        if let Err(e) = self.check_buy_limits(&n.purpose, price, free_cash, player) {
            self.negotiation = Some(n);
            return Err(e);
        }
        Ok(NegStep::Done(self.finalize(n, price, player, instruments)))
    }

    /// Walk away from the table. Salary reviews fall back to no raise.
    pub fn negotiate_walk(&mut self, player: &Player) -> String {
        let Some(n) = self.negotiation.take() else {
            return String::new();
        };
        self.npcs[n.npc].adjust(-1.0);
        let msg = match n.purpose {
            NegPurpose::Salary { .. } => format!("You turn down the offer. {} shrugs; no raise this year.", self.npcs[n.npc].name),
            _ => format!("You walk away from {}.", self.npcs[n.npc].name),
        };
        self.push_news(player.current_tick, NewsKind::Deal, msg.clone());
        msg
    }

    fn finalize(&mut self, n: ActiveNegotiation, price: f64, player: &mut Player, instruments: &[InstrumentState]) -> String {
        let now = player.current_tick;
        let npc_name = self.npcs[n.npc].name.clone();
        let msg = match n.purpose {
            NegPurpose::Forward { instrument_id, symbol, is_long, quantity, horizon } => {
                let id = self.alloc_id();
                let _ = instruments;
                self.contracts.push(Forward {
                    id,
                    instrument_id,
                    symbol: symbol.clone(),
                    is_long,
                    quantity,
                    strike: price,
                    settle_tick: now + horizon,
                });
                format!(
                    "Deal! {} {} {} at {:.4}, settling in {} days.",
                    if is_long { "Long" } else { "Short" },
                    quantity,
                    symbol,
                    price,
                    horizon
                )
            }
            NegPurpose::BuyBusiness { listing_id } => {
                if let Some(pos) = self.listings.iter().position(|l| l.id == listing_id) {
                    let l = self.listings.remove(pos);
                    player.cash -= price;
                    self.record(now, "deal", format!("Bought {}", l.kind().name), -price);
                    let id = self.alloc_id();
                    let name = l.kind().name.to_string();
                    self.businesses.push(Business {
                        id,
                        kind: l.kind,
                        name: name.clone(),
                        monthly_profit: l.monthly_profit,
                        efficiency: 1.0,
                        bought_tick: now,
                        price_paid: price,
                    });
                    format!("Deal! You buy the {name} for {price:.0}.")
                } else {
                    "The listing is gone.".to_string()
                }
            }
            NegPurpose::BuyHome { listing_id } => {
                if let Some(pos) = self.home_listings.iter().position(|l| l.id == listing_id) {
                    let l = self.home_listings.remove(pos);
                    let cash_needed = price * (DOWN_PAYMENT + CLOSING_COST);
                    let principal = price * (1.0 - DOWN_PAYMENT);
                    let rate = player.interest_rate + MORTGAGE_SPREAD;
                    player.cash -= cash_needed;
                    player.expenses = (player.expenses - self.rent).max(0.0);
                    self.home = Some(Home {
                        name: l.name().to_string(),
                        bought_tick: now,
                        price_paid: price,
                        hpi_at_purchase: self.economy.home_price_index,
                        mortgage: Mortgage { principal, annual_rate: rate, payment: mortgage_payment(principal, rate) },
                    });
                    self.stats.home_purchases += 1;
                    self.record(now, "home", format!("Bought a {}: down payment and closing costs", l.name()), -cash_needed);
                    let pmt = mortgage_payment(principal, rate);
                    format!(
                        "Deal! You buy the {} for {price:.0}: {cash_needed:.0} up front, then {pmt:.0}/mo mortgage at {:.2}% plus upkeep. No more rent.",
                        l.name(),
                        rate * 100.0
                    )
                } else {
                    "The listing is gone.".to_string()
                }
            }
            NegPurpose::SellBusiness { business_id } => {
                if let Some(pos) = self.businesses.iter().position(|b| b.id == business_id) {
                    let b = self.businesses.remove(pos);
                    player.cash += price;
                    self.record(now, "deal", format!("Sold {}", b.name), price);
                    format!("Deal! You sell {} for {price:.0} (you paid {:.0}).", b.name, b.price_paid)
                } else {
                    "The business is gone.".to_string()
                }
            }
            NegPurpose::Salary { promote } => {
                let gain = (price - player.salary).max(0.0);
                player.salary = price;
                let mut creep = String::new();
                if promote && self.career.title_idx + 1 < LADDER.len() {
                    self.career.title_idx += 1;
                    // Lifestyle creep: a better title brings a nicer place,
                    // better clothes, more eating out.
                    let extra = gain * LIFESTYLE_CREEP;
                    player.expenses += extra;
                    creep = format!(" Your lifestyle grows with it: expenses +{extra:.0}/mo.");
                }
                format!("Agreed: {:.0}/mo as {}.{creep}", price, self.career.title())
            }
        };
        self.npcs[n.npc].adjust(3.0);
        self.stats.deals_done += 1;
        self.push_news(now, NewsKind::Deal, format!("{npc_name}: {msg}"));
        msg
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::{asset_price, purchase_terms, MAX_ASSETS};
    use crate::engine::{fast_forward_with_rng, LONG_RUN_INTEREST_RATE};
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    fn inst(id: i32, symbol: &str, class: AssetClass, price: f64) -> InstrumentState {
        InstrumentState {
            instrument: Instrument {
                id,
                symbol: symbol.to_string(),
                name: symbol.to_string(),
                class,
                base_price: price,
                drift: 0.0,
                volatility: 0.0,
                face_value: 0.0,
                coupon_rate: 0.0,
                duration: 0.0,
                income_yield: 0.0,
                rate_sensitivity: 0.0,
                expense_ratio: 0.0,
                constituents: vec![],
            },
            price,
        }
    }

    fn player(cash: f64) -> Player {
        Player {
            id: 1,
            game_mode: GameMode::SalariedInvestor,
            cash,
            salary: 4_000.0,
            expenses: 3_200.0,
            current_tick: 0,
            interest_rate: LONG_RUN_INTEREST_RATE,
        }
    }

    struct World {
        p: Player,
        i: Vec<InstrumentState>,
        pf: Portfolio,
        m: Market,
        g: GameState,
        rng: StdRng,
    }

    fn world(cash: f64, events: bool) -> World {
        let p = player(cash);
        let i = vec![
            inst(1, "OIL", AssetClass::Commodity, 100.0),
            inst(2, "IDX", AssetClass::IndexFund, 100.0),
        ];
        let m = Market::new(&i, vec![]);
        let g = GameState::new(&p, 2_000.0, events);
        World { p, i, pf: Portfolio::default(), m, g, rng: StdRng::seed_from_u64(9) }
    }

    fn run(w: &mut World, ticks: i64) -> crate::engine::TickOutput {
        fast_forward_with_rng(&mut w.p, &mut w.i, &mut w.pf, &mut w.m, &mut w.g, ticks, false, &mut w.rng)
    }

    #[test]
    fn payday_pays_salary_less_expenses() {
        let mut w = world(1_000.0, false);
        run(&mut w, 30);
        let expected = 1_000.0 + 800.0 - monthly_withholding(4_000.0, 1.0);
        assert!((w.p.cash - expected).abs() < 1e-6, "{}", w.p.cash);
    }

    #[test]
    fn unemployment_pays_only_a_benefit() {
        let mut w = world(10_000.0, false);
        w.g.career.employed = false;
        w.g.career.rehire_tick = Some(10_000);
        run(&mut w, 30);
        let expected = 10_000.0 + 4_000.0 * BENEFIT_RATE - 3_200.0;
        assert!((w.p.cash - expected).abs() < 1e-6, "{}", w.p.cash);
    }

    #[test]
    fn rehired_after_the_rehire_tick() {
        let mut w = world(10_000.0, false);
        w.g.career.employed = false;
        w.g.career.rehire_tick = Some(5);
        run(&mut w, 6);
        assert!(w.g.career.employed);
    }

    #[test]
    fn shortfall_forces_a_sale_of_holdings() {
        let mut w = world(100.0, false);
        w.p.expenses = 6_000.0; // -2,000/month
        w.pf.buy(2, 100.0, 100.0); // 10,000 of IDX
        run(&mut w, 30);
        assert!(w.p.cash >= 0.0, "cash {}", w.p.cash);
        assert!(w.pf.quantity_of(2) < 100.0);
        assert!(w.g.news.iter().any(|n| n.text.contains("Forced to sell")));
    }

    #[test]
    fn three_missed_paydays_means_bankruptcy() {
        let mut w = world(0.0, false);
        w.p.expenses = 6_000.0; // nothing to sell
        run(&mut w, 30 * 3);
        assert!(w.g.status.is_lost(), "{:?}", w.g.status);
        // The game is frozen afterwards.
        let tick = w.p.current_tick;
        let out = run(&mut w, 50);
        assert_eq!(w.p.current_tick, tick);
        assert_eq!(out.ticks_run, 0);
    }

    #[test]
    fn covering_the_bills_resets_distress() {
        let mut w = world(0.0, false);
        w.p.expenses = 6_000.0;
        run(&mut w, 30);
        assert_eq!(w.g.distress_months, 1);
        w.p.cash = 5_000.0;
        w.p.expenses = 3_000.0;
        run(&mut w, 30);
        assert_eq!(w.g.distress_months, 0);
    }

    #[test]
    fn scheduled_event_shocks_prices_and_leaves_an_effect() {
        let mut w = world(1_000.0, false);
        w.g.scheduled.push(ScheduledEvent { key: "oil_spike".into(), fire_tick: 3 });
        run(&mut w, 2);
        assert_eq!(w.i[0].price, 100.0);
        run(&mut w, 1);
        // +18% jump, plus a sliver of upward pressure from the new supply shortage.
        assert!(w.i[0].price >= 118.0 && w.i[0].price < 118.5, "{}", w.i[0].price);
        assert!(w.g.economy.price_pressure("OIL") > 0.1, "the spike also created a shortage");
        assert!(w.g.news.iter().any(|n| n.text.contains("oil")));
        assert_eq!(w.g.effects.len(), 1);
        assert!(w.g.vol_mult(&w.i[0].instrument) > 1.0);
        run(&mut w, 40);
        assert!(w.g.effects.is_empty(), "effect should have expired");
    }

    #[test]
    fn rate_shock_moves_the_prevailing_rate() {
        let mut w = world(1_000.0, false);
        w.g.scheduled.push(ScheduledEvent { key: "rate_hike".into(), fire_tick: 1 });
        run(&mut w, 1);
        assert!(w.p.interest_rate > LONG_RUN_INTEREST_RATE + 0.01, "{}", w.p.interest_rate);
    }

    #[test]
    fn decisions_apply_their_effects() {
        let mut w = world(5_000.0, false);
        w.g.decision = Some(PendingDecision { key: "rent_increase".into(), deadline_tick: 99 });
        let before = w.p.expenses;
        w.g.choose_decision(&mut w.p, 0, &mut w.rng).unwrap();
        assert_eq!(w.p.expenses, before + 100.0);
        assert!(w.g.decision.is_none());
        assert!(w.g.choose_decision(&mut w.p, 0, &mut w.rng).is_err());
    }

    #[test]
    fn unanswered_decisions_resolve_to_their_default() {
        let mut w = world(5_000.0, false);
        w.g.decision = Some(PendingDecision { key: "rent_increase".into(), deadline_tick: 5 });
        run(&mut w, 5);
        assert!(w.g.decision.is_none());
        assert!(w.p.expenses > 3_200.0);
        assert!(w.g.news.iter().any(|n| n.text.contains("didn't decide in time")));
    }

    #[test]
    fn decision_outcomes_follow_their_probabilities() {
        // "Negotiate" on rent is a coin flip: over many tries both occur.
        let mut hits = 0;
        let mut rng = StdRng::seed_from_u64(1);
        for _ in 0..200 {
            let mut p = player(0.0);
            let mut g = GameState::quiet(&p);
            g.decision = Some(PendingDecision { key: "rent_increase".into(), deadline_tick: 99 });
            g.resolve_decision(&mut p, 1, &mut rng);
            if p.expenses == 3_200.0 {
                hits += 1;
            }
        }
        assert!(hits > 70 && hits < 130, "hits {hits}");
    }

    #[test]
    fn delayed_cash_arrives_on_time() {
        let mut w = world(0.0, false);
        w.g.scheduled_cash.push(ScheduledCash { due_tick: 4, amount: 900.0, label: "payout".into() });
        run(&mut w, 3);
        assert_eq!(w.p.cash, 0.0);
        run(&mut w, 1);
        assert_eq!(w.p.cash, 900.0);
    }

    #[test]
    fn layoff_effect_stops_paychecks() {
        let mut w = world(1_000.0, false);
        w.g.apply_effect(Effect::LoseJob { min_ticks: 100, max_ticks: 100 }, &mut w.p, &mut StdRng::seed_from_u64(1));
        assert!(!w.g.career.employed);
        assert_eq!(w.g.career.rehire_tick, Some(100));
    }

    #[test]
    fn annual_review_opens_a_negotiation_that_resolves_by_default() {
        let mut w = world(1_000.0, false);
        let before = w.p.salary;
        run(&mut w, 360);
        let n = w.g.negotiation.as_ref().expect("review should be pending");
        assert!(matches!(n.purpose, NegPurpose::Salary { .. }));
        run(&mut w, REVIEW_WINDOW_TICKS);
        assert!(w.g.negotiation.is_none());
        assert!(w.p.salary > before, "salary {}", w.p.salary);
    }

    #[test]
    fn haggling_over_a_raise_can_beat_the_default_offer() {
        let mut w = world(1_000.0, false);
        w.g.career.next_review_tick = 0;
        run(&mut w, 30);
        let proposal = w.g.negotiation.as_ref().unwrap().neg.counter;
        let reservation = w.g.negotiation.as_ref().unwrap().neg.reservation;
        assert!(reservation > proposal);
        let step = w.g.negotiate_offer(&mut w.p, &w.i, 0.0, reservation, &mut w.rng).unwrap();
        assert!(matches!(step, NegStep::Done(_)));
        assert!((w.p.salary - reservation).abs() < 1e-6);
        assert!(w.g.npcs[NPC_BOSS].relationship > 50.0);
    }

    #[test]
    fn overreaching_walks_the_boss_away() {
        let mut w = world(1_000.0, false);
        w.g.career.next_review_tick = 0;
        run(&mut w, 30);
        let before = w.p.salary;
        let mut done = false;
        for _ in 0..4 {
            if let Ok(NegStep::Done(_)) = w.g.negotiate_offer(&mut w.p, &w.i, 0.0, before * 3.0, &mut w.rng) {
                done = true;
                break;
            }
        }
        assert!(done);
        assert_eq!(w.p.salary, before);
        assert!(w.g.npcs[NPC_BOSS].relationship < 50.0);
    }

    #[test]
    fn forward_contract_full_lifecycle() {
        let mut w = world(10_000.0, false);
        let oil = w.i[0].clone();
        w.g.start_forward(&w.p, 10_000.0, &oil, true, 10.0, 30, &mut w.rng).unwrap();
        // Can't start a second thing while one is pending.
        assert!(w.g.start_forward(&w.p, 10_000.0, &oil, true, 1.0, 30, &mut w.rng).is_err());
        let step = w.g.negotiate_accept(&mut w.p, &w.i, 10_000.0, &mut w.rng).unwrap();
        assert!(matches!(step, NegStep::Done(_)));
        assert_eq!(w.g.contracts.len(), 1);
        let strike = w.g.contracts[0].strike;
        assert!(strike > 100.0); // above spot: includes carry and the broker's margin

        // Oil jumps; the long forward is worth more, in net worth too.
        w.i[0].price = 130.0;
        let prices: HashMap<i32, f64> = w.i.iter().map(|s| (s.instrument.id, s.price)).collect();
        assert!((w.g.assets_value(&prices) - 10.0 * (130.0 - strike)).abs() < 1e-6);

        // Settles in cash at maturity.
        let cash_before = w.p.cash;
        w.i[0].instrument.volatility = 0.0;
        w.p.current_tick = 29;
        run(&mut w, 1);
        assert!(w.g.contracts.is_empty());
        assert!(w.p.cash > cash_before + 1.0 || w.p.cash < cash_before - 1.0);
    }

    #[test]
    fn forward_needs_margin() {
        let mut w = world(10_000.0, false);
        let oil = w.i[0].clone();
        assert!(w.g.start_forward(&w.p, 50.0, &oil, true, 10.0, 30, &mut w.rng).is_err());
    }

    #[test]
    fn buying_a_business_and_collecting_profit() {
        let mut w = world(100_000.0, false);
        run(&mut w, 1);
        assert!(!w.g.listings.is_empty());
        let fair = w.g.listings[0].fair_price();
        w.g.start_buy_business(0, &mut w.rng).unwrap();
        // Can't pay more than you have.
        assert!(w.g.negotiate_offer(&mut w.p, &w.i, 10.0, 50_000.0, &mut w.rng).is_err());
        let step = w.g.negotiate_accept(&mut w.p, &w.i, 100_000.0, &mut w.rng).unwrap();
        assert!(matches!(step, NegStep::Done(_)));
        assert_eq!(w.g.businesses.len(), 1);
        assert!(w.p.cash < 100_000.0);
        assert!((w.g.businesses[0].value() - fair).abs() < 1e-6);
        let cash = w.p.cash;
        let tick = w.p.current_tick;
        let to_payday = 30 - tick % 30;
        run(&mut w, to_payday);
        assert!(w.g.last_business_income != 0.0);
        // Salary - expenses + profit landed.
        let net_business = w.g.last_business_income - w.g.last_business_income.max(0.0) * BUSINESS_PROFIT_RATE;
        let expected = cash + 800.0 - monthly_withholding(4_000.0, 1.0) + net_business;
        assert!((w.p.cash - expected).abs() < 1e-6, "{} vs {}", w.p.cash, expected);
    }

    #[test]
    fn selling_a_business_pays_out() {
        let mut w = world(100_000.0, false);
        run(&mut w, 1);
        w.g.start_buy_business(0, &mut w.rng).unwrap();
        w.g.negotiate_accept(&mut w.p, &w.i, 100_000.0, &mut w.rng).unwrap();
        let cash = w.p.cash;
        w.g.start_sell_business(0, &mut w.rng).unwrap();
        w.g.negotiate_accept(&mut w.p, &w.i, 0.0, &mut w.rng).unwrap();
        assert!(w.g.businesses.is_empty());
        assert!(w.p.cash > cash);
    }

    #[test]
    fn upgrades_cost_money_and_raise_efficiency() {
        let mut w = world(100_000.0, false);
        run(&mut w, 1);
        w.g.start_buy_business(0, &mut w.rng).unwrap();
        w.g.negotiate_accept(&mut w.p, &w.i, 100_000.0, &mut w.rng).unwrap();
        let v = w.g.businesses[0].value();
        w.g.upgrade_business(&mut w.p, 0, 100_000.0).unwrap();
        assert!(w.g.businesses[0].value() > v);
        assert!(w.g.upgrade_business(&mut w.p, 0, 1.0).is_err());
    }

    #[test]
    fn fire_sale_covers_bills_when_nothing_else_is_left() {
        let mut w = world(100_000.0, false);
        run(&mut w, 1);
        w.g.start_buy_business(0, &mut w.rng).unwrap();
        w.g.negotiate_accept(&mut w.p, &w.i, 100_000.0, &mut w.rng).unwrap();
        w.p.cash = 0.0;
        w.p.expenses = 6_000.0;
        let to_payday = 30 - w.p.current_tick % 30;
        run(&mut w, to_payday);
        assert!(w.g.businesses.is_empty());
        assert!(w.p.cash >= 0.0);
    }

    #[test]
    fn training_costs_cash_and_has_a_cooldown() {
        let mut w = world(5_000.0, false);
        w.p.current_tick = 200;
        w.g.train(&mut w.p, 5_000.0).unwrap();
        assert_eq!(w.g.career.skill, 1.0);
        assert_eq!(w.p.cash, 5_000.0 - TRAINING_COST);
        assert!(w.g.train(&mut w.p, 5_000.0).is_err());
        w.p.current_tick += TRAINING_COOLDOWN_TICKS;
        assert!(w.g.train(&mut w.p, 5_000.0).is_ok());
        assert!(w.g.train(&mut w.p, 1.0).is_err());
    }

    #[test]
    fn skill_improves_promotion_odds() {
        let mut c = Career::new(4_000.0, 0);
        let low = c.promotion_chance();
        c.skill = 8.0;
        assert!(c.promotion_chance() > low);
        c.title_idx = LADDER.len() - 1;
        assert_eq!(c.promotion_chance(), 0.0);
    }

    #[test]
    fn financial_independence_wins_and_play_continues() {
        let mut w = world(1_000_000.0, false);
        w.p.expenses = 3_200.0;
        run(&mut w, 1);
        assert!(w.g.status.is_won());
        let t = w.p.current_tick;
        run(&mut w, 5);
        assert_eq!(w.p.current_tick, t + 5);
    }

    #[test]
    fn milestones_are_announced_once() {
        let mut w = world(30_000.0, false);
        run(&mut w, 3);
        // The test world starts the game at 2k net worth, so 10k and 25k fire now.
        let count = |w: &World| w.g.news.iter().filter(|n| n.text.contains("Milestone")).count();
        assert_eq!(count(&w), 2);
        w.p.cash = 60_000.0;
        run(&mut w, 2);
        assert_eq!(count(&w), 3);
        run(&mut w, 2);
        assert_eq!(count(&w), 3);
    }

    #[test]
    fn rivals_track_the_market_and_save() {
        let mut w = world(1_000.0, false);
        let before: Vec<f64> = w.g.rivals.iter().map(|r| r.wealth).collect();
        w.i[1].instrument.drift = 0.0;
        run(&mut w, 60);
        for (r, b) in w.g.rivals.iter().zip(before) {
            assert!(r.wealth > b, "{} should have saved something", r.name);
        }
        let board = w.g.leaderboard(1_000.0);
        assert_eq!(board.len(), 4);
        assert!(board.windows(2).all(|p| p[0].1 >= p[1].1));
        assert_eq!(board.iter().filter(|e| e.2).count(), 1);
    }

    #[test]
    fn random_events_eventually_happen_and_never_break_the_economy() {
        let mut w = world(50_000.0, true);
        run(&mut w, 2_000);
        let kinds: std::collections::HashSet<_> = w.g.news.iter().map(|n| n.kind).collect();
        assert!(kinds.contains(&NewsKind::Market), "no market news in 2000 ticks");
        assert!(w.i.iter().all(|s| s.price.is_finite() && s.price > 0.0));
        assert!(w.p.cash.is_finite());
    }

    #[test]
    fn interrupt_stops_a_skip_when_a_prompt_appears() {
        let mut w = world(50_000.0, true);
        let mut stopped = false;
        for _ in 0..40 {
            let out = fast_forward_with_rng(&mut w.p, &mut w.i, &mut w.pf, &mut w.m, &mut w.g, 500, true, &mut w.rng);
            if out.interrupted {
                assert!(w.g.has_prompt());
                assert!(out.ticks_run < 500);
                stopped = true;
                break;
            }
            if w.g.status.is_lost() {
                break;
            }
        }
        assert!(stopped);
    }

    #[test]
    fn broker_tips_require_a_good_relationship() {
        let tips = |rel: f64| {
            let mut w = world(50_000.0, true);
            w.g.npcs[NPC_BROKER].relationship = rel;
            run(&mut w, 4_000);
            w.g.news.iter().filter(|n| n.text.contains("(broker)")).count()
        };
        assert_eq!(tips(30.0), 0);
        assert!(tips(95.0) > 0);
    }

    #[test]
    fn game_state_survives_a_json_roundtrip() {
        let mut w = world(50_000.0, true);
        run(&mut w, 300);
        w.g.start_buy_business(0, &mut w.rng).ok();
        let json = serde_json::to_string(&w.g).unwrap();
        let back: GameState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.news.len(), w.g.news.len());
        assert_eq!(back.status, w.g.status);
        assert_eq!(back.has_prompt(), w.g.has_prompt());
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    #[test]
    fn loans_charge_interest_and_amortize() {
        let mut w = world(1_000.0, false);
        assert!(w.g.borrow(&mut w.p, 100_000.0).is_err(), "over the limit");
        assert!(w.g.borrow(&mut w.p, -5.0).is_err());
        w.g.borrow(&mut w.p, 10_000.0).unwrap();
        assert_eq!(w.p.cash, 11_000.0);
        assert_eq!(w.g.debt(), 10_000.0);
        let rate = w.g.loan.as_ref().unwrap().annual_rate;
        assert!((rate - (LONG_RUN_INTEREST_RATE + LOAN_SPREAD)).abs() < 1e-9);
        // Net worth counts the debt.
        let prices = HashMap::new();
        assert_eq!(w.g.assets_value(&prices), -10_000.0);
        run(&mut w, 30);
        let interest = 10_000.0 * w.g.loan.as_ref().unwrap().annual_rate / 12.0;
        assert!(w.g.debt() < 10_000.0 && (w.g.debt() - 9_800.0).abs() < 1e-6);
        assert!((w.g.stats.interest_paid - interest).abs() < 1e-6);
        // Salary - expenses - interest - minimum repayment.
        let expected = 11_000.0 + 800.0 - monthly_withholding(4_000.0, 1.0) - interest - 200.0;
        assert!((w.p.cash - expected).abs() < 1e-6, "{}", w.p.cash);
    }

    #[test]
    fn repaying_a_loan_early() {
        let mut w = world(1_000.0, false);
        w.g.borrow(&mut w.p, 5_000.0).unwrap();
        assert!(w.g.repay(&mut w.p, 1_000.0, 100.0).is_err(), "not enough free cash");
        let free = w.p.cash;
        w.g.repay(&mut w.p, 2_000.0, free).unwrap();
        assert_eq!(w.g.debt(), 3_000.0);
        let free = w.p.cash;
        w.g.repay(&mut w.p, 99_999.0, free).unwrap();
        assert!(w.g.loan.is_none());
        assert!(w.g.repay(&mut w.p, 1.0, 100.0).is_err());
    }

    #[test]
    fn the_unemployed_cannot_borrow_and_debt_limits_add_up() {
        let mut w = world(0.0, false);
        w.g.career.employed = false;
        assert!(w.g.borrow(&mut w.p, 100.0).is_err());
        w.g.career.employed = true;
        w.g.borrow(&mut w.p, 20_000.0).unwrap();
        assert!((w.g.borrowing_room(&w.p) - 4_000.0).abs() < 1e-9);
        assert!(w.g.borrow(&mut w.p, 4_001.0).is_err());
        w.g.borrow(&mut w.p, 4_000.0).unwrap();
    }

    #[test]
    fn too_much_debt_can_bankrupt_you() {
        let mut w = world(0.0, false);
        w.g.borrow(&mut w.p, 24_000.0).unwrap();
        w.p.cash = 0.0; // spent it all
        w.p.expenses = 3_900.0; // barely above salary before interest
        run(&mut w, 30 * 6);
        assert!(w.g.status.is_lost(), "{:?}", w.g.status);
    }

    #[test]
    fn stats_track_the_run() {
        let mut w = world(60_000.0, false);
        w.g.decision = Some(PendingDecision { key: "rent_increase".into(), deadline_tick: 99 });
        w.g.choose_decision(&mut w.p, 0, &mut w.rng).unwrap();
        run(&mut w, 60);
        assert_eq!(w.g.stats.decisions_made, 1);
        assert_eq!(w.g.stats.salary_income, 8_000.0);
        assert!(w.g.stats.peak_net_worth >= 60_000.0);
        assert!(w.g.stats.expenses_paid > 6_000.0);
    }

    #[test]
    fn saves_from_before_loans_and_stats_still_load() {
        let w = world(1_000.0, false);
        let mut v: serde_json::Value = serde_json::to_value(&w.g).unwrap();
        v.as_object_mut().unwrap().remove("loan");
        v.as_object_mut().unwrap().remove("stats");
        let back: GameState = serde_json::from_value(v).unwrap();
        assert!(back.loan.is_none());
        assert_eq!(back.stats, Stats::default());
    }

    #[test]
    fn dividends_pay_on_payday_and_funds_reflect_them() {
        let mut w = world(0.0, false);
        w.p.salary = 0.0;
        w.p.expenses = 0.0;
        w.i[1].instrument.income_yield = 0.06; // IDX pays 6% a year
        w.pf.buy(2, 100.0, 100.0);
        run(&mut w, 30);
        let expected = 100.0 * 100.0 * 0.06 * (30.0 / 365.0) * (1.0 - INVESTMENT_INCOME_RATE);
        assert!((w.p.cash - expected).abs() < 0.01, "{}", w.p.cash);
    }

    #[test]
    fn savings_earn_interest_taxed_and_back_up_the_bills() {
        let mut w = world(10_000.0, false);
        assert!(w.g.deposit(&mut w.p, 20_000.0, 10_000.0).is_err());
        w.g.deposit(&mut w.p, 6_000.0, 10_000.0).unwrap();
        assert_eq!((w.p.cash, w.g.savings), (4_000.0, 6_000.0));
        let prices = HashMap::new();
        assert_eq!(w.g.assets_value(&prices), 6_000.0, "savings count toward net worth");
        run(&mut w, 30);
        let earned = w.g.stats.investment_income;
        assert!(earned > 0.0 && earned < 6_000.0 * 0.06 / 12.0 + 1.0, "{earned}");
        // Bills bigger than cash fall back on the savings account first.
        w.p.cash = 0.0;
        w.p.expenses = 6_000.0;
        run(&mut w, 30);
        assert!(w.p.cash >= 0.0);
        assert!(w.g.savings < 6_000.0);
        assert_eq!(w.g.distress_months, 0);
        assert_eq!(w.pf.positions.len(), 0, "no forced sale was needed");
        assert!(w.g.withdraw(&mut w.p, 100.0).is_ok());
    }

    #[test]
    fn capital_gains_are_taxed_and_losses_carry_forward() {
        let mut w = world(0.0, false);
        w.p.salary = 0.0;
        w.p.expenses = 0.0;
        w.pf.buy(2, 100.0, 50.0); // cost basis 50
        // Sell 100 at a 100 bid: about 5,000 gain.
        w.m.market_order(&mut w.p, &mut w.pf, 2, false, 100.0).unwrap();
        assert!(w.pf.realized_gains > 4_800.0);
        let cash = w.p.cash;
        run(&mut w, 30);
        let taxed = cash - w.p.cash;
        assert!(taxed > 700.0 && taxed < 760.0, "15% of ~4.9k gain, got {taxed}");
        assert_eq!(w.pf.realized_gains, 0.0);
        // A later loss is carried forward and offsets the next gain.
        w.pf.buy(2, 10.0, 200.0);
        w.m.market_order(&mut w.p, &mut w.pf, 2, false, 10.0).unwrap();
        run(&mut w, 30);
        assert!(w.g.loss_carry < -500.0, "{}", w.g.loss_carry);
    }

    #[test]
    fn monthly_economy_runs_with_the_game_and_feeds_inflation_into_expenses() {
        let mut w = world(100_000.0, false);
        w.g.economy.inflation = 0.06;
        let before = w.p.expenses;
        run(&mut w, 30);
        assert_eq!(w.g.economy.history.len(), 1);
        assert_eq!(w.g.new_points.len(), 1);
        assert!(w.p.expenses > before, "inflation raises your cost of living");
        run(&mut w, 330);
        assert_eq!(w.g.economy.history.len(), 12);
        assert!(w.g.economy.cpi > 100.0);
        assert!(w.g.news.iter().any(|n| n.text.contains("Year in review")));
    }

    #[test]
    fn every_cash_movement_is_in_the_ledger() {
        let mut w = world(5_000.0, false);
        let start = w.p.cash;
        w.g.borrow(&mut w.p, 1_000.0).unwrap();
        run(&mut w, 30);
        // Net of everything that touched cash, the ledger explains the change.
        let ledger_total: f64 = w.g.ledger.iter().map(|e| e.amount).sum();
        let actual = w.p.cash - start;
        assert!((ledger_total - actual).abs() < 1e-6, "ledger {ledger_total} vs cash change {actual}");
        let cats: std::collections::HashSet<&str> = w.g.ledger.iter().map(|e| e.category.as_str()).collect();
        for c in ["salary", "tax", "living", "interest", "loan_principal", "loan_draw"] {
            assert!(cats.contains(c), "missing category {c}: {cats:?}");
        }
    }

    #[test]
    fn tax_policy_events_move_the_government_stance() {
        let mut w = world(0.0, false);
        w.g.apply_effect(Effect::TaxMult(0.08), &mut w.p, &mut w.rng);
        assert!((w.g.economy.govt.tax_mult - 1.08).abs() < 1e-9);
        w.g.apply_effect(Effect::TaxMult(-5.0), &mut w.p, &mut w.rng);
        assert_eq!(w.g.economy.govt.tax_mult, 0.7);
        let expected = monthly_withholding(4_000.0, 0.7);
        w.p.cash = 0.0;
        run(&mut w, 30);
        let net = 4_000.0 - expected - 3_200.0;
        assert!((w.p.cash - net).abs() < 1e-6, "{} vs {net}", w.p.cash);
    }

    #[test]
    fn old_saves_without_the_economy_still_load() {
        let w = world(1_000.0, false);
        let mut v: serde_json::Value = serde_json::to_value(&w.g).unwrap();
        for k in ["economy", "savings", "pending_gains", "loss_carry"] {
            v.as_object_mut().unwrap().remove(k);
        }
        let back: GameState = serde_json::from_value(v).unwrap();
        assert_eq!(back.economy.cpi, 100.0);
        assert_eq!(back.savings, 0.0);
    }

    #[test]
    fn being_away_pauses_your_personal_finances() {
        let mut w = world(10_000.0, true); // events ON
        w.g.away = true;
        let expenses = w.p.expenses;
        w.g.apply_effect(Effect::TempExpense { amount: 100.0, ticks: 40, label: "plan", }, &mut w.p, &mut w.rng);
        let rivals: Vec<f64> = w.g.rivals.iter().map(|r| r.monthly_saving).collect();
        let out = run(&mut w, 600);
        assert_eq!(out.ticks_run, 600);
        // No salary, bills, taxes on pay, reviews, personal events or decisions.
        assert_eq!(w.p.cash, 10_000.0, "cash must not move: {}", w.p.cash);
        assert_eq!(w.p.expenses, expenses);
        assert!(w.g.decision.is_none() && w.g.negotiation.is_none());
        assert_eq!(w.g.stats.salary_income, 0.0);
        assert_eq!(w.g.distress_months, 0);
        assert!(!w.g.news.iter().any(|n| n.kind == NewsKind::Personal));
        // The payment plan hasn't been used up by time away.
        assert_eq!(w.g.temp_expenses.len(), 1);
        // Rivals didn't save either.
        let after: Vec<f64> = w.g.rivals.iter().map(|r| r.monthly_saving).collect();
        assert_eq!(rivals, after);
        // The world kept moving: the economy updated and market events happened.
        assert_eq!(w.g.economy.history.len(), 20);
        assert!(w.g.news.iter().any(|n| n.kind == NewsKind::Market), "market events still happen");
    }

    #[test]
    fn being_away_does_not_stop_investments_earning() {
        let mut w = world(0.0, false);
        w.g.away = true;
        w.i[1].instrument.income_yield = 0.06;
        w.pf.buy(2, 100.0, 100.0);
        run(&mut w, 30);
        assert!(w.p.cash > 40.0, "dividends still arrive while away: {}", w.p.cash);
        // Resting orders can still fill while away.
        let mut w = world(10_000.0, false);
        w.g.away = true;
        w.m.limit_order(&mut w.p, &mut w.pf, 1, true, 10.0, 130.0).unwrap(); // above market: fills at once
        assert!(w.pf.quantity_of(1) > 0.0);
    }

    #[test]
    fn prompts_wait_for_you_while_away_and_resume_after() {
        let mut w = world(10_000.0, false);
        w.g.decision = Some(PendingDecision { key: "rent_increase".into(), deadline_tick: 5 });
        w.g.away = true;
        run(&mut w, 60);
        assert!(w.g.decision.is_some(), "an unanswered decision must not expire while you're away");
        w.g.away = false;
        run(&mut w, 1);
        assert!(w.g.decision.is_none(), "once you're back its deadline applies");
    }

    #[test]
    fn coming_back_resumes_normal_pay() {
        let mut w = world(1_000.0, false);
        w.g.away = true;
        run(&mut w, 60);
        assert_eq!(w.p.cash, 1_000.0);
        w.g.away = false;
        run(&mut w, 30);
        let expected = 1_000.0 + 800.0 - monthly_withholding(4_000.0, 1.0);
        assert!((w.p.cash - expected).abs() < 1e-6, "{}", w.p.cash);
    }

    fn studio(price: f64) -> HomeListing {
        HomeListing { id: 900, kind: 0, price, expires_tick: 10_000 }
    }

    fn buy_studio(w: &mut World, price: f64) {
        w.g.home_listings = vec![studio(price)];
        let free = w.p.cash;
        w.g.start_buy_home(&w.p, free, 0, &mut w.rng).unwrap();
        let step = w.g.negotiate_accept(&mut w.p, &w.i, free, &mut w.rng).unwrap();
        assert!(matches!(step, NegStep::Done(_)), "{step:?}");
    }

    #[test]
    fn buying_a_home_swaps_rent_for_a_mortgage() {
        let mut w = world(100_000.0, false);
        let rent = w.g.rent;
        assert!((rent - 3_200.0 * RENT_SHARE).abs() < 1e-9);
        buy_studio(&mut w, 110_000.0);
        let h = w.g.home.as_ref().expect("bought");
        let price = h.price_paid;
        assert!(price > 100_000.0 && price < 140_000.0, "{price}");
        assert!((100_000.0 - w.p.cash - price * (DOWN_PAYMENT + CLOSING_COST)).abs() < 1e-6);
        assert!((w.p.expenses - (3_200.0 - rent)).abs() < 1e-9, "rent is gone from expenses");
        assert!((h.mortgage.principal - price * 0.8).abs() < 1e-6);
        assert_eq!(w.g.stats.home_purchases, 1);
        // Net worth: the home counts as equity (price - mortgage = the 20% down payment).
        let prices = HashMap::new();
        assert!((w.g.assets_value(&prices) - price * 0.2).abs() < 1e-6);

        // One payday: salary - tax - (non-rent) expenses - mortgage payment - upkeep.
        let before = w.p.cash;
        let expenses = w.p.expenses;
        let pay = w.g.home.as_ref().unwrap().mortgage.payment;
        run(&mut w, 30);
        let upkeep = w.g.home.as_ref().unwrap().monthly_upkeep(w.g.economy.home_price_index);
        let expected = before + 4_000.0 - monthly_withholding(4_000.0, 1.0) - expenses - pay - upkeep;
        assert!((w.p.cash - expected).abs() < 1e-6, "{} vs {expected}", w.p.cash);
        assert!(w.g.home.as_ref().unwrap().mortgage.principal < price * 0.8, "principal is paid down");
        assert!(w.g.ledger.iter().any(|e| e.category == "housing"));
        assert!(w.g.ledger.iter().any(|e| e.category == "mortgage_principal"));
        assert!(w.g.ledger.iter().any(|e| e.category == "home"));
    }

    #[test]
    fn you_need_the_cash_and_the_income() {
        // Not enough cash for 23% up front: the agent won't even sit down.
        let mut w = world(10_000.0, false);
        w.g.home_listings = vec![studio(110_000.0)];
        let err = w.g.start_buy_home(&w.p, 10_000.0, 0, &mut w.rng).unwrap_err();
        assert!(err.contains("in cash"), "{err}");
        assert!(w.g.negotiation.is_none() && w.g.home.is_none());

        // Plenty of cash but the salary can't carry the payment.
        let mut w = world(500_000.0, false);
        w.p.salary = 1_000.0;
        w.g.home_listings = vec![studio(400_000.0)];
        let err = w.g.start_buy_home(&w.p, 500_000.0, 0, &mut w.rng).unwrap_err();
        assert!(err.contains("won't lend"), "{err}");

        // Cash that drains during haggling is still caught when you accept.
        let mut w = world(100_000.0, false);
        w.g.home_listings = vec![studio(110_000.0)];
        w.g.start_buy_home(&w.p, 100_000.0, 0, &mut w.rng).unwrap();
        let err = w.g.negotiate_accept(&mut w.p, &w.i, 5_000.0, &mut w.rng).unwrap_err();
        assert!(err.contains("in cash"), "{err}");
        assert!(w.g.negotiation.is_some(), "the negotiation stays open");

        // No mortgage when unemployed, and only one home at a time.
        let mut w = world(100_000.0, false);
        w.g.home_listings = vec![studio(110_000.0)];
        w.g.career.employed = false;
        assert!(w.g.start_buy_home(&w.p, 100_000.0, 0, &mut w.rng).is_err());
        w.g.career.employed = true;
        buy_studio(&mut w, 110_000.0);
        w.g.home_listings = vec![studio(110_000.0)];
        assert!(w.g.start_buy_home(&w.p, 100_000.0, 0, &mut w.rng).is_err());
    }

    #[test]
    fn selling_a_home_clears_the_mortgage_and_restores_rent() {
        let mut w = world(100_000.0, false);
        let rent = w.g.rent;
        buy_studio(&mut w, 110_000.0);
        let price = w.g.home.as_ref().unwrap().price_paid;
        w.g.economy.home_price_index = 120.0; // prices rose 20%
        let cash = w.p.cash;
        let expenses = w.p.expenses;
        w.g.sell_home(&mut w.p).unwrap();
        let expected = price * 1.2 * (1.0 - SELLING_COST) - price * 0.8;
        assert!((w.p.cash - cash - expected).abs() < 1e-6);
        assert!((w.p.expenses - expenses - rent).abs() < 1e-9);
        assert!(w.g.home.is_none());
        assert!(w.g.sell_home(&mut w.p).is_err());
    }

    #[test]
    fn underwater_homes_cannot_be_sold_without_cash() {
        let mut w = world(100_000.0, false);
        buy_studio(&mut w, 110_000.0);
        w.g.economy.home_price_index = 50.0;
        w.p.cash = 100.0;
        let err = w.g.sell_home(&mut w.p).unwrap_err();
        assert!(err.contains("mortgage"), "{err}");
        assert!(w.g.home.is_some());
        assert!(w.g.home_equity() < 0.0);
    }

    #[test]
    fn the_house_is_sold_when_nothing_else_covers_the_bills() {
        let mut w = world(100_000.0, false);
        buy_studio(&mut w, 110_000.0);
        w.g.economy.home_price_index = 140.0; // plenty of equity
        w.p.cash = 0.0;
        w.p.expenses = 9_000.0;
        let to_payday = 30 - w.p.current_tick % 30;
        run(&mut w, to_payday);
        assert!(w.g.home.is_none(), "forced sale");
        assert!(w.p.cash >= 0.0);
        assert!(w.g.news.iter().any(|n| n.text.contains("forced sale")));
    }

    #[test]
    fn home_equity_does_not_count_toward_independence() {
        let mut w = world(0.0, false);
        w.g.home = Some(Home {
            name: "Mansion".into(),
            bought_tick: 0,
            price_paid: 2_000_000.0,
            hpi_at_purchase: 100.0,
            mortgage: Mortgage { principal: 0.0, annual_rate: 0.05, payment: 0.0 },
        });
        let prices = HashMap::new();
        let nw = w.p.cash + w.g.assets_value(&prices);
        assert!(nw > 1_900_000.0);
        assert!(w.g.fi_progress(&w.p, nw) < 0.05, "a paid-off mansion isn't an income");
        run(&mut w, 5);
        assert!(!w.g.status.is_won());
    }

    #[test]
    fn renters_get_landlord_decisions_and_owners_get_roof_leaks() {
        let seen = |own: bool| {
            let mut w = world(200_000.0, true);
            w.p.salary = 20_000.0;
            if own {
                buy_studio(&mut w, 110_000.0);
            }
            let mut keys: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut done = 0;
            while done < 6_000 && !w.g.status.is_lost() {
                let out = fast_forward_with_rng(&mut w.p, &mut w.i, &mut w.pf, &mut w.m, &mut w.g, 6_000 - done, true, &mut w.rng);
                done += out.ticks_run;
                if let Some(d) = w.g.decision.as_ref() {
                    keys.insert(d.key.clone());
                    let def = decision(&d.key).unwrap().default_option;
                    w.g.choose_decision(&mut w.p, def, &mut w.rng).unwrap();
                }
                if w.g.negotiation.is_some() {
                    w.g.negotiate_walk(&w.p); // annual reviews also pause a skip
                }
                if out.ticks_run == 0 {
                    break;
                }
            }
            keys
        };
        let renter = seen(false);
        assert!(renter.contains("rent_increase"), "{renter:?}");
        assert!(!renter.contains("roof_leak"), "renters have no roof");
        let owner = seen(true);
        assert!(owner.contains("roof_leak"), "{owner:?}");
        assert!(!owner.contains("rent_increase"), "owners have no landlord");
    }

    #[test]
    fn listings_follow_the_home_price_index() {
        let mut cheap = world(0.0, false);
        cheap.g.economy.home_price_index = 60.0;
        cheap.g.home_listings.clear();
        cheap.g.next_listing_tick = 0;
        run(&mut cheap, 1);
        let mut dear = world(0.0, false);
        dear.g.economy.home_price_index = 160.0;
        dear.g.home_listings.clear();
        dear.g.next_listing_tick = 0;
        run(&mut dear, 1);
        let avg = |g: &GameState| g.home_listings.iter().map(|l| l.price / HOME_KINDS[l.kind].1).sum::<f64>() / g.home_listings.len() as f64;
        assert!(avg(&dear.g) > avg(&cheap.g) * 2.0);
        assert_eq!(cheap.g.home_listings.len(), 3);
    }

    #[test]
    fn old_saves_gain_a_realtor_and_a_rent_figure() {
        let w = world(1_000.0, false);
        let mut v: serde_json::Value = serde_json::to_value(&w.g).unwrap();
        for k in ["rent", "home", "home_listings", "assets"] {
            v.as_object_mut().unwrap().remove(k);
        }
        v["npcs"].as_array_mut().unwrap().pop();
        let mut back: GameState = serde_json::from_value(v).unwrap();
        assert_eq!(back.npcs.len(), 4);
        assert_eq!(back.rent, 0.0);
        let mut w2 = world(1_000.0, false);
        w2.g = back.clone();
        run(&mut w2, 1);
        assert_eq!(w2.g.npcs.len(), 5);
        assert!((w2.g.rent - 3_200.0 * RENT_SHARE).abs() < 5.0);
        back.ensure_compat(&w2.p);
        assert_eq!(back.npcs[NPC_REALTOR].name, "Helen Okafor");
    }

    #[test]
    fn savings_compound_and_report_their_interest() {
        let mut w = world(0.0, false);
        w.p.salary = 0.0;
        w.p.expenses = 0.0;
        w.g.savings = 10_000.0;
        // Net monthly rate at a 4% policy rate: 3.5%/12 less 15% tax.
        let rate = (w.p.interest_rate - SAVINGS_SPREAD) / 12.0 * (1.0 - INVESTMENT_INCOME_RATE);
        let projected = GameState::project_savings(10_000.0, w.p.interest_rate, 10.0);
        let mut b = 10_000.0;
        for _ in 0..120 {
            b *= 1.0 + rate;
        }
        assert!((projected - b).abs() < 1e-6, "{projected} vs {b}");
        assert!(projected > 13_000.0 && projected < 14_000.0, "{projected}");
        assert!((w.g.next_savings_interest(w.p.interest_rate) - 10_000.0 * rate).abs() < 1e-9);
        // The account actually pays it, and the total is tracked after tax.
        let before = w.g.savings;
        run(&mut w, 30);
        assert!(w.g.stats.savings_interest > 20.0 && w.g.stats.savings_interest < 40.0, "{}", w.g.stats.savings_interest);
        assert!((w.p.cash - w.g.stats.savings_interest).abs() < 1e-6, "interest is paid out to your account");
        assert_eq!(w.g.savings, before, "the balance itself stays put");
        // Old saves that held the retired SAVE instrument keep their money.
        assert_eq!(w.g.absorb_retired_savings(0.2, 5), 0.0);
        assert_eq!(w.g.absorb_retired_savings(1_500.0, 5), 1_500.0);
        assert_eq!(w.g.savings, before + 1_500.0);
        assert!(w.g.news.iter().any(|n| n.text.contains("High-Yield Savings")));
    }

    // ------------------------------------------------------------ assets

    #[test]
    fn a_financed_flat_counts_its_equity_and_the_loan_is_a_transfer() {
        let mut w = world(100_000.0, false);
        let free = w.p.cash;
        let price = asset_price(0, w.g.economy.home_price_index, w.g.economy.cpi);
        let (need, loan) = purchase_terms(0, price, true);
        w.g.buy_asset(&mut w.p, free, 0, true).unwrap();
        assert_eq!(w.g.assets.len(), 1);
        assert!((w.p.cash - (100_000.0 - need)).abs() < 1e-6);
        assert!((w.g.assets[0].debt() - loan).abs() < 1e-6);
        let prices = HashMap::new();
        assert!((w.g.assets_value(&prices) - (price - loan)).abs() < 1e-6, "equity is value minus mortgage");
        assert!(w.g.ledger.iter().any(|e| e.category == "asset" && e.amount < 0.0));
        // Not enough cash for a second one.
        let err = w.g.buy_asset(&mut w.p, 1_000.0, 0, true).unwrap_err();
        assert!(err.contains("free cash"), "{err}");
    }

    #[test]
    fn rental_profit_is_taxed_and_a_mortgage_is_paid_down() {
        let mut w = world(400_000.0, false);
        let free = w.p.cash;
        w.g.buy_asset(&mut w.p, free, 1, false).unwrap(); // storage units, cash
        for _ in 0..12 {
            w.g.assets_payday(&mut w.p, &mut w.rng);
        }
        let s = &w.g.stats;
        assert!(s.rental_income > 0.0 && s.rental_income <= 45_000.0 * 0.085 + 1e-6, "{}", s.rental_income);
        assert!(s.taxes_paid > 0.0 && s.taxes_paid <= s.rental_income * INVESTMENT_INCOME_RATE + 1e-6, "{}", s.taxes_paid);
        assert!(w.g.ledger.iter().any(|e| e.category == "rent_income"));
        assert!(w.g.ledger.iter().any(|e| e.description == "Tax on rental profit"));

        let mut w = world(200_000.0, false);
        let free = w.p.cash;
        w.g.buy_asset(&mut w.p, free, 0, true).unwrap();
        let debt0 = w.g.assets[0].debt();
        for _ in 0..12 {
            w.g.assets_payday(&mut w.p, &mut w.rng);
        }
        assert!(w.g.assets[0].debt() < debt0, "principal is repaid every month");
        assert!(w.g.ledger.iter().any(|e| e.category == "mortgage_principal"));
    }

    #[test]
    fn selling_investment_property_feeds_capital_gains_but_a_car_does_not() {
        let mut w = world(400_000.0, false);
        let free = w.p.cash;
        w.g.buy_asset(&mut w.p, free, 0, false).unwrap();
        let basis = w.g.assets[0].cost_basis();
        w.g.assets[0].value = 180_000.0;
        let cash = w.p.cash;
        w.g.sell_asset(&mut w.p, 0).unwrap();
        let net = 180_000.0 * (1.0 - 0.06);
        assert!((w.p.cash - (cash + net)).abs() < 1e-6);
        assert!((w.g.pending_gains - (net - basis)).abs() < 1e-6, "{}", w.g.pending_gains);
        assert!(w.g.assets.is_empty());

        let before = w.g.pending_gains;
        let free = w.p.cash;
        w.g.buy_asset(&mut w.p, free, 3, false).unwrap();
        w.g.sell_asset(&mut w.p, 0).unwrap();
        assert_eq!(w.g.pending_gains, before, "a loss on a personal car is not deductible");
        assert!(w.g.sell_asset(&mut w.p, 0).is_err());
    }

    #[test]
    fn solar_saves_money_and_a_car_costs_it_and_neither_counts_toward_fi() {
        let mut w = world(100_000.0, false);
        let base = w.g.total_costs(&w.p);
        let free = w.p.cash;
        w.g.buy_asset(&mut w.p, free, 2, false).unwrap();
        assert!((w.g.total_costs(&w.p) - base).abs() < 1e-6, "solar's saving outweighs its upkeep");
        let cash = w.p.cash;
        w.g.assets_payday(&mut w.p, &mut w.rng);
        assert!(w.p.cash > cash, "bill savings are paid every month");
        let free = w.p.cash;
        w.g.buy_asset(&mut w.p, free, 3, false).unwrap();
        assert!(w.g.total_costs(&w.p) > base + 50.0, "the car's running costs count as living costs");
        let worth: f64 = w.g.assets.iter().map(|a| a.value).sum();
        assert!((w.g.non_fi_wealth() - worth).abs() < 1e-6, "lifestyle assets don't count toward FI");
        assert!((w.g.assets_equity() - worth).abs() < 1e-6, "but they are part of net worth");
    }

    #[test]
    fn assets_are_sold_before_the_home_when_bills_cant_be_paid() {
        let mut w = world(100_000.0, false);
        let free = w.p.cash;
        w.g.buy_asset(&mut w.p, free, 3, false).unwrap();
        w.p.cash = -1_000.0;
        let sold = w.g.forced_asset_sales(&mut w.p);
        assert_eq!(sold.len(), 1);
        assert!(w.g.assets.is_empty());
        assert!(w.p.cash > 10_000.0, "{}", w.p.cash);
    }

    #[test]
    fn owning_assets_is_paused_while_you_are_away() {
        let mut w = world(400_000.0, false);
        let free = w.p.cash;
        w.g.buy_asset(&mut w.p, free, 1, false).unwrap();
        let cash = w.p.cash;
        w.g.away = true;
        run(&mut w, 90);
        assert_eq!(w.g.stats.rental_income, 0.0);
        assert_eq!(w.p.cash, cash, "no rent, upkeep or tax while away");
    }

    #[test]
    fn asset_purchase_limits() {
        let mut w = world(10_000_000.0, false);
        for _ in 0..MAX_ASSETS {
            let free = w.p.cash;
            w.g.buy_asset(&mut w.p, free, 3, false).unwrap();
        }
        let free = w.p.cash;
        assert!(w.g.buy_asset(&mut w.p, free, 3, false).unwrap_err().contains("more than"));
        let mut w = world(1_000_000.0, false);
        w.g.career.employed = false;
        let free = w.p.cash;
        assert!(w.g.buy_asset(&mut w.p, free, 0, true).is_err(), "no mortgage while unemployed");
        assert!(w.g.buy_asset(&mut w.p, free, 0, false).is_ok(), "but you can pay cash");
    }
}
