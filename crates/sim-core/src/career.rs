//! Career ladder and side businesses.

use serde::{Deserialize, Serialize};

/// (title, monthly salary). The first rung matches the starting salary.
pub const LADDER: &[(&str, f64)] = &[
    ("Junior Analyst", 4_200.0),
    ("Analyst", 5_150.0),
    ("Senior Analyst", 6_400.0),
    ("Associate Director", 8_200.0),
    ("Director", 11_000.0),
    ("Vice President", 14_700.0),
];

/// Ticks between annual reviews (one simulated year).
pub const REVIEW_PERIOD_TICKS: i64 = 360;
/// Share of salary paid as unemployment benefit while between jobs.
pub const BENEFIT_RATE: f64 = 0.50;
pub const MAX_SKILL: f64 = 10.0;
pub const TRAINING_COST: f64 = 600.0;
pub const TRAINING_COOLDOWN_TICKS: i64 = 90;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Career {
    pub title_idx: usize,
    /// 0-10. Raises promotion odds and your bargaining power at reviews.
    pub skill: f64,
    pub employed: bool,
    pub rehire_tick: Option<i64>,
    /// Actively searching: rehire arrives twice as fast.
    pub job_hunting: bool,
    pub next_review_tick: i64,
    pub last_training_tick: i64,
}

impl Career {
    /// Starts on the rung whose salary best matches `salary`.
    pub fn new(salary: f64, tick: i64) -> Career {
        let title_idx = LADDER.iter().rposition(|(_, s)| *s <= salary + 1.0).unwrap_or(0);
        Career {
            title_idx,
            skill: 0.0,
            employed: true,
            rehire_tick: None,
            job_hunting: false,
            next_review_tick: tick + REVIEW_PERIOD_TICKS,
            last_training_tick: -TRAINING_COOLDOWN_TICKS,
        }
    }

    pub fn title(&self) -> &'static str {
        LADDER[self.title_idx.min(LADDER.len() - 1)].0
    }

    pub fn promotion_chance(&self) -> f64 {
        if self.title_idx + 1 >= LADDER.len() {
            return 0.0;
        }
        (0.25 + 0.08 * self.skill).clamp(0.0, 0.90)
    }

    pub fn next_title(&self) -> Option<(&'static str, f64)> {
        LADDER.get(self.title_idx + 1).copied()
    }
}

/// Static description of a kind of business.
pub struct BusinessKind {
    pub name: &'static str,
    /// Typical monthly profit at normal efficiency.
    pub base_profit: f64,
    /// Month-to-month profit noise (fraction of profit).
    pub volatility: f64,
    /// Fair value = monthly profit x this many months.
    pub multiple: f64,
    /// Which NPC owns/buys this kind of business (index into the cast).
    pub owner_npc: usize,
}

pub const BUSINESS_KINDS: &[BusinessKind] = &[
    BusinessKind { name: "Food Cart", base_profit: 320.0, volatility: 0.45, multiple: 18.0, owner_npc: 2 },
    BusinessKind { name: "Corner Cafe", base_profit: 900.0, volatility: 0.35, multiple: 22.0, owner_npc: 2 },
    BusinessKind { name: "Laundromat", base_profit: 650.0, volatility: 0.15, multiple: 24.0, owner_npc: 2 },
    BusinessKind { name: "Delivery Van Route", base_profit: 1_100.0, volatility: 0.30, multiple: 20.0, owner_npc: 3 },
    BusinessKind { name: "Online Gift Shop", base_profit: 500.0, volatility: 0.50, multiple: 18.0, owner_npc: 3 },
];

pub const MAX_EFFICIENCY: f64 = 1.6;
pub const UPGRADE_EFFICIENCY_GAIN: f64 = 0.08;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Business {
    pub id: u64,
    pub kind: usize,
    pub name: String,
    /// Typical monthly profit at efficiency 1.0.
    pub monthly_profit: f64,
    pub efficiency: f64,
    pub bought_tick: i64,
    pub price_paid: f64,
}

impl Business {
    pub fn kind(&self) -> &'static BusinessKind {
        &BUSINESS_KINDS[self.kind.min(BUSINESS_KINDS.len() - 1)]
    }

    /// What the business is worth on paper (and in net worth).
    pub fn value(&self) -> f64 {
        self.monthly_profit * self.efficiency * self.kind().multiple
    }

    pub fn upgrade_cost(&self) -> f64 {
        3.0 * self.monthly_profit
    }
}

/// A business for sale. Expires if you don't act on it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Listing {
    pub id: u64,
    pub kind: usize,
    pub monthly_profit: f64,
    pub expires_tick: i64,
}

impl Listing {
    pub fn kind(&self) -> &'static BusinessKind {
        &BUSINESS_KINDS[self.kind.min(BUSINESS_KINDS.len() - 1)]
    }

    pub fn fair_price(&self) -> f64 {
        self.monthly_profit * self.kind().multiple
    }
}
