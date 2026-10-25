use rand::Rng;
use rand_distr::{Distribution, Normal};

use std::collections::HashMap;

use crate::game::GameState;
use crate::instrument::{AssetClass, InstrumentState};
use crate::market::{Fill, LimitOrder, Market};
use crate::player::{Player, PAY_PERIOD_TICKS};
use crate::portfolio::Portfolio;

/// Length of one simulation tick, expressed as a fraction of a year.
/// 1 tick = 1 simulated day. Change this in one place to retune pacing.
pub const TICK_DT_YEARS: f64 = 1.0 / 365.0;

/// Long-run average prevailing interest rate the economy reverts
/// toward (annualized, e.g. 0.04 = 4%). New saves also start here.
pub const LONG_RUN_INTEREST_RATE: f64 = 0.04;
/// How strongly the prevailing rate pulls back toward the long-run
/// average each tick - a standard mean-reversion (Ornstein-Uhlenbeck
/// style) speed constant.
const RATE_REVERSION_SPEED: f64 = 0.15;
/// Annualized volatility of the prevailing rate's own random walk.
const RATE_VOLATILITY: f64 = 0.01;
pub(crate) const MIN_INTEREST_RATE: f64 = 0.005;
pub(crate) const MAX_INTEREST_RATE: f64 = 0.15;

#[derive(Debug, Clone)]
pub struct PricePoint {
    pub instrument_id: i32,
    pub tick: i64,
    pub price: f64,
}

#[derive(Debug, Clone)]
pub struct NetWorthPoint {
    pub tick: i64,
    pub net_worth: f64,
}

/// One Geometric Brownian Motion step. This is the stochastic
/// approximation of each instrument's fair price - a realistic random
/// walk shaped by its drift/volatility. Trades don't move it directly;
/// they execute against NPC quotes around it (see the `market`
/// module). Used for equity, index, savings, commodity, real estate
/// and crypto. Bonds are priced off the prevailing interest rate
/// (`bond_price`), forex mean-reverts (`step_mean_reverting`), and
/// mutual funds are derived from their basket (`step_fund`).
fn step_price(price: f64, drift: f64, volatility: f64, dt: f64, rng: &mut impl Rng) -> f64 {
    let normal = Normal::new(0.0, 1.0).expect("valid normal distribution");
    let z: f64 = normal.sample(rng);
    let exponent = (drift - 0.5 * volatility * volatility) * dt + volatility * dt.sqrt() * z;
    (price * exponent.exp()).max(0.01)
}

/// Advances the economy's prevailing interest rate by one tick: pulls
/// toward the long-run average, plus a small random shock. This is
/// what every bond's price reacts to.
fn step_interest_rate(rate: f64, dt: f64, rng: &mut impl Rng) -> f64 {
    let normal = Normal::new(0.0, 1.0).expect("valid normal distribution");
    let z: f64 = normal.sample(rng);
    let drift = RATE_REVERSION_SPEED * (LONG_RUN_INTEREST_RATE - rate) * dt;
    let shock = RATE_VOLATILITY * dt.sqrt() * z;
    (rate + drift + shock).clamp(MIN_INTEREST_RATE, MAX_INTEREST_RATE)
}

/// Simplified duration-based bond pricing: %ΔPrice ≈ -Duration ×
/// (prevailing_rate - coupon_rate). This is the standard first-order
/// price-sensitivity approximation, not a full discounted-cash-flow
/// model - deliberately: it's enough to teach the core lesson (bond
/// prices fall when prevailing rates rise above the bond's own
/// coupon, and vice versa) without needing amortization or maturity
/// bookkeeping. Clamped so a large rate gap can't send price to zero
/// or beyond a sane multiple of face value.
pub fn bond_price(face_value: f64, coupon_rate: f64, duration: f64, prevailing_rate: f64) -> f64 {
    let ratio = (1.0 - duration * (prevailing_rate - coupon_rate)).clamp(0.5, 1.5);
    face_value * ratio
}

/// Annualized speed at which a forex pair is pulled back toward its
/// long-run level (half-life of a few months).
const FOREX_REVERSION_SPEED: f64 = 2.0;

/// Mean-reverting step used for Forex: log-price is pulled toward the
/// log of the pair's long-run level (an Ornstein-Uhlenbeck process on
/// log price) plus a random shock. Currencies wander but don't drift
/// off to zero or infinity the way a stock can.
fn step_mean_reverting(price: f64, long_run: f64, speed: f64, volatility: f64, dt: f64, rng: &mut impl Rng) -> f64 {
    let normal = Normal::new(0.0, 1.0).expect("valid normal distribution");
    let z: f64 = normal.sample(rng);
    let log_p = price.max(0.0001).ln();
    let pull = speed * (long_run.max(0.0001).ln() - log_p) * dt;
    let shock = volatility * dt.sqrt() * z;
    (log_p + pull + shock).exp().max(0.0001)
}

/// Fraction of a unit's price paid out as income over one tick (bond
/// coupons accrue on face value; property trusts pay a % of price).
/// Mutual funds fold this into their NAV return, so owning a fund
/// holding bonds or property is a total-return experience.
fn income_return_per_tick(instrument: &crate::instrument::Instrument, prev_price: f64, dt: f64) -> f64 {
    match instrument.class {
        AssetClass::Bond => {
            if prev_price > 0.0 {
                instrument.face_value * instrument.coupon_rate * dt / prev_price
            } else {
                0.0
            }
        }
        _ => instrument.income_yield * dt,
    }
}

/// One tick for a mutual fund: its NAV moves by the weighted average
/// total return of its constituents (price change plus income, as if
/// reinvested), minus the expense ratio's slice for this tick. Weights
/// are effectively rebalanced every tick, like an index tracker.
fn step_fund(
    fund: &InstrumentState,
    prev: &std::collections::HashMap<i32, f64>,
    now: &std::collections::HashMap<i32, f64>,
    income: &std::collections::HashMap<i32, f64>,
    dt: f64,
) -> f64 {
    let mut basket_return = 0.0;
    for c in &fund.instrument.constituents {
        if let (Some(p0), Some(p1)) = (prev.get(&c.instrument_id), now.get(&c.instrument_id)) {
            if *p0 > 0.0 {
                let income_r = income.get(&c.instrument_id).copied().unwrap_or(0.0);
                basket_return += c.weight * ((p1 / p0 - 1.0) + income_r);
            }
        }
    }
    let drag = fund.instrument.expense_ratio * dt;
    (fund.price * (1.0 + basket_return - drag)).max(0.01)
}

/// Everything one `fast_forward` batch produced, ready to persist.
#[derive(Debug, Clone, Default)]
pub struct TickOutput {
    pub price_points: Vec<PricePoint>,
    pub net_worth_points: Vec<NetWorthPoint>,
    /// Executions during the batch: resting limit orders that filled,
    /// and holdings force-sold to cover bills.
    pub fills: Vec<Fill>,
    /// Resting limit orders dropped because they became unfundable.
    pub cancelled_orders: Vec<LimitOrder>,
    /// Ticks actually simulated (fewer than requested if the batch was
    /// interrupted by a prompt or the game ended).
    pub ticks_run: i64,
    /// True if the batch stopped early because something needs an answer.
    pub interrupted: bool,
}

/// Advance the whole simulation by `ticks` ticks in one batch.
///
/// This single function powers three call sites: the live per-interval
/// tick while the app is open, launch catch-up (many ticks at once
/// after time away), and a manual player-triggered time skip. They
/// differ only in how many ticks they pass in and how the result is
/// presented, not in the underlying logic.
///
/// Each tick: the interest rate moves; scheduled market events fire
/// (price jumps and lingering effects); income (coupons, rent) is paid
/// on paydays; every instrument's price steps (funds last, since their
/// NAV is derived from their constituents' moves); NPC market-makers
/// re-quote and the player's resting limit orders are batch-cleared;
/// then the game layer runs (salary and bills, businesses, deals
/// settling, career reviews, rivals, random events and decisions,
/// goals). Net worth is recorded after all of that.
///
/// Prompts that arrive during the batch (a decision, a salary review)
/// expire on their own deadline, so catch-up never blocks. A manual
/// skip can pass `interrupt = true` to stop as soon as a new prompt
/// appears so the player can answer it.
pub fn fast_forward(
    player: &mut Player,
    instruments: &mut [InstrumentState],
    portfolio: &mut Portfolio,
    market: &mut Market,
    game: &mut GameState,
    ticks: i64,
) -> TickOutput {
    fast_forward_with_rng(player, instruments, portfolio, market, game, ticks, false, &mut rand::thread_rng())
}

/// `fast_forward` with a caller-supplied RNG (seeded in tests) and the
/// option to stop when a prompt appears.
#[allow(clippy::too_many_arguments)]
pub fn fast_forward_with_rng<R: Rng>(
    player: &mut Player,
    instruments: &mut [InstrumentState],
    portfolio: &mut Portfolio,
    market: &mut Market,
    game: &mut GameState,
    ticks: i64,
    interrupt: bool,
    rng: &mut R,
) -> TickOutput {
    let n = ticks.max(0) as usize;
    let mut out = TickOutput {
        price_points: Vec::with_capacity(instruments.len() * n),
        net_worth_points: Vec::with_capacity(n),
        ..Default::default()
    };
    let had_prompt = game.has_prompt();

    for _ in 0..ticks {
        if game.status.is_lost() {
            break;
        }
        player.current_tick += 1;
        out.ticks_run += 1;
        player.interest_rate = step_interest_rate(player.interest_rate, TICK_DT_YEARS, rng);

        // Prices before this tick's move, and each non-fund
        // instrument's income return for the tick (what funds need).
        let prev: HashMap<i32, f64> = instruments.iter().map(|s| (s.instrument.id, s.price)).collect();
        let income: HashMap<i32, f64> = instruments
            .iter()
            .map(|s| (s.instrument.id, income_return_per_tick(&s.instrument, s.price, TICK_DT_YEARS)))
            .collect();

        // Income shares the payday cadence: bond coupons, and rent
        // from instruments with an income yield.
        if player.current_tick % PAY_PERIOD_TICKS == 0 {
            let period_years = PAY_PERIOD_TICKS as f64 * TICK_DT_YEARS;
            let mut gross_income = 0.0;
            for state in instruments.iter() {
                let per_unit = match state.instrument.class {
                    AssetClass::Bond => state.instrument.face_value * state.instrument.coupon_rate * period_years,
                    _ => state.price * state.instrument.income_yield * period_years,
                };
                if per_unit > 0.0 {
                    gross_income += portfolio.quantity_of(state.instrument.id) * per_unit;
                }
            }
            // Taxed at source by the government.
            game.pay_investment_income(player, gross_income);
        }

        // Scheduled events fire here, so the jump is part of the move.
        game.pre_step(player, instruments);

        for state in instruments.iter_mut() {
            let inst = &state.instrument;
            let vol = inst.volatility * game.vol_mult(inst);
            state.price = match inst.class {
                AssetClass::MutualFund => continue, // second pass
                AssetClass::Bond => bond_price(inst.face_value, inst.coupon_rate, inst.duration, player.interest_rate),
                AssetClass::Forex => step_mean_reverting(
                    state.price,
                    inst.base_price,
                    FOREX_REVERSION_SPEED,
                    vol,
                    TICK_DT_YEARS,
                    rng,
                ),
                _ => {
                    let drift = inst.drift
                        + game.drift_shift(inst)
                        + game.economy.price_pressure(&inst.symbol)
                        - inst.rate_sensitivity * (player.interest_rate - LONG_RUN_INTEREST_RATE);
                    step_price(state.price, drift, vol, TICK_DT_YEARS, rng)
                }
            };
        }

        let now: HashMap<i32, f64> = instruments.iter().map(|s| (s.instrument.id, s.price)).collect();
        for state in instruments.iter_mut() {
            if state.instrument.class == AssetClass::MutualFund {
                state.price = step_fund(state, &prev, &now, &income, TICK_DT_YEARS);
            }
        }

        for state in instruments.iter() {
            out.price_points.push(PricePoint {
                instrument_id: state.instrument.id,
                tick: player.current_tick,
                price: state.price,
            });
        }

        // Market-makers re-quote at the new prices, then resting
        // orders are batch-cleared against the fresh ladders.
        market.requote(instruments);
        let cleared = market.clear_resting(player, portfolio, player.current_tick);
        out.fills.extend(cleared.fills);
        out.cancelled_orders.extend(cleared.cancelled);

        // The broad-market return this tick (first index fund), which
        // rivals' wealth is correlated with.
        let index_return = instruments
            .iter()
            .find(|s| s.instrument.class == AssetClass::IndexFund)
            .and_then(|s| prev.get(&s.instrument.id).map(|p0| s.price / p0 - 1.0))
            .unwrap_or(0.0);

        // The game layer: bills, deals, career, rivals, events.
        game.post_step(player, instruments, portfolio, market, index_return, rng, &mut out.fills);

        let prices: HashMap<i32, f64> = instruments.iter().map(|s| (s.instrument.id, s.price)).collect();
        let net_worth = player.cash + portfolio.market_value(&prices) + game.assets_value(&prices);
        game.check_goals(player, net_worth);
        out.net_worth_points.push(NetWorthPoint { tick: player.current_tick, net_worth });

        if interrupt && !had_prompt && game.has_prompt() {
            out.interrupted = true;
            break;
        }
    }

    out
}

/// How many whole ticks have elapsed since `last_tick_at`, given a
/// fixed real-world duration per tick. Drives launch catch-up.
pub fn ticks_elapsed_since(
    last_tick_at: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
    tick_real_seconds: i64,
) -> i64 {
    let elapsed_seconds = (now - last_tick_at).num_seconds();
    (elapsed_seconds / tick_real_seconds).max(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instrument::{Constituent, Instrument};
    use crate::player::GameMode;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    fn inst(id: i32, class: AssetClass, price: f64) -> InstrumentState {
        InstrumentState {
            instrument: Instrument {
                id,
                symbol: format!("T{id}"),
                name: format!("Test {id}"),
                class,
                base_price: price,
                drift: 0.05,
                volatility: 0.2,
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

    fn player() -> Player {
        Player {
            id: 1,
            game_mode: GameMode::SalariedInvestor,
            cash: 1_000.0,
            salary: 0.0,
            expenses: 0.0,
            current_tick: 0,
            interest_rate: LONG_RUN_INTEREST_RATE,
        }
    }

    fn run(instruments: &mut [InstrumentState], pf: &mut Portfolio, ticks: i64) -> (Player, TickOutput) {
        let mut p = player();
        let mut m = Market::new(instruments, vec![]);
        let mut g = GameState::quiet(&p);
        let mut rng = StdRng::seed_from_u64(7);
        let out = fast_forward_with_rng(&mut p, instruments, pf, &mut m, &mut g, ticks, false, &mut rng);
        (p, out)
    }

    #[test]
    fn fund_nav_follows_its_basket() {
        let mut fund = inst(3, AssetClass::MutualFund, 100.0);
        fund.instrument.volatility = 0.0;
        fund.instrument.constituents = vec![
            Constituent { instrument_id: 1, weight: 0.5 },
            Constituent { instrument_id: 2, weight: 0.5 },
        ];
        let mut set = [inst(1, AssetClass::Equity, 50.0), inst(2, AssetClass::Equity, 20.0), fund];
        let mut pf = Portfolio::default();
        let (_, out) = run(&mut set, &mut pf, 60);
        // Reconstruct: fund return per tick == mean of the two stocks' returns.
        let series = |id: i32| -> Vec<f64> {
            out.price_points.iter().filter(|p| p.instrument_id == id).map(|p| p.price).collect()
        };
        let (a, b, f) = (series(1), series(2), series(3));
        for t in 1..a.len() {
            let expected = f[t - 1] * (1.0 + 0.5 * (a[t] / a[t - 1] - 1.0) + 0.5 * (b[t] / b[t - 1] - 1.0));
            assert!((f[t] - expected).abs() < 1e-9, "tick {t}: {} vs {expected}", f[t]);
        }
    }

    #[test]
    fn fund_expense_ratio_is_a_drag() {
        let mk = |fee: f64| {
            let mut fund = inst(2, AssetClass::MutualFund, 100.0);
            fund.instrument.expense_ratio = fee;
            fund.instrument.constituents = vec![Constituent { instrument_id: 1, weight: 1.0 }];
            let mut set = [inst(1, AssetClass::Equity, 50.0), fund];
            let mut pf = Portfolio::default();
            run(&mut set, &mut pf, 365);
            set[1].price
        };
        let (free, pricey) = (mk(0.0), mk(0.02));
        let ratio = pricey / free;
        assert!(ratio < 0.99 && ratio > 0.97, "ratio {ratio}");
    }

    #[test]
    fn forex_stays_near_its_long_run_level() {
        let mut fx = inst(1, AssetClass::Forex, 1.10);
        fx.instrument.volatility = 0.08;
        let mut set = [fx];
        let mut pf = Portfolio::default();
        let (_, out) = run(&mut set, &mut pf, 365 * 3);
        assert!(out.price_points.iter().all(|p| p.price > 0.7 && p.price < 1.6));
    }

    #[test]
    fn rent_is_paid_on_payday() {
        let mut re = inst(1, AssetClass::RealEstate, 100.0);
        re.instrument.income_yield = 0.05;
        re.instrument.volatility = 0.0;
        re.instrument.drift = 0.0;
        let mut set = [re];
        let mut pf = Portfolio::default();
        pf.buy(1, 10.0, 100.0);
        let (p, _) = run(&mut set, &mut pf, PAY_PERIOD_TICKS);
        let expected = 10.0 * 100.0 * 0.05 * (PAY_PERIOD_TICKS as f64 / 365.0) * (1.0 - crate::economy::INVESTMENT_INCOME_RATE);
        assert!((p.cash - 1_000.0 - expected).abs() < 0.05, "cash {}", p.cash);
    }

    #[test]
    fn higher_rates_depress_rate_sensitive_drift() {
        let mut p = player();
        p.interest_rate = LONG_RUN_INTEREST_RATE;
        let mut re = inst(1, AssetClass::RealEstate, 100.0);
        re.instrument.volatility = 0.0;
        re.instrument.drift = 0.05;
        re.instrument.rate_sensitivity = 2.0;
        // With zero vol the price path is deterministic given the rate.
        let eff = |rate: f64| re.instrument.drift - re.instrument.rate_sensitivity * (rate - LONG_RUN_INTEREST_RATE);
        assert!(eff(0.08) < eff(0.04));
        assert!(eff(0.01) > eff(0.04));
    }

    #[test]
    fn resting_limit_order_fills_during_fast_forward() {
        let mut set = [inst(1, AssetClass::Equity, 100.0)];
        set[0].instrument.volatility = 0.4;
        let mut p = player();
        p.cash = 100_000.0;
        let mut pf = Portfolio::default();
        let mut m = Market::new(&set, vec![]);
        // Buy limit 1% under market: over a year of a 40%-vol walk it will fill.
        m.limit_order(&mut p, &mut pf, 1, true, 10.0, 99.0).unwrap();
        let mut g = GameState::quiet(&p);
        let mut rng = StdRng::seed_from_u64(3);
        let out = fast_forward_with_rng(&mut p, &mut set, &mut pf, &mut m, &mut g, 365, false, &mut rng);
        assert_eq!(out.fills.len(), 1);
        assert!(pf.quantity_of(1) > 9.99);
        assert!(m.orders.is_empty());
    }
}
