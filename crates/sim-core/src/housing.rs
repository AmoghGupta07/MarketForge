//! Rent vs buy. You start out renting (about a third of your expenses is
//! rent). Buying needs a down payment and closing costs, a mortgage your
//! income can support, and a seller you negotiate with; in exchange your
//! rent turns into mortgage payments plus upkeep, and you own an asset
//! that follows the home price index.

use serde::{Deserialize, Serialize};

pub const MORTGAGE_MONTHS: i32 = 360;
/// Mortgage rate = policy rate + this.
pub const MORTGAGE_SPREAD: f64 = 0.025;
pub const DOWN_PAYMENT: f64 = 0.20;
pub const CLOSING_COST: f64 = 0.03;
pub const SELLING_COST: f64 = 0.06;
/// Property tax and maintenance, as a yearly share of the home's value.
pub const UPKEEP_RATE: f64 = 0.02;
/// The bank approves a mortgage whose payment is at most this share of
/// your gross monthly salary.
pub const MAX_PAYMENT_TO_INCOME: f64 = 0.40;
/// Share of your starting monthly expenses that is rent.
pub const RENT_SHARE: f64 = 0.35;
/// Share of value a forced sale (to cover bills) fetches.
pub const FORCED_SALE_FACTOR: f64 = 0.85;
pub const HOME_LISTING_LIFETIME_TICKS: i64 = 180;
/// Index into the NPC cast.
pub const NPC_REALTOR: usize = 4;

/// (name, price at a home price index of 100).
pub const HOME_KINDS: &[(&str, f64)] = &[
    ("Starter Studio", 110_000.0),
    ("Townhouse", 185_000.0),
    ("Family House", 280_000.0),
    ("Villa", 520_000.0),
];

/// Fixed-rate monthly payment on a 30-year mortgage.
pub fn mortgage_payment(principal: f64, annual_rate: f64) -> f64 {
    let r = annual_rate / 12.0;
    if r.abs() < 1e-9 {
        principal / MORTGAGE_MONTHS as f64
    } else {
        principal * r / (1.0 - (1.0 + r).powi(-MORTGAGE_MONTHS))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mortgage {
    pub principal: f64,
    pub annual_rate: f64,
    pub payment: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Home {
    pub name: String,
    pub bought_tick: i64,
    pub price_paid: f64,
    pub hpi_at_purchase: f64,
    pub mortgage: Mortgage,
}

impl Home {
    /// Market value: what you paid, moved by the home price index.
    pub fn value(&self, hpi: f64) -> f64 {
        self.price_paid * hpi / self.hpi_at_purchase
    }

    pub fn equity(&self, hpi: f64) -> f64 {
        self.value(hpi) - self.mortgage.principal
    }

    pub fn monthly_upkeep(&self, hpi: f64) -> f64 {
        self.value(hpi) * UPKEEP_RATE / 12.0
    }

    /// Mortgage payment plus upkeep: what owning costs each month.
    pub fn monthly_cost(&self, hpi: f64) -> f64 {
        self.mortgage.payment + self.monthly_upkeep(hpi)
    }
}

/// A home for sale.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HomeListing {
    pub id: u64,
    pub kind: usize,
    pub price: f64,
    pub expires_tick: i64,
}

impl HomeListing {
    pub fn name(&self) -> &'static str {
        HOME_KINDS[self.kind.min(HOME_KINDS.len() - 1)].0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mortgage_payment_matches_the_textbook() {
        // 200k at 6% over 30 years is about 1,199.10 a month.
        assert!((mortgage_payment(200_000.0, 0.06) - 1_199.10).abs() < 0.01);
        assert!((mortgage_payment(360_000.0, 0.0) - 1_000.0).abs() < 1e-9);
        assert!(mortgage_payment(200_000.0, 0.08) > mortgage_payment(200_000.0, 0.04));
    }

    #[test]
    fn a_full_term_of_payments_repays_the_loan() {
        let (mut p, rate) = (150_000.0, 0.05);
        let pmt = mortgage_payment(p, rate);
        for _ in 0..MORTGAGE_MONTHS {
            p -= pmt - p * rate / 12.0;
        }
        assert!(p.abs() < 1.0, "{p}");
    }

    #[test]
    fn value_and_equity_follow_the_index() {
        let h = Home {
            name: "x".into(),
            bought_tick: 0,
            price_paid: 200_000.0,
            hpi_at_purchase: 100.0,
            mortgage: Mortgage { principal: 160_000.0, annual_rate: 0.06, payment: 959.0 },
        };
        assert_eq!(h.value(100.0), 200_000.0);
        assert_eq!(h.value(120.0), 240_000.0);
        assert_eq!(h.equity(120.0), 80_000.0);
        assert!(h.equity(70.0) < 0.0, "underwater when prices fall");
        assert!((h.monthly_upkeep(100.0) - 200_000.0 * 0.02 / 12.0).abs() < 1e-9);
    }
}
