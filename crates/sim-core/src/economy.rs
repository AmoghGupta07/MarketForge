//! The macro economy, simulated once a month: inflation and a price
//! level (CPI), GDP growth, unemployment, a central bank that sets the
//! policy rate, commodity supply and demand, and a government that taxes
//! and runs a budget. Pure logic like the rest of sim-core.
//!
//! The pieces feed each other: growth (driven by boom/recession events)
//! moves unemployment and inflation; inflation moves the central bank's
//! rate (a Taylor-style rule), which moves bond and property prices and
//! loan costs; growth also drives commodity demand, whose imbalance with
//! supply pushes commodity prices; inflation erodes your purchasing
//! power through your expenses; and the government raises or cuts taxes
//! depending on its debt.

use rand::Rng;
use rand_distr::{Distribution, Normal};
use serde::{Deserialize, Serialize};

pub const INFLATION_TARGET: f64 = 0.025;
pub const TREND_GROWTH: f64 = 0.025;
/// Real policy rate when the economy is at target (so the neutral
/// nominal rate is 4%, matching the sim's long-run rate).
pub const NEUTRAL_REAL_RATE: f64 = 0.015;
/// Months kept in the in-memory indicator history.
pub const HISTORY_MONTHS: usize = 240;

// ------------------------------------------------------------------ tax

/// Annual income brackets: (upper bound, marginal rate).
pub const BRACKETS: &[(f64, f64)] = &[
    (24_000.0, 0.00),
    (60_000.0, 0.10),
    (120_000.0, 0.20),
    (250_000.0, 0.30),
    (f64::INFINITY, 0.38),
];
pub const CAPITAL_GAINS_RATE: f64 = 0.15;
/// Dividends, coupons, rent and bank interest.
pub const INVESTMENT_INCOME_RATE: f64 = 0.15;
pub const BUSINESS_PROFIT_RATE: f64 = 0.20;

/// Progressive tax on an annual income. `mult` scales every rate (the
/// government's current tax stance; 1.0 = standard).
pub fn annual_income_tax(income: f64, mult: f64) -> f64 {
    let mut tax = 0.0;
    let mut lower = 0.0;
    for (upper, rate) in BRACKETS {
        if income > lower {
            tax += (income.min(*upper) - lower) * rate;
        }
        lower = *upper;
    }
    tax * mult
}

/// Tax withheld from one month's gross salary.
pub fn monthly_withholding(monthly_gross: f64, mult: f64) -> f64 {
    annual_income_tax(monthly_gross * 12.0, mult) / 12.0
}

// --------------------------------------------------------------- state

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonthPoint {
    #[serde(default)]
    pub home_price_index: f64,
    pub month: i64,
    pub tick: i64,
    pub inflation: f64,
    pub cpi: f64,
    pub gdp_growth: f64,
    pub unemployment: f64,
    pub policy_rate: f64,
    pub debt_to_gdp: f64,
}

/// Supply and demand for one commodity. 1.0 means "normal"; the gap
/// between demand and supply pushes the price.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommodityMarket {
    pub symbol: String,
    pub supply: f64,
    pub demand: f64,
}

impl CommodityMarket {
    /// Positive = shortage (price pressure up), negative = glut.
    pub fn imbalance(&self) -> f64 {
        self.demand - self.supply
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Government {
    /// Scales every tax rate. Moves with policy and the budget.
    pub tax_mult: f64,
    /// Public debt as a share of GDP.
    pub debt_to_gdp: f64,
    pub revenue: f64,
    pub spending: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Economy {
    /// Annual inflation rate (the latest monthly reading, annualized).
    pub inflation: f64,
    /// Price level; 100 at the start of the game.
    pub cpi: f64,
    pub gdp_growth: f64,
    pub unemployment: f64,
    pub govt: Government,
    pub commodities: Vec<CommodityMarket>,
    pub history: Vec<MonthPoint>,
    /// House prices; 100 at the start. Follows inflation and growth, and
    /// falls when interest rates rise.
    #[serde(default = "default_hpi")]
    pub home_price_index: f64,
}

fn default_hpi() -> f64 {
    100.0
}

fn normal(rng: &mut impl Rng, sd: f64) -> f64 {
    sd * Normal::new(0.0, 1.0).expect("valid normal").sample(rng)
}

impl Economy {
    pub fn new() -> Economy {
        Economy {
            inflation: INFLATION_TARGET,
            cpi: 100.0,
            gdp_growth: TREND_GROWTH,
            unemployment: 0.05,
            govt: Government { tax_mult: 1.0, debt_to_gdp: 0.80, revenue: 0.0, spending: 0.0 },
            commodities: ["WHET", "OIL", "STEE"]
                .iter()
                .map(|s| CommodityMarket { symbol: s.to_string(), supply: 1.0, demand: 1.0 })
                .collect(),
            history: vec![],
            home_price_index: 100.0,
        }
    }

    pub fn commodity(&self, symbol: &str) -> Option<&CommodityMarket> {
        self.commodities.iter().find(|c| c.symbol == symbol)
    }

    /// Annualized drift added to a commodity's price by its supply/demand
    /// gap. Each point of imbalance is worth 100% a year, capped.
    pub fn price_pressure(&self, symbol: &str) -> f64 {
        self.commodity(symbol).map_or(0.0, |c| (c.imbalance() * 1.0).clamp(-0.5, 0.5))
    }

    /// Supply disruption (negative) or windfall (positive).
    pub fn shock_supply(&mut self, symbol: &str, delta: f64) {
        if let Some(c) = self.commodities.iter_mut().find(|c| c.symbol == symbol) {
            c.supply = (c.supply + delta).clamp(0.5, 1.6);
        }
    }

    /// Macro side effects of named events (price jumps live in the event
    /// catalog; these move the real economy).
    pub fn apply_event(&mut self, key: &str) {
        match key {
            "oil_spike" => self.shock_supply("OIL", -0.20),
            "oil_glut" => self.shock_supply("OIL", 0.20),
            "harvest_failure" => self.shock_supply("WHET", -0.22),
            "inflation_spike" => self.inflation = (self.inflation + 0.03).min(0.15),
            "stimulus" => self.gdp_growth += 0.012,
            "recession" => {
                self.gdp_growth -= 0.02;
                self.home_price_index *= 0.97;
            }
            "housing_boom" => self.home_price_index *= 1.05,
            "boom" => self.gdp_growth += 0.015,
            _ => {}
        }
    }

    /// One month. `econ_factor` is the combined effect of active
    /// boom/recession events; `price_ratio` gives a commodity's price
    /// relative to its base (high prices coax out more supply).
    /// Returns the central bank's new policy rate.
    pub fn monthly<R: Rng>(
        &mut self,
        month: i64,
        tick: i64,
        econ_factor: f64,
        policy_rate: f64,
        price_ratio: impl Fn(&str) -> f64,
        rng: &mut R,
    ) -> (f64, Vec<String>) {
        let mut news = vec![];

        // Growth: pulled toward trend, shifted by boom/recession events.
        let target = TREND_GROWTH + (econ_factor - 1.0) * 0.10;
        self.gdp_growth += 0.4 * (target - self.gdp_growth) + normal(rng, 0.003);
        self.gdp_growth = self.gdp_growth.clamp(-0.08, 0.10);

        // Unemployment follows growth (a rough Okun's law).
        let u_target = 0.05 - 0.8 * (self.gdp_growth - TREND_GROWTH);
        self.unemployment += 0.2 * (u_target - self.unemployment) + normal(rng, 0.001);
        self.unemployment = self.unemployment.clamp(0.02, 0.15);

        // Commodity markets: demand tracks growth, supply recovers and
        // responds to price.
        for c in &mut self.commodities {
            let d_target = 1.0 + 2.0 * (self.gdp_growth - TREND_GROWTH);
            c.demand += 0.3 * (d_target - c.demand) + normal(rng, 0.01);
            let ratio = price_ratio(&c.symbol);
            c.supply += 0.15 * (1.0 - c.supply) + 0.05 * (ratio - 1.0).clamp(-1.0, 1.0) + normal(rng, 0.01);
            c.demand = c.demand.clamp(0.6, 1.5);
            c.supply = c.supply.clamp(0.5, 1.6);
        }
        let oil = self.commodity("OIL").map_or(0.0, |c| c.imbalance());

        // Inflation: demand pressure, energy costs, mean reversion.
        let pi_target = INFLATION_TARGET + 0.6 * (self.gdp_growth - TREND_GROWTH) + 0.03 * oil;
        self.inflation += 0.15 * (pi_target - self.inflation) + normal(rng, 0.002);
        self.inflation = self.inflation.clamp(-0.02, 0.15);
        self.cpi *= 1.0 + self.inflation / 12.0;

        // Central bank: a Taylor-style rule, moved a quarter of the way
        // each month.
        let rule = NEUTRAL_REAL_RATE
            + self.inflation
            + 0.5 * (self.inflation - INFLATION_TARGET)
            + 0.4 * (self.gdp_growth - TREND_GROWTH);
        let new_rate = policy_rate + 0.25 * (rule - policy_rate);

        // House prices: inflation and growth lift them, high rates drag them down.
        let hpi_growth = self.inflation + 0.6 * (self.gdp_growth - TREND_GROWTH) - 1.0 * (new_rate - 0.04) + normal(rng, 0.012);
        self.home_price_index = (self.home_price_index * (1.0 + hpi_growth / 12.0)).max(30.0);

        // Government budget: recessions raise spending and cut revenue.
        let revenue = 0.19 * self.govt.tax_mult * (1.0 + (self.gdp_growth - TREND_GROWTH) * 2.0);
        let spending = 0.205 + 0.6 * (self.unemployment - 0.05);
        self.govt.revenue = revenue;
        self.govt.spending = spending;
        self.govt.debt_to_gdp = (self.govt.debt_to_gdp + (spending - revenue) / 12.0).clamp(0.2, 2.5);
        if month % 12 == 0 {
            if self.govt.debt_to_gdp > 1.0 && self.govt.tax_mult < 1.3 {
                self.govt.tax_mult += 0.05;
                news.push("The government raises taxes to bring the deficit under control.".to_string());
            } else if self.govt.debt_to_gdp < 0.7 && self.govt.tax_mult > 0.8 {
                self.govt.tax_mult -= 0.05;
                news.push("With the budget in good shape, the government cuts taxes.".to_string());
            }
        }

        self.history.push(MonthPoint {
            home_price_index: self.home_price_index,
            month,
            tick,
            inflation: self.inflation,
            cpi: self.cpi,
            gdp_growth: self.gdp_growth,
            unemployment: self.unemployment,
            policy_rate: new_rate,
            debt_to_gdp: self.govt.debt_to_gdp,
        });
        if self.history.len() > HISTORY_MONTHS {
            let excess = self.history.len() - HISTORY_MONTHS;
            self.history.drain(0..excess);
        }
        (new_rate, news)
    }

    /// Prices today vs `months` ago, as a rate (e.g. 0.031 = 3.1%).
    pub fn inflation_over(&self, months: usize) -> Option<f64> {
        if self.history.len() < months {
            return None;
        }
        // The price level started at 100, so a full run of `months` readings
        // compares against that baseline.
        let then = if self.history.len() == months { 100.0 } else { self.history[self.history.len() - 1 - months].cpi };
        Some(self.cpi / then - 1.0)
    }
}

impl Default for Economy {
    fn default() -> Self {
        Economy::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    fn run(econ: &mut Economy, months: i64, factor: f64, rng: &mut StdRng) -> f64 {
        let mut rate = 0.04;
        for m in 1..=months {
            let (r, _) = econ.monthly(m, m * 30, factor, rate, |_| 1.0, rng);
            rate = r;
        }
        rate
    }

    #[test]
    fn tax_is_progressive_and_scales() {
        assert_eq!(annual_income_tax(20_000.0, 1.0), 0.0);
        assert!((annual_income_tax(48_000.0, 1.0) - 2_400.0).abs() < 1e-9);
        assert!((annual_income_tax(100_000.0, 1.0) - (3_600.0 + 8_000.0)).abs() < 1e-9);
        let (a, b) = (annual_income_tax(80_000.0, 1.0), annual_income_tax(160_000.0, 1.0));
        assert!(b / 160_000.0 > a / 80_000.0, "effective rate rises with income");
        assert!((annual_income_tax(48_000.0, 1.2) - 2_880.0).abs() < 1e-9);
        assert!((monthly_withholding(4_000.0, 1.0) - 200.0).abs() < 1e-9);
    }

    #[test]
    fn a_calm_economy_stays_near_target() {
        let mut e = Economy::new();
        let mut rng = StdRng::seed_from_u64(1);
        let rate = run(&mut e, 240, 1.0, &mut rng);
        assert!((e.inflation - INFLATION_TARGET).abs() < 0.03, "{}", e.inflation);
        assert!(e.unemployment > 0.03 && e.unemployment < 0.08, "{}", e.unemployment);
        assert!(rate > 0.02 && rate < 0.07, "{rate}");
        assert!(e.cpi > 100.0);
        assert_eq!(e.history.len(), 240);
    }

    #[test]
    fn recessions_raise_unemployment_and_cut_rates() {
        let mut calm = Economy::new();
        let mut slump = Economy::new();
        let mut rng = StdRng::seed_from_u64(2);
        let r_calm = run(&mut calm, 24, 1.0, &mut rng);
        let r_slump = run(&mut slump, 24, 0.65, &mut rng);
        assert!(slump.unemployment > calm.unemployment + 0.01);
        assert!(slump.gdp_growth < calm.gdp_growth);
        assert!(r_slump < r_calm, "central bank eases in a slump: {r_slump} vs {r_calm}");
    }

    #[test]
    fn inflation_makes_the_central_bank_tighten() {
        let mut e = Economy::new();
        e.inflation = 0.08;
        let (rate, _) = e.monthly(1, 30, 1.0, 0.04, |_| 1.0, &mut StdRng::seed_from_u64(3));
        assert!(rate > 0.04, "{rate}");
    }

    #[test]
    fn shortages_push_commodity_prices_up_and_gluts_down() {
        let mut e = Economy::new();
        assert_eq!(e.price_pressure("OIL"), 0.0);
        e.apply_event("oil_spike");
        assert!(e.price_pressure("OIL") > 0.15);
        e.apply_event("oil_glut");
        e.apply_event("oil_glut");
        assert!(e.price_pressure("OIL") < 0.0);
        assert_eq!(e.price_pressure("NOPE"), 0.0);
    }

    #[test]
    fn high_prices_coax_out_supply_over_time() {
        let mut e = Economy::new();
        e.shock_supply("OIL", -0.3);
        let mut rng = StdRng::seed_from_u64(4);
        let before = e.commodity("OIL").unwrap().imbalance();
        for m in 1..=12 {
            e.monthly(m, m * 30, 1.0, 0.04, |_| 1.5, &mut rng);
        }
        assert!(e.commodity("OIL").unwrap().imbalance() < before);
    }

    #[test]
    fn inflation_event_and_cpi_compounding() {
        let mut e = Economy::new();
        e.apply_event("inflation_spike");
        assert!(e.inflation > 0.05);
        let mut rng = StdRng::seed_from_u64(5);
        for m in 1..=13 {
            e.monthly(m, m * 30, 1.0, 0.04, |_| 1.0, &mut rng);
        }
        let yoy = e.inflation_over(12).unwrap();
        assert!(yoy > 0.02, "{yoy}");
        assert!(Economy::new().inflation_over(12).is_none());
    }

    #[test]
    fn house_prices_fall_with_high_rates_and_rise_with_inflation() {
        let (mut hot, mut cold) = (Economy::new(), Economy::new());
        for m in 1..=36 {
            hot.monthly(m, m * 30, 1.0, 0.10, |_| 1.0, &mut StdRng::seed_from_u64(m as u64));
            cold.monthly(m, m * 30, 1.0, 0.01, |_| 1.0, &mut StdRng::seed_from_u64(m as u64));
        }
        assert!(cold.home_price_index > hot.home_price_index * 1.1, "{} vs {}", cold.home_price_index, hot.home_price_index);
        let mut e = Economy::new();
        e.apply_event("housing_boom");
        assert!((e.home_price_index - 105.0).abs() < 1e-9);
    }

    #[test]
    fn government_adjusts_taxes_to_its_debt() {
        let mut e = Economy::new();
        e.govt.debt_to_gdp = 1.4;
        let (_, news) = e.monthly(12, 360, 1.0, 0.04, |_| 1.0, &mut StdRng::seed_from_u64(6));
        assert!(e.govt.tax_mult > 1.0);
        assert!(news.iter().any(|n| n.contains("raises taxes")));
        let mut e = Economy::new();
        e.govt.debt_to_gdp = 0.4;
        e.monthly(12, 360, 1.0, 0.04, |_| 1.0, &mut StdRng::seed_from_u64(6));
        assert!(e.govt.tax_mult < 1.0);
    }
}
