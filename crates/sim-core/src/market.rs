//! Order book execution with NPC market-makers.
//!
//! The pricing engine still produces one "fair" price per instrument
//! per tick. On top of that, an NPC market-maker per instrument quotes
//! a ladder of limit orders around that price: asks above it, bids
//! below it, with a class-specific spread and a finite amount of depth
//! per level. Everything the player does trades against that ladder:
//!
//! * **Market orders** walk the ladder immediately. You pay the spread,
//!   large orders get worse prices as they eat deeper levels
//!   (slippage), and orders bigger than the visible depth fill only
//!   partially.
//! * **Limit orders** fill immediately against any levels that cross
//!   their limit; whatever is left rests in the book.
//! * **Batch clearing**: once per tick, after prices move, the
//!   market-makers re-quote around the new price (restoring depth) and
//!   all resting player orders are matched against the fresh ladder in
//!   order of placement. That is how a resting limit order gets filled
//!   while time passes - including during catch-up and time skips.
//!
//! Resting buy orders *reserve* cash (limit price x remaining quantity)
//! and resting sell orders reserve units, without moving anything out
//! of the portfolio. Reservations are checked when an order is placed
//! so cash/units can't be committed twice.

use std::collections::HashMap;

use crate::instrument::{AssetClass, InstrumentState};
use crate::player::Player;
use crate::portfolio::Portfolio;

const EPS: f64 = 1e-9;

/// How an NPC market-maker quotes one class of instrument.
#[derive(Debug, Clone, Copy)]
pub struct MakerParams {
    /// Distance of the best bid/ask from the fair price, as a fraction.
    pub half_spread: f64,
    /// Extra distance per additional level, as a fraction of price.
    pub level_step: f64,
    pub levels: usize,
    /// Cash value offered at each level.
    pub level_notional: f64,
}

/// Spreads and depth by asset class: liquid, standardized things (FX,
/// index funds) are tight and deep; property and crypto are wide and
/// shallow. Savings is a deposit, not a market: no spread, no limit.
pub fn maker_params(class: AssetClass) -> MakerParams {
    let (half_spread, level_step, levels, level_notional) = match class {
        AssetClass::Equity => (0.0015, 0.0010, 5, 8_000.0),
        AssetClass::IndexFund => (0.0005, 0.0005, 5, 15_000.0),
        AssetClass::MutualFund => (0.0005, 0.0005, 5, 15_000.0),
        AssetClass::Savings => (0.0, 0.0, 1, 1e9),
        AssetClass::Commodity => (0.0030, 0.0015, 5, 6_000.0),
        AssetClass::Bond => (0.0020, 0.0010, 5, 12_000.0),
        AssetClass::RealEstate => (0.0100, 0.0040, 4, 20_000.0),
        AssetClass::Forex => (0.0003, 0.0002, 5, 50_000.0),
        AssetClass::Crypto => (0.0060, 0.0030, 5, 3_000.0),
    };
    MakerParams { half_spread, level_step, levels, level_notional }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BookLevel {
    pub price: f64,
    pub qty: f64,
}

/// One instrument's NPC ladder. `bids` are sorted best (highest) first,
/// `asks` best (lowest) first.
#[derive(Debug, Clone, Default)]
pub struct OrderBook {
    pub bids: Vec<BookLevel>,
    pub asks: Vec<BookLevel>,
}

impl OrderBook {
    /// A fresh, fully-stocked ladder around `mid`.
    pub fn quote(mid: f64, p: &MakerParams) -> Self {
        let qty = p.level_notional / mid;
        let asks = (0..p.levels)
            .map(|k| BookLevel {
                price: mid * (1.0 + p.half_spread + k as f64 * p.level_step),
                qty,
            })
            .collect();
        let bids = (0..p.levels)
            .map(|k| BookLevel {
                price: (mid * (1.0 - p.half_spread - k as f64 * p.level_step)).max(0.0001),
                qty,
            })
            .collect();
        OrderBook { bids, asks }
    }

    pub fn best_bid(&self) -> Option<f64> {
        self.bids.first().map(|l| l.price)
    }

    pub fn best_ask(&self) -> Option<f64> {
        self.asks.first().map(|l| l.price)
    }

    /// Buys up to `qty` from the asks, best price first, only at prices
    /// <= `limit` (None = market order). Consumes depth. Returns
    /// (quantity filled, total cost).
    pub fn take_buy(&mut self, qty: f64, limit: Option<f64>) -> (f64, f64) {
        Self::take(&mut self.asks, qty, |price| limit.map_or(true, |l| price <= l + EPS))
    }

    /// Sells up to `qty` into the bids, best price first, only at prices
    /// >= `limit` (None = market order). Returns (quantity filled,
    /// total proceeds).
    pub fn take_sell(&mut self, qty: f64, limit: Option<f64>) -> (f64, f64) {
        Self::take(&mut self.bids, qty, |price| limit.map_or(true, |l| price >= l - EPS))
    }

    fn take(side: &mut Vec<BookLevel>, qty: f64, acceptable: impl Fn(f64) -> bool) -> (f64, f64) {
        let mut remaining = qty;
        let mut filled = 0.0;
        let mut notional = 0.0;
        for level in side.iter_mut() {
            if remaining <= EPS || !acceptable(level.price) {
                break;
            }
            let take = remaining.min(level.qty);
            level.qty -= take;
            remaining -= take;
            filled += take;
            notional += take * level.price;
        }
        side.retain(|l| l.qty > EPS);
        (filled, notional)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillKind {
    /// Market order.
    Market,
    /// The immediately-crossing part of a limit order, at placement.
    LimitImmediate,
    /// A resting limit order filled during a tick's batch clearing.
    LimitResting,
    /// Sold automatically to cover bills you couldn't pay.
    Forced,
}

impl FillKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FillKind::Market => "market",
            FillKind::LimitImmediate => "limit",
            FillKind::LimitResting => "limit_resting",
            FillKind::Forced => "forced",
        }
    }

    pub fn from_db_str(s: &str) -> FillKind {
        match s {
            "limit" => FillKind::LimitImmediate,
            "limit_resting" => FillKind::LimitResting,
            "forced" => FillKind::Forced,
            _ => FillKind::Market,
        }
    }
}

/// An execution: `quantity` units at volume-weighted average `price`.
#[derive(Debug, Clone, PartialEq)]
pub struct Fill {
    pub instrument_id: i32,
    pub is_buy: bool,
    pub quantity: f64,
    pub price: f64,
    pub tick: i64,
    pub kind: FillKind,
    /// Profit (or loss) this sale realized against your average cost,
    /// before tax. Zero for buys.
    pub realized_pnl: f64,
}

/// A player's resting limit order. `quantity` is the unfilled remainder.
#[derive(Debug, Clone, PartialEq)]
pub struct LimitOrder {
    pub id: i64,
    pub instrument_id: i32,
    pub is_buy: bool,
    pub quantity: f64,
    pub limit_price: f64,
    pub placed_tick: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TradeError {
    InvalidInput,
    UnknownInstrument,
    /// Nothing available at an acceptable price (market order only).
    NoLiquidity,
    InsufficientCash { needed: f64, available: f64 },
    InsufficientHoldings { requested: f64, available: f64 },
}

impl std::fmt::Display for TradeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TradeError::InvalidInput => write!(f, "Invalid quantity or price"),
            TradeError::UnknownInstrument => write!(f, "Unknown instrument"),
            TradeError::NoLiquidity => write!(f, "No liquidity at an acceptable price"),
            TradeError::InsufficientCash { needed, available } => {
                write!(f, "Not enough free cash: need {needed:.2}, have {available:.2}")
            }
            TradeError::InsufficientHoldings { requested, available } => write!(
                f,
                "Not enough free units: want {requested:.4}, have {available:.4} (open sell orders reserve units)"
            ),
        }
    }
}

impl std::error::Error for TradeError {}

/// Result of placing a limit order.
#[derive(Debug, Clone, PartialEq)]
pub struct LimitOutcome {
    /// The part that crossed the book right away, if any.
    pub fill: Option<Fill>,
    /// The part now resting in the book, if any.
    pub resting: Option<LimitOrder>,
}

/// Result of one batch-clearing pass over the resting orders.
#[derive(Debug, Clone, Default)]
pub struct ClearingResult {
    pub fills: Vec<Fill>,
    /// Orders dropped because the player could no longer fund them.
    pub cancelled: Vec<LimitOrder>,
}

/// All order-book state for one save: the NPC ladders (rebuilt from
/// prices, never persisted) and the player's resting orders (persisted).
#[derive(Debug, Clone, Default)]
pub struct Market {
    pub books: HashMap<i32, OrderBook>,
    pub orders: Vec<LimitOrder>,
    next_order_id: i64,
}

impl Market {
    pub fn new(instruments: &[InstrumentState], orders: Vec<LimitOrder>) -> Self {
        let next_order_id = orders.iter().map(|o| o.id).max().unwrap_or(0) + 1;
        let mut m = Market { books: HashMap::new(), orders, next_order_id };
        m.requote(instruments);
        m
    }

    /// NPC market-makers re-quote around the current prices, restoring
    /// every ladder to full depth.
    pub fn requote(&mut self, instruments: &[InstrumentState]) {
        for s in instruments {
            self.books.insert(
                s.instrument.id,
                OrderBook::quote(s.price, &maker_params(s.instrument.class)),
            );
        }
    }

    /// Cash tied up by resting buy orders.
    pub fn reserved_cash(&self) -> f64 {
        self.orders
            .iter()
            .filter(|o| o.is_buy)
            .map(|o| o.quantity * o.limit_price)
            .sum()
    }

    /// Units tied up by resting sell orders for one instrument.
    pub fn reserved_units(&self, instrument_id: i32) -> f64 {
        self.orders
            .iter()
            .filter(|o| !o.is_buy && o.instrument_id == instrument_id)
            .map(|o| o.quantity)
            .sum()
    }

    pub fn available_cash(&self, player: &Player) -> f64 {
        player.cash - self.reserved_cash()
    }

    pub fn available_units(&self, portfolio: &Portfolio, instrument_id: i32) -> f64 {
        portfolio.quantity_of(instrument_id) - self.reserved_units(instrument_id)
    }

    pub fn orders_for(&self, instrument_id: i32) -> impl Iterator<Item = &LimitOrder> {
        self.orders.iter().filter(move |o| o.instrument_id == instrument_id)
    }

    /// Cancels every resting order on one instrument; returns how many.
    pub fn cancel_orders_for(&mut self, instrument_id: i32) -> usize {
        let before = self.orders.len();
        self.orders.retain(|o| o.instrument_id != instrument_id);
        before - self.orders.len()
    }

    /// Executes against the ladder right now. May fill only part of
    /// `qty` if the visible depth runs out.
    pub fn market_order(
        &mut self,
        player: &mut Player,
        portfolio: &mut Portfolio,
        instrument_id: i32,
        is_buy: bool,
        qty: f64,
    ) -> Result<Fill, TradeError> {
        if !(qty.is_finite() && qty > 0.0) {
            return Err(TradeError::InvalidInput);
        }
        let tick = player.current_tick;
        self.execute_now(player, portfolio, instrument_id, is_buy, qty, None, tick, FillKind::Market)?
            .ok_or(TradeError::NoLiquidity)
    }

    /// Places a limit order: crosses what it can immediately, rests the
    /// remainder.
    pub fn limit_order(
        &mut self,
        player: &mut Player,
        portfolio: &mut Portfolio,
        instrument_id: i32,
        is_buy: bool,
        qty: f64,
        limit_price: f64,
    ) -> Result<LimitOutcome, TradeError> {
        if !(qty.is_finite() && qty > 0.0 && limit_price.is_finite() && limit_price > 0.0) {
            return Err(TradeError::InvalidInput);
        }
        if !self.books.contains_key(&instrument_id) {
            return Err(TradeError::UnknownInstrument);
        }

        // Reserve up front for the worst case (whole order at the limit).
        if is_buy {
            let available = self.available_cash(player);
            let needed = qty * limit_price;
            if needed > available + EPS {
                return Err(TradeError::InsufficientCash { needed, available });
            }
        } else {
            let available = self.available_units(portfolio, instrument_id);
            if qty > available + EPS {
                return Err(TradeError::InsufficientHoldings { requested: qty, available });
            }
        }

        let tick = player.current_tick;
        let fill = self.execute_now(
            player,
            portfolio,
            instrument_id,
            is_buy,
            qty,
            Some(limit_price),
            tick,
            FillKind::LimitImmediate,
        )?;

        let remaining = qty - fill.as_ref().map_or(0.0, |f| f.quantity);
        let resting = if remaining > EPS {
            let order = LimitOrder {
                id: self.next_order_id,
                instrument_id,
                is_buy,
                quantity: remaining,
                limit_price,
                placed_tick: tick,
            };
            self.next_order_id += 1;
            self.orders.push(order.clone());
            Some(order)
        } else {
            None
        };

        Ok(LimitOutcome { fill, resting })
    }

    /// Shared immediate-execution path. Dry-runs the walk on a copy of
    /// the book so a rejected trade doesn't consume depth. Returns
    /// `Ok(None)` if nothing was available at an acceptable price.
    #[allow(clippy::too_many_arguments)]
    fn execute_now(
        &mut self,
        player: &mut Player,
        portfolio: &mut Portfolio,
        instrument_id: i32,
        is_buy: bool,
        qty: f64,
        limit: Option<f64>,
        tick: i64,
        kind: FillKind,
    ) -> Result<Option<Fill>, TradeError> {
        let book = self.books.get(&instrument_id).ok_or(TradeError::UnknownInstrument)?;
        let mut trial = book.clone();

        if is_buy {
            let (filled, cost) = trial.take_buy(qty, limit);
            if filled <= EPS {
                return Ok(None);
            }
            // Orders already resting hold their own reservations; a
            // market order may only spend what's left over.
            let available = self.available_cash(player);
            // A limit order's own reservation was checked at placement
            // and isn't in `orders` yet, so only compare against free cash.
            if cost > available + EPS {
                return Err(TradeError::InsufficientCash { needed: cost, available });
            }
            let price = cost / filled;
            portfolio.buy(instrument_id, filled, price);
            player.cash -= cost;
            self.books.insert(instrument_id, trial);
            Ok(Some(Fill { instrument_id, is_buy, quantity: filled, price, tick, kind, realized_pnl: 0.0 }))
        } else {
            let available = self.available_units(portfolio, instrument_id);
            if qty > available + EPS {
                return Err(TradeError::InsufficientHoldings { requested: qty, available });
            }
            let (filled, proceeds) = trial.take_sell(qty, limit);
            if filled <= EPS {
                return Ok(None);
            }
            let price = proceeds / filled;
            let gains_before = portfolio.realized_gains;
            match portfolio.sell(instrument_id, filled, price) {
                Some(p) => player.cash += p,
                None => {
                    return Err(TradeError::InsufficientHoldings {
                        requested: filled,
                        available: portfolio.quantity_of(instrument_id),
                    })
                }
            }
            self.books.insert(instrument_id, trial);
            let realized_pnl = portfolio.realized_gains - gains_before;
            Ok(Some(Fill { instrument_id, is_buy, quantity: filled, price, tick, kind, realized_pnl }))
        }
    }

    /// Batch clearing: match every resting order, oldest first, against
    /// the current ladders. Call after `requote` once the tick's prices
    /// are known. Orders compete for the same depth, so a large earlier
    /// order can leave less for later ones.
    pub fn clear_resting(&mut self, player: &mut Player, portfolio: &mut Portfolio, tick: i64) -> ClearingResult {
        let mut result = ClearingResult::default();
        let orders = std::mem::take(&mut self.orders);
        let mut kept: Vec<LimitOrder> = Vec::with_capacity(orders.len());

        for mut order in orders {
            let Some(book) = self.books.get_mut(&order.instrument_id) else {
                kept.push(order);
                continue;
            };

            let mut trial = book.clone();
            let (filled, value) = if order.is_buy {
                trial.take_buy(order.quantity, Some(order.limit_price))
            } else {
                trial.take_sell(order.quantity, Some(order.limit_price))
            };

            if filled > EPS {
                let price = value / filled;
                let gains_before = portfolio.realized_gains;
                if order.is_buy {
                    // Salary/expenses can shrink cash below what an old
                    // order was reserved against; an unfundable order
                    // is cancelled rather than letting cash go negative.
                    if value > player.cash + EPS {
                        result.cancelled.push(order);
                        continue;
                    }
                    portfolio.buy(order.instrument_id, filled, price);
                    player.cash -= value;
                } else {
                    match portfolio.sell(order.instrument_id, filled, price) {
                        Some(p) => player.cash += p,
                        None => {
                            result.cancelled.push(order);
                            continue;
                        }
                    }
                }
                *book = trial;
                result.fills.push(Fill {
                    instrument_id: order.instrument_id,
                    is_buy: order.is_buy,
                    quantity: filled,
                    price,
                    tick,
                    kind: FillKind::LimitResting,
                    realized_pnl: portfolio.realized_gains - gains_before,
                });
                order.quantity -= filled;
            }

            if order.quantity > EPS {
                kept.push(order);
            }
        }

        self.orders = kept;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instrument::Instrument;
    use crate::player::GameMode;

    fn state(id: i32, class: AssetClass, price: f64) -> InstrumentState {
        InstrumentState {
            instrument: Instrument {
                id,
                symbol: format!("T{id}"),
                name: format!("Test {id}"),
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
            salary: 0.0,
            expenses: 0.0,
            current_tick: 5,
            interest_rate: 0.04,
        }
    }

    #[test]
    fn ladder_is_centered_with_spread() {
        let b = OrderBook::quote(100.0, &maker_params(AssetClass::Equity));
        assert!(b.best_ask().unwrap() > 100.0);
        assert!(b.best_bid().unwrap() < 100.0);
        assert_eq!(b.asks.len(), 5);
        assert!(b.asks.windows(2).all(|w| w[0].price < w[1].price));
        assert!(b.bids.windows(2).all(|w| w[0].price > w[1].price));
    }

    #[test]
    fn savings_has_no_spread() {
        let b = OrderBook::quote(1.0, &maker_params(AssetClass::Savings));
        assert_eq!(b.best_ask(), Some(1.0));
        assert_eq!(b.best_bid(), Some(1.0));
    }

    #[test]
    fn market_buy_pays_the_spread() {
        let inst = [state(1, AssetClass::Equity, 100.0)];
        let mut m = Market::new(&inst, vec![]);
        let (mut p, mut pf) = (player(10_000.0), Portfolio::default());
        let fill = m.market_order(&mut p, &mut pf, 1, true, 10.0).unwrap();
        assert!(fill.price > 100.0);
        assert!((fill.quantity - 10.0).abs() < 1e-9);
        assert!((p.cash - (10_000.0 - 10.0 * fill.price)).abs() < 1e-6);
        assert!((pf.quantity_of(1) - 10.0).abs() < 1e-9);
    }

    #[test]
    fn big_market_buy_walks_levels_and_partially_fills() {
        let inst = [state(1, AssetClass::Equity, 100.0)];
        let mut m = Market::new(&inst, vec![]);
        let (mut p, mut pf) = (player(1_000_000.0), Portfolio::default());
        let level_qty = m.books[&1].asks[0].qty;
        // Two levels' worth: VWAP must sit between level 0 and level 1.
        let fill = m.market_order(&mut p, &mut pf, 1, true, level_qty * 2.0).unwrap();
        let book_after = &m.books[&1];
        assert_eq!(book_after.asks.len(), 3);
        assert!(fill.price > OrderBook::quote(100.0, &maker_params(AssetClass::Equity)).asks[0].price);
        // Far more than all visible depth: only the depth fills.
        let total_depth: f64 = book_after.asks.iter().map(|l| l.qty).sum();
        let fill2 = m.market_order(&mut p, &mut pf, 1, true, total_depth * 10.0).unwrap();
        assert!((fill2.quantity - total_depth).abs() < 1e-6);
        assert!(m.books[&1].asks.is_empty());
        assert_eq!(
            m.market_order(&mut p, &mut pf, 1, true, 1.0),
            Err(TradeError::NoLiquidity)
        );
    }

    #[test]
    fn rejected_trade_does_not_consume_depth() {
        let inst = [state(1, AssetClass::Equity, 100.0)];
        let mut m = Market::new(&inst, vec![]);
        let (mut p, mut pf) = (player(50.0), Portfolio::default());
        let before = m.books[&1].asks.clone();
        assert!(matches!(
            m.market_order(&mut p, &mut pf, 1, true, 10.0),
            Err(TradeError::InsufficientCash { .. })
        ));
        assert_eq!(m.books[&1].asks, before);
        assert_eq!(p.cash, 50.0);
    }

    #[test]
    fn sell_requires_holdings_and_receives_bid() {
        let inst = [state(1, AssetClass::Equity, 100.0)];
        let mut m = Market::new(&inst, vec![]);
        let (mut p, mut pf) = (player(0.0), Portfolio::default());
        assert!(matches!(
            m.market_order(&mut p, &mut pf, 1, false, 1.0),
            Err(TradeError::InsufficientHoldings { .. })
        ));
        pf.buy(1, 10.0, 100.0);
        let fill = m.market_order(&mut p, &mut pf, 1, false, 10.0).unwrap();
        assert!(fill.price < 100.0);
        assert!(p.cash > 0.0);
        assert_eq!(pf.quantity_of(1), 0.0);
    }

    #[test]
    fn limit_below_market_rests_then_fills_when_price_falls() {
        let mut inst = [state(1, AssetClass::Equity, 100.0)];
        let mut m = Market::new(&inst, vec![]);
        let (mut p, mut pf) = (player(10_000.0), Portfolio::default());

        let out = m.limit_order(&mut p, &mut pf, 1, true, 10.0, 95.0).unwrap();
        assert!(out.fill.is_none());
        let order = out.resting.unwrap();
        assert_eq!(p.cash, 10_000.0); // nothing moves while resting
        assert!((m.reserved_cash() - 950.0).abs() < 1e-9);
        assert!((m.available_cash(&p) - 9_050.0).abs() < 1e-9);

        // Price unchanged: still resting.
        let r = m.clear_resting(&mut p, &mut pf, 6);
        assert!(r.fills.is_empty());
        assert_eq!(m.orders.len(), 1);

        // Price drops to 90: asks now start below 95, order fills.
        inst[0].price = 90.0;
        m.requote(&inst);
        let r = m.clear_resting(&mut p, &mut pf, 7);
        assert_eq!(r.fills.len(), 1);
        let f = &r.fills[0];
        assert_eq!(f.kind, FillKind::LimitResting);
        assert!(f.price <= 95.0 && f.price > 89.0);
        assert!((f.quantity - order.quantity).abs() < 1e-9);
        assert!(m.orders.is_empty());
        assert!((pf.quantity_of(1) - 10.0).abs() < 1e-9);
        assert!(p.cash < 10_000.0 - 10.0 * 89.0);
    }

    #[test]
    fn crossing_limit_fills_immediately_and_rests_remainder() {
        let inst = [state(1, AssetClass::Equity, 100.0)];
        let mut m = Market::new(&inst, vec![]);
        let (mut p, mut pf) = (player(1_000_000.0), Portfolio::default());
        let level0 = m.books[&1].asks[0].clone();
        // Willing to pay up to just above level 0 only.
        let limit = level0.price + 0.0001;
        let out = m.limit_order(&mut p, &mut pf, 1, true, level0.qty * 2.0, limit).unwrap();
        let fill = out.fill.unwrap();
        assert!((fill.quantity - level0.qty).abs() < 1e-6);
        assert!((fill.price - level0.price).abs() < 1e-9);
        let rest = out.resting.unwrap();
        assert!((rest.quantity - level0.qty).abs() < 1e-6);
    }

    #[test]
    fn reservations_prevent_double_spending() {
        let inst = [state(1, AssetClass::Equity, 100.0)];
        let mut m = Market::new(&inst, vec![]);
        let (mut p, mut pf) = (player(1_000.0), Portfolio::default());
        m.limit_order(&mut p, &mut pf, 1, true, 8.0, 90.0).unwrap(); // reserves 720
        assert!(matches!(
            m.limit_order(&mut p, &mut pf, 1, true, 5.0, 90.0), // needs 450, only 280 free
            Err(TradeError::InsufficientCash { .. })
        ));
        assert!(matches!(
            m.market_order(&mut p, &mut pf, 1, true, 5.0), // ~500 > 280 free
            Err(TradeError::InsufficientCash { .. })
        ));
    }

    #[test]
    fn sell_orders_reserve_units() {
        let inst = [state(1, AssetClass::Equity, 100.0)];
        let mut m = Market::new(&inst, vec![]);
        let (mut p, mut pf) = (player(0.0), Portfolio::default());
        pf.buy(1, 10.0, 100.0);
        m.limit_order(&mut p, &mut pf, 1, false, 7.0, 150.0).unwrap();
        assert!(matches!(
            m.market_order(&mut p, &mut pf, 1, false, 5.0),
            Err(TradeError::InsufficientHoldings { .. })
        ));
        assert_eq!(m.cancel_orders_for(1), 1);
        assert!(m.market_order(&mut p, &mut pf, 1, false, 5.0).is_ok());
    }

    #[test]
    fn unfundable_resting_order_is_cancelled() {
        let mut inst = [state(1, AssetClass::Equity, 100.0)];
        let mut m = Market::new(&inst, vec![]);
        let (mut p, mut pf) = (player(1_000.0), Portfolio::default());
        m.limit_order(&mut p, &mut pf, 1, true, 10.0, 95.0).unwrap();
        p.cash = 10.0; // e.g. expenses ate the balance
        inst[0].price = 80.0;
        m.requote(&inst);
        let r = m.clear_resting(&mut p, &mut pf, 9);
        assert!(r.fills.is_empty());
        assert_eq!(r.cancelled.len(), 1);
        assert!(m.orders.is_empty());
        assert_eq!(p.cash, 10.0);
    }

    #[test]
    fn order_ids_continue_after_reload() {
        let inst = [state(1, AssetClass::Equity, 100.0)];
        let existing = LimitOrder {
            id: 41,
            instrument_id: 1,
            is_buy: true,
            quantity: 1.0,
            limit_price: 50.0,
            placed_tick: 0,
        };
        let mut m = Market::new(&inst, vec![existing]);
        let (mut p, mut pf) = (player(1_000.0), Portfolio::default());
        let out = m.limit_order(&mut p, &mut pf, 1, true, 1.0, 50.0).unwrap();
        assert_eq!(out.resting.unwrap().id, 42);
    }

    #[test]
    fn sells_report_the_profit_against_your_average_cost() {
        let inst = [state(1, AssetClass::Equity, 100.0)];
        let mut mk = Market::new(&inst, vec![]);
        let (mut p, mut pf) = (player(0.0), Portfolio::default());
        pf.buy(1, 10.0, 80.0); // bought at 80, now ~100
        let fill = mk.market_order(&mut p, &mut pf, 1, false, 10.0).unwrap();
        let expected = (fill.price - 80.0) * 10.0;
        assert!(expected > 150.0);
        assert!((fill.realized_pnl - expected).abs() < 1e-6, "{} vs {expected}", fill.realized_pnl);
        // Buys never carry a realized figure; a loss shows as negative.
        let mut p = player(10_000.0);
        let buy = mk.market_order(&mut p, &mut pf, 1, true, 5.0).unwrap();
        assert_eq!(buy.realized_pnl, 0.0);
        pf.positions.get_mut(&1).unwrap().avg_cost = 150.0;
        let loss = mk.market_order(&mut p, &mut pf, 1, false, 5.0).unwrap();
        assert!(loss.realized_pnl < -200.0);
    }

    #[test]
    fn resting_sells_also_record_profit() {
        let mut inst = [state(1, AssetClass::Equity, 100.0)];
        let mut mk = Market::new(&inst, vec![]);
        let (mut p, mut pf) = (player(0.0), Portfolio::default());
        pf.buy(1, 10.0, 50.0);
        mk.limit_order(&mut p, &mut pf, 1, false, 10.0, 130.0).unwrap();
        inst[0].price = 140.0;
        mk.requote(&inst);
        let r = mk.clear_resting(&mut p, &mut pf, 9);
        assert_eq!(r.fills.len(), 1);
        assert!(r.fills[0].realized_pnl > 800.0, "{}", r.fills[0].realized_pnl);
    }
}
