use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AssetClass {
    Equity,
    IndexFund,
    /// Basket-derived fund: its price (NAV) is computed from its
    /// constituents every tick, not simulated on its own.
    MutualFund,
    Savings,
    Commodity,
    Bond,
    RealEstate,
    Forex,
    Crypto,
}

impl AssetClass {
    /// Display / allocation order.
    pub const ALL: [AssetClass; 9] = [
        AssetClass::Equity,
        AssetClass::IndexFund,
        AssetClass::MutualFund,
        AssetClass::Savings,
        AssetClass::Commodity,
        AssetClass::Bond,
        AssetClass::RealEstate,
        AssetClass::Forex,
        AssetClass::Crypto,
    ];

    /// The string stored in `instruments.class`.
    pub fn db_str(self) -> &'static str {
        match self {
            AssetClass::Equity => "equity",
            AssetClass::IndexFund => "index_fund",
            AssetClass::MutualFund => "mutual_fund",
            AssetClass::Savings => "savings",
            AssetClass::Commodity => "commodity",
            AssetClass::Bond => "bond",
            AssetClass::RealEstate => "real_estate",
            AssetClass::Forex => "forex",
            AssetClass::Crypto => "crypto",
        }
    }

    /// Unknown strings fall back to Equity (same as the old CLI-side match).
    pub fn from_db_str(s: &str) -> AssetClass {
        match s {
            "index_fund" => AssetClass::IndexFund,
            "mutual_fund" => AssetClass::MutualFund,
            "savings" => AssetClass::Savings,
            "commodity" => AssetClass::Commodity,
            "bond" => AssetClass::Bond,
            "real_estate" => AssetClass::RealEstate,
            "forex" => AssetClass::Forex,
            "crypto" => AssetClass::Crypto,
            _ => AssetClass::Equity,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AssetClass::Equity => "Equity",
            AssetClass::IndexFund => "Index Fund",
            AssetClass::MutualFund => "Mutual Fund",
            AssetClass::Savings => "Savings",
            AssetClass::Commodity => "Commodity",
            AssetClass::Bond => "Bond",
            AssetClass::RealEstate => "Real Estate",
            AssetClass::Forex => "Forex",
            AssetClass::Crypto => "Crypto",
        }
    }

    /// Compact label for narrow table columns.
    pub fn short_label(self) -> &'static str {
        match self {
            AssetClass::Equity => "Equity",
            AssetClass::IndexFund => "Index",
            AssetClass::MutualFund => "Fund",
            AssetClass::Savings => "Savings",
            AssetClass::Commodity => "Cmdty",
            AssetClass::Bond => "Bond",
            AssetClass::RealEstate => "RealEst",
            AssetClass::Forex => "Forex",
            AssetClass::Crypto => "Crypto",
        }
    }
}

/// One holding inside a mutual fund's basket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Constituent {
    pub instrument_id: i32,
    /// Share of the fund held in this instrument (weights are
    /// effectively rebalanced every tick, like an index tracker).
    pub weight: f64,
}

/// Static definition of a tradable instrument. Does not carry a live
/// price - see `InstrumentState`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instrument {
    pub id: i32,
    pub symbol: String,
    pub name: String,
    pub class: AssetClass,
    /// Starting price of a new save. Forex also uses it as the
    /// long-run level the pair mean-reverts toward.
    pub base_price: f64,
    /// Annualized expected return, e.g. 0.08 = 8%/yr. Used by the
    /// GBM-priced classes (equity, index, savings, commodity, real
    /// estate, crypto).
    pub drift: f64,
    /// Annualized volatility, e.g. 0.25 = 25%/yr. Used by every
    /// stochastic class (GBM ones and Forex).
    pub volatility: f64,
    /// Bond-only: par value paid back per unit. 0 otherwise.
    pub face_value: f64,
    /// Bond-only: fixed annual coupon rate. 0 otherwise.
    pub coupon_rate: f64,
    /// Bond-only: interest-rate sensitivity (~years to maturity).
    pub duration: f64,
    /// Annual income paid as a fraction of price (e.g. rent from a
    /// property trust), paid on payday. 0 for instruments with no
    /// income stream. (Bonds use face_value/coupon_rate instead.)
    pub income_yield: f64,
    /// How much GBM drift falls per unit the prevailing interest rate
    /// sits above its long-run level: drift_eff = drift -
    /// rate_sensitivity * (rate - long_run). Real estate is the main
    /// user (property is rate-sensitive). 0 = unaffected.
    pub rate_sensitivity: f64,
    /// MutualFund-only: annual fee, deducted continuously from NAV.
    pub expense_ratio: f64,
    /// MutualFund-only: the basket the NAV is derived from.
    pub constituents: Vec<Constituent>,
}

/// An instrument plus its current price - what the engine actually
/// operates on each tick.
#[derive(Debug, Clone)]
pub struct InstrumentState {
    pub instrument: Instrument,
    pub price: f64,
}
