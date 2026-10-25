use std::collections::HashMap;

use chrono::{DateTime, Utc};
use ratatui::widgets::TableState;
use sim_core::{AssetClass, Fill, GameState, InstrumentState, Market, Player, Portfolio};

/// What the footer/input line is currently doing. Normal mode reads
/// single keypresses as commands; the other modes are capturing input
/// for a pending action. Only holds Copy fields, so it can be matched
/// by value out of `&mut App` in the event loop.
#[derive(Clone, Copy)]
pub enum InputMode {
    Normal,
    /// Market order quantity.
    Quantity { instrument_id: i32, is_buy: bool },
    SkipTicks,
    /// Limit order, step 1: waiting for `b` (buy) or `s` (sell).
    LimitSide { instrument_id: i32 },
    /// Limit order, step 2: quantity.
    LimitQuantity { instrument_id: i32, is_buy: bool },
    /// Limit order, step 3: limit price.
    LimitPrice { instrument_id: i32, is_buy: bool, qty: f64 },
    /// `c`: career menu (1 train, 2 job hunt).
    CareerMenu,
    /// `v`: business menu (1 buy, 2 sell, 3 upgrade).
    BusinessMenu,
    /// Pick a listing / owned business by number.
    BuyBusiness,
    SellBusiness,
    UpgradeBusiness,
    /// `m`: money menu (1 borrow, 2 repay).
    MoneyMenu,
    Borrow,
    Repay,
    Deposit,
    Withdraw,
    /// `h`: housing menu (1 buy a home, 2 sell yours).
    HousingMenu,
    BuyHome,
    ConfirmSellHome,
    /// `a`: assets menu (1 buy, 2 sell, 3 repay a mortgage).
    AssetMenu,
    BuyAsset,
    /// A financeable asset: 1 = buy-to-let mortgage, 2 = pay cash.
    BuyAssetFinance { kind: usize },
    SellAsset,
    ConfirmSellAsset { index: usize },
    PrepayAssetPick,
    PrepayAssetAmount { index: usize },
    /// Forward contract, step 1: `b` (long) or `s` (short).
    ForwardSide { instrument_id: i32 },
    ForwardQty { instrument_id: i32, is_long: bool },
    /// Step 3: 1/2/3 for 30/60/90 ticks.
    ForwardHorizon { instrument_id: i32, is_long: bool, qty: f64 },
}

/// What the right-hand panel is currently showing. Cycled with 'w':
/// InstrumentPrice -> NetWorth -> Allocation -> OrderBook -> Trades ->
/// Life -> News -> Deals -> Report -> Economy.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ChartMode {
    /// Price history of whichever instrument is selected.
    InstrumentPrice,
    /// This save's total net worth over time - cash plus the market
    /// value of every position, cash and each instrument being
    /// exactly what feeds `Player::apply_tick_cashflow` and the price
    /// engine respectively.
    NetWorth,
    /// Current allocation across cash and each asset class - the
    /// diversification snapshot, as opposed to NetWorth's trend.
    Allocation,
    /// The selected instrument's NPC order book ladder, plus the
    /// player's resting limit orders on it.
    OrderBook,
    /// Recent executions (market, limit, and resting-limit fills).
    Trades,
    /// Career, goals and the leaderboard against your rivals.
    Life,
    /// The news feed: events, decisions, deals, milestones.
    News,
    /// Forward contracts, businesses, listings, and NPC relationships.
    Deals,
    /// Where your money came from and went; the run report.
    Report,
    /// Macro indicators: inflation, growth, unemployment, rates, government.
    Economy,
    /// Savings, loan and housing in one place.
    Money,
    /// The full cash ledger from SQL, newest first.
    Transactions,
    /// The finance manual: terms, examples, and why they matter.
    Manual,
}

/// The five top-level screens, jumped to with 1-5. Each has one or more
/// views, cycled with `w`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Market,
    Portfolio,
    Life,
    Deals,
    World,
    Logs,
    Manual,
}

impl Tab {
    pub const ALL: [Tab; 7] = [Tab::Market, Tab::Portfolio, Tab::Life, Tab::Deals, Tab::World, Tab::Logs, Tab::Manual];

    pub fn name(self) -> &'static str {
        match self {
            Tab::Market => "Market",
            Tab::Portfolio => "Portfolio",
            Tab::Life => "Life",
            Tab::Deals => "Deals",
            Tab::World => "World",
            Tab::Logs => "Logs",
            Tab::Manual => "Manual",
        }
    }

    pub fn views(self) -> &'static [ChartMode] {
        match self {
            Tab::Market => &[ChartMode::InstrumentPrice, ChartMode::OrderBook],
            Tab::Portfolio => &[ChartMode::Allocation, ChartMode::NetWorth],
            Tab::Life => &[ChartMode::Life, ChartMode::Report],
            Tab::Deals => &[ChartMode::Deals, ChartMode::Money],
            Tab::World => &[ChartMode::Economy],
            Tab::Logs => &[ChartMode::Transactions, ChartMode::News, ChartMode::Trades],
            Tab::Manual => &[ChartMode::Manual],
        }
    }
}


/// How many recent fills the Trades view keeps in memory.
pub const FILLS_LEN: usize = 40;

/// One slice of the allocation breakdown - either cash, or everything
/// held in one asset class.
pub struct AllocationSlice {
    pub label: String,
    pub value: f64,
    /// 0.0-1.0 share of total net worth. 0.0 if net worth is ~0.
    pub fraction: f64,
}

pub struct App {
    pub save_name: String,
    pub player: Player,
    /// When this save's ticks were last advanced/persisted - drives
    /// this save's own catch-up math, independent of any other save.
    pub last_tick_at: DateTime<Utc>,
    pub instruments: Vec<InstrumentState>,
    pub portfolio: Portfolio,
    /// NPC order books and this save's resting limit orders.
    pub market: Market,
    /// Career, businesses, NPCs, rivals, events, news, win/lose status.
    pub game: GameState,
    /// False while the player has tucked a pending prompt away (Tab).
    pub modal_visible: bool,
    /// The `?` help overlay.
    pub show_help: bool,
    /// When the clock next advances a day by itself (for the header countdown).
    pub next_auto_tick: std::time::Instant,
    /// Closed months from the SQL ledger (monthly_summary), oldest first.
    pub cashflow: Vec<db::CashflowRow>,
    /// Lifetime ledger totals per category (maintained by a SQL trigger).
    pub ledger_totals: Vec<(String, f64)>,
    /// Newest cash ledger rows (SQL transactions table), newest first.
    pub transactions: Vec<db::LedgerRow>,
    /// How far the Logs view is scrolled down.
    pub log_scroll: usize,
    /// The manual entry being read (index into manual::ENTRIES).
    pub manual_selected: usize,
    /// Where Esc returns to from the manual.
    pub prev_tab: Tab,
    /// Recent executions, oldest first, capped at FILLS_LEN.
    pub fills: Vec<Fill>,
    pub table_state: TableState,
    /// Recent price points per instrument, oldest first, capped at
    /// HISTORY_LEN - what the instrument chart panel draws.
    pub history: HashMap<i32, Vec<f64>>,
    /// Recent net worth points, oldest first, capped at HISTORY_LEN -
    /// what the net worth chart panel draws.
    pub net_worth_history: Vec<f64>,
    pub chart_mode: ChartMode,
    pub tab: Tab,
    pub selected: usize,
    pub input_mode: InputMode,
    pub input_buffer: String,
    /// Highlighted row while choosing from a list (homes, businesses, assets).
    pub pick: usize,
    pub status: String,
    pub should_quit: bool,
}

impl App {
    pub fn selected_instrument(&self) -> Option<&InstrumentState> {
        self.instruments.get(self.selected)
    }

    pub fn net_worth(&self) -> f64 {
        let prices: HashMap<i32, f64> = self
            .instruments
            .iter()
            .map(|s| (s.instrument.id, s.price))
            .collect();
        self.player.cash + self.portfolio.market_value(&prices) + self.game.assets_value(&prices)
    }

    /// Cash plus each asset class's total market value, as shares of
    /// net worth - the diversification snapshot. Always returns one
    /// slice per class (even if 0) so the view stays consistent
    /// whether or not the player holds anything in it yet.
    pub fn allocation(&self) -> Vec<AllocationSlice> {
        let mut by_class: HashMap<AssetClass, f64> = HashMap::new();
        for class in AssetClass::ALL {
            by_class.insert(class, 0.0);
        }

        for s in &self.instruments {
            if let Some(pos) = self.portfolio.positions.get(&s.instrument.id) {
                *by_class.entry(s.instrument.class).or_insert(0.0) += pos.quantity * s.price;
            }
        }

        let total = self.net_worth();
        let fraction_of = |value: f64| if total > 0.0 { (value / total).clamp(0.0, 1.0) } else { 0.0 };

        let mut slices = vec![AllocationSlice {
            label: "Cash".to_string(),
            value: self.player.cash,
            fraction: fraction_of(self.player.cash),
        }];

        // The bank account is not an investment class, so it gets its own slice.
        slices.push(AllocationSlice {
            label: "Bank savings".to_string(),
            value: self.game.savings,
            fraction: fraction_of(self.game.savings),
        });

        for class in AssetClass::ALL {
            if class == AssetClass::Savings {
                continue; // retired as a tradable class; see Bank savings above
            }
            let value = by_class[&class];
            slices.push(AllocationSlice {
                label: class.label().to_string(),
                value,
                fraction: fraction_of(value),
            });
        }

        let home = self.game.home_equity();
        slices.push(AllocationSlice {
            label: "Home equity".to_string(),
            value: home,
            fraction: fraction_of(home),
        });
        let owned = self.game.assets_equity();
        slices.push(AllocationSlice {
            label: "Assets (property, solar, car)".to_string(),
            value: owned,
            fraction: fraction_of(owned),
        });
        let businesses: f64 = self.game.businesses.iter().map(|b| b.value()).sum();
        slices.push(AllocationSlice {
            label: "Businesses".to_string(),
            value: businesses,
            fraction: fraction_of(businesses),
        });

        slices
    }

    /// Cash not tied up by resting buy orders.
    pub fn free_cash(&self) -> f64 {
        self.market.available_cash(&self.player)
    }

    pub fn symbol_of(&self, instrument_id: i32) -> String {
        self.instruments
            .iter()
            .find(|s| s.instrument.id == instrument_id)
            .map_or_else(|| format!("#{instrument_id}"), |s| s.instrument.symbol.clone())
    }

    pub fn record_fills(&mut self, new_fills: &[Fill]) {
        self.fills.extend_from_slice(new_fills);
        if self.fills.len() > FILLS_LEN {
            let excess = self.fills.len() - FILLS_LEN;
            self.fills.drain(0..excess);
        }
    }

    /// Jumps to a tab, showing its first view.
    pub fn set_tab(&mut self, tab: Tab) {
        self.tab = tab;
        self.chart_mode = tab.views()[0];
    }

    /// Cycles the current tab's views (`w`).
    pub fn next_view(&mut self) {
        let views = self.tab.views();
        let i = views.iter().position(|v| *v == self.chart_mode).unwrap_or(0);
        self.chart_mode = views[(i + 1) % views.len()];
    }

    pub fn select_next(&mut self) {
        if !self.instruments.is_empty() {
            self.selected = (self.selected + 1) % self.instruments.len();
        }
    }

    /// Opens the manual at the entry that explains what you're looking at.
    pub fn open_manual(&mut self) {
        let term = match (self.tab, self.chart_mode) {
            (Tab::Manual, _) => return,
            (Tab::Market, ChartMode::OrderBook) => "Bid, ask and spread",
            (Tab::Market, _) => self
                .selected_instrument()
                .map(|s| crate::manual::term_for_class(s.instrument.class))
                .unwrap_or("Stocks (equities)"),
            (Tab::Portfolio, ChartMode::Allocation) => "Diversification",
            (Tab::Portfolio, _) => "Net worth",
            (Tab::Life, ChartMode::Report) => "Cash flow",
            (Tab::Life, _) => "Financial independence",
            (Tab::Deals, ChartMode::Money) => "Mortgage",
            (Tab::Deals, _) => "Business valuation and profit",
            (Tab::World, _) => "Inflation and CPI",
            (Tab::Logs, _) => "Cash flow",
        };
        self.prev_tab = self.tab;
        self.manual_selected = crate::manual::find(term).unwrap_or(0);
        self.set_tab(Tab::Manual);
    }

    pub fn manual_move(&mut self, delta: isize) {
        let n = crate::manual::ENTRIES.len() as isize;
        self.manual_selected = (self.manual_selected as isize + delta).rem_euclid(n) as usize;
    }

    /// Scrolls the Logs view (positive = further back in time).
    pub fn scroll_log(&mut self, delta: isize) {
        let max = match self.chart_mode {
            ChartMode::Transactions => self.transactions.len(),
            ChartMode::News => self.game.news.len(),
            _ => self.fills.len(),
        };
        let next = (self.log_scroll as isize + delta).clamp(0, max.saturating_sub(1) as isize);
        self.log_scroll = next as usize;
    }

    pub fn select_prev(&mut self) {
        if !self.instruments.is_empty() {
            self.selected = if self.selected == 0 {
                self.instruments.len() - 1
            } else {
                self.selected - 1
            };
        }
    }
}
