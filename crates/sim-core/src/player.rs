use serde::{Deserialize, Serialize};

/// How many ticks make up one pay period. 1 tick = 1 simulated day (see
/// engine::TICK_DT_YEARS), so 30 ticks = roughly one simulated month -
/// salary and expenses are monthly figures, so they should only apply
/// once per 30 ticks, not every single tick.
pub const PAY_PERIOD_TICKS: i64 = 30;

/// A game mode is just a different configuration of the same engine:
/// which cashflow rules apply, and (at the CLI layer) which starting
/// instrument set gets seeded. No separate simulation per mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameMode {
    SalariedInvestor,
    Trader,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Player {
    pub id: i32,
    pub game_mode: GameMode,
    pub cash: f64,
    pub salary: f64,
    pub expenses: f64,
    pub current_tick: i64,
    /// This save's prevailing interest rate (annualized, e.g. 0.04 =
    /// 4%). Drifts each tick (see engine::fast_forward) and drives
    /// every bond's price and coupon income - a shared macro variable
    /// for this save's economy, same as cash is this save's balance.
    pub interest_rate: f64,
}

impl Player {
    /// Applies salary/expenses on payday ticks only (see
    /// PAY_PERIOD_TICKS) - not every tick. No-op for Trader mode,
    /// which has no income cycle at all. Assumes `current_tick` has
    /// already been incremented for this tick before this is called
    /// (see engine::fast_forward), so payday lands on tick 30, 60, 90,
    /// etc. rather than immediately at creation (tick 0).
    pub fn apply_tick_cashflow(&mut self) {
        if let GameMode::SalariedInvestor = self.game_mode {
            if self.current_tick % PAY_PERIOD_TICKS == 0 {
                self.cash += self.salary;
                self.cash -= self.expenses;
            }
        }
    }
}
