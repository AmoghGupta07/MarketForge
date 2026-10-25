//! The NPC cast: deal partners with relationship scores, and rival
//! investors you are measured against.

use rand::Rng;
use rand_distr::{Distribution, Normal};
use serde::{Deserialize, Serialize};

pub const NPC_BROKER: usize = 0;
pub const NPC_BOSS: usize = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Npc {
    pub name: String,
    pub role: String,
    /// 0-100. Better relationships mean better prices and more patience.
    pub relationship: f64,
}

impl Npc {
    pub fn adjust(&mut self, delta: f64) {
        self.relationship = (self.relationship + delta).clamp(0.0, 100.0);
    }

    pub fn mood(&self) -> &'static str {
        match self.relationship {
            r if r >= 80.0 => "close ally",
            r if r >= 60.0 => "friendly",
            r if r >= 40.0 => "neutral",
            r if r >= 20.0 => "cold",
            _ => "hostile",
        }
    }
}

/// Index order matters: see NPC_* constants and BusinessKind::owner_npc.
pub fn default_cast() -> Vec<Npc> {
    let mk = |name: &str, role: &str| Npc { name: name.to_string(), role: role.to_string(), relationship: 50.0 };
    vec![
        mk("Marcus Chen", "Broker"),
        mk("Diane Walsh", "Your boss"),
        mk("Rosa Alvarez", "Cafe & laundry owner"),
        mk("Tom Briggs", "Logistics & e-commerce"),
        mk("Helen Okafor", "Real estate agent"),
    ]
}

/// A competing investor. Their wealth follows the market (via `beta`)
/// plus their own skill/luck, and they save a bit every month.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rival {
    pub name: String,
    pub style: String,
    pub wealth: f64,
    /// Sensitivity to the broad-market index return.
    pub beta: f64,
    pub alpha_drift: f64,
    pub alpha_vol: f64,
    pub monthly_saving: f64,
}

pub fn default_rivals(player_net_worth: f64) -> Vec<Rival> {
    let base = player_net_worth.max(2_000.0);
    vec![
        Rival { name: "Elena Rossi".into(), style: "steady".into(), wealth: base * 1.10, beta: 0.8, alpha_drift: 0.030, alpha_vol: 0.04, monthly_saving: 1_150.0 },
        Rival { name: "Victor Hale".into(), style: "aggressive".into(), wealth: base * 0.95, beta: 1.6, alpha_drift: 0.040, alpha_vol: 0.18, monthly_saving: 950.0 },
        Rival { name: "Priya Nair".into(), style: "conservative".into(), wealth: base * 1.25, beta: 0.4, alpha_drift: 0.020, alpha_vol: 0.02, monthly_saving: 1_250.0 },
    ]
}

impl Rival {
    /// One tick: market-correlated return plus idiosyncratic noise.
    pub fn step<R: Rng>(&mut self, market_return: f64, dt: f64, rng: &mut R) {
        let z: f64 = Normal::new(0.0, 1.0).expect("valid normal").sample(rng);
        let r = self.beta * market_return + self.alpha_drift * dt + self.alpha_vol * dt.sqrt() * z;
        self.wealth = (self.wealth * (1.0 + r)).max(100.0);
    }

    pub fn payday(&mut self) {
        self.wealth += self.monthly_saving;
    }

    /// Savings grow with the cost of living each year.
    pub fn annual_raise(&mut self) {
        self.monthly_saving *= 1.03;
    }
}
