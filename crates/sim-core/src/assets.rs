//! Assets beyond your own home: buy-to-let property, storage units, solar
//! panels and a car. Two families:
//!
//! * **Investment property** (rental flat, storage units) follows the home
//!   price index, pays rent every payday (less vacancies), costs upkeep, and
//!   can be bought with a buy-to-let mortgage. Rental profit (rent minus
//!   upkeep and mortgage interest) is taxed, and a profitable sale adds to
//!   your capital gains (a loss carries forward, like a trading loss).
//! * **Lifestyle / utility assets** (solar panels, a car) lose value every
//!   year. Solar cuts your electricity bill; a car just costs money to run.
//!   They never count toward financial independence, like the home you live
//!   in, and a loss on them isn't tax-deductible.
//!
//! Owning is paused while you're away (income and costs both), exactly like
//! the rest of your personal life.

use rand::Rng;
use serde::{Deserialize, Serialize};

use crate::economy::{CAPITAL_GAINS_RATE, INVESTMENT_INCOME_RATE};
use crate::events::NewsKind;
use crate::game::GameState;
use crate::housing::{mortgage_payment, Mortgage, FORCED_SALE_FACTOR};
use crate::player::Player;

/// Most assets you can hold at once (keeps the menus to single digits).
pub const MAX_ASSETS: usize = 8;
/// Buy-to-let mortgage = policy rate + this (dearer than a home loan).
pub const BTL_SPREAD: f64 = 0.03;
/// Down payment on a buy-to-let mortgage.
pub const BTL_DOWN_PAYMENT: f64 = 0.25;
/// Depreciating assets never fall below this share of what you paid.
pub const SCRAP_FLOOR: f64 = 0.05;

/// One thing you can buy. Prices are quoted at an index of 100 and scale with
/// the home price index (property) or the price level (everything else).
#[derive(Debug, Clone, Copy)]
pub struct AssetSpec {
    pub name: &'static str,
    pub blurb: &'static str,
    pub base_price: f64,
    /// True for property: priced by the home price index, value follows it.
    pub is_property: bool,
    /// A buy-to-let mortgage is available.
    pub financeable: bool,
    /// Yearly rent as a share of value (0 = no rent).
    pub gross_yield: f64,
    /// Share of months with no tenant.
    pub vacancy: f64,
    /// Upkeep, tax and insurance, as a yearly share of value.
    pub upkeep_rate: f64,
    /// Extra monthly running cost at a price level of 100.
    pub fixed_monthly_cost: f64,
    /// Monthly saving on your bills at a price level of 100.
    pub monthly_saving: f64,
    /// Yearly loss of value (0 for property).
    pub depreciation: f64,
    /// Fees paid on top when buying / lost from the value when selling.
    pub buy_cost: f64,
    pub sell_cost: f64,
}

impl AssetSpec {
    /// Income-producing property: counts toward financial independence and
    /// is covered by capital gains tax.
    pub fn is_investment(&self) -> bool {
        self.gross_yield > 0.0
    }
}

pub const ASSET_SPECS: &[AssetSpec] = &[
    AssetSpec {
        name: "Rental Flat",
        blurb: "A flat let to tenants. Buy-to-let mortgage available (25% down).",
        base_price: 150_000.0,
        is_property: true,
        financeable: true,
        gross_yield: 0.055,
        vacancy: 0.08,
        upkeep_rate: 0.015,
        fixed_monthly_cost: 0.0,
        monthly_saving: 0.0,
        depreciation: 0.0,
        buy_cost: 0.025,
        sell_cost: 0.06,
    },
    AssetSpec {
        name: "Storage Units",
        blurb: "A block of self-storage units. Cash only, higher yield, patchier tenants.",
        base_price: 45_000.0,
        is_property: true,
        financeable: false,
        gross_yield: 0.085,
        vacancy: 0.14,
        upkeep_rate: 0.02,
        fixed_monthly_cost: 0.0,
        monthly_saving: 0.0,
        depreciation: 0.0,
        buy_cost: 0.02,
        sell_cost: 0.05,
    },
    AssetSpec {
        name: "Solar Panels",
        blurb: "Roof solar: cuts your electricity bill every month. Loses ~5% of its value a year.",
        base_price: 14_000.0,
        is_property: false,
        financeable: false,
        gross_yield: 0.0,
        vacancy: 0.0,
        upkeep_rate: 0.005,
        fixed_monthly_cost: 0.0,
        monthly_saving: 85.0,
        depreciation: 0.05,
        buy_cost: 0.0,
        sell_cost: 0.10,
    },
    AssetSpec {
        name: "Car",
        blurb: "A reliable car. Loses ~14% a year and costs insurance, fuel and servicing.",
        base_price: 26_000.0,
        is_property: false,
        financeable: false,
        gross_yield: 0.0,
        vacancy: 0.0,
        upkeep_rate: 0.0,
        fixed_monthly_cost: 170.0,
        monthly_saving: 0.0,
        depreciation: 0.14,
        buy_cost: 0.0,
        sell_cost: 0.03,
    },
];

/// What the showroom or agent asks for this asset today.
pub fn asset_price(kind: usize, home_price_index: f64, cpi: f64) -> f64 {
    let s = &ASSET_SPECS[kind.min(ASSET_SPECS.len() - 1)];
    let index = if s.is_property { home_price_index } else { cpi };
    s.base_price * index / 100.0
}

/// Mortgage and cash needed up front for `price`, financed or not.
/// Returns (cash needed now, mortgage principal).
pub fn purchase_terms(kind: usize, price: f64, financed: bool) -> (f64, f64) {
    let s = &ASSET_SPECS[kind.min(ASSET_SPECS.len() - 1)];
    let fees = price * s.buy_cost;
    if financed && s.financeable {
        let loan = price * (1.0 - BTL_DOWN_PAYMENT);
        (price - loan + fees, loan)
    } else {
        (price + fees, 0.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnedAsset {
    pub id: u64,
    pub kind: usize,
    pub bought_tick: i64,
    pub price_paid: f64,
    pub hpi_at_purchase: f64,
    /// Current market value, refreshed every payday.
    pub value: f64,
    #[serde(default)]
    pub mortgage: Option<Mortgage>,
}

impl OwnedAsset {
    pub fn spec(&self) -> &'static AssetSpec {
        &ASSET_SPECS[self.kind.min(ASSET_SPECS.len() - 1)]
    }

    pub fn name(&self) -> &'static str {
        self.spec().name
    }

    pub fn debt(&self) -> f64 {
        self.mortgage.as_ref().map_or(0.0, |m| m.principal)
    }

    pub fn equity(&self) -> f64 {
        self.value - self.debt()
    }

    /// Upkeep plus fixed running costs for one month.
    pub fn monthly_running_cost(&self, cpi: f64) -> f64 {
        let s = self.spec();
        self.value * s.upkeep_rate / 12.0 + s.fixed_monthly_cost * cpi / 100.0
    }

    /// Rent you'd get in a month with a tenant.
    pub fn monthly_rent(&self) -> f64 {
        self.value * self.spec().gross_yield / 12.0
    }

    pub fn monthly_saving(&self, cpi: f64) -> f64 {
        self.spec().monthly_saving * cpi / 100.0
    }

    /// What you paid including purchase fees: the basis for a gain or loss.
    pub fn cost_basis(&self) -> f64 {
        self.price_paid * (1.0 + self.spec().buy_cost)
    }

    /// Refreshes `value` once a month: property follows the home price index,
    /// everything else loses a twelfth of its yearly depreciation.
    pub fn revalue(&mut self, home_price_index: f64) {
        let s = self.spec();
        if s.is_property {
            self.value = self.price_paid * home_price_index / self.hpi_at_purchase.max(1e-9);
        } else {
            let monthly = (1.0 - s.depreciation).powf(1.0 / 12.0);
            self.value = (self.value * monthly).max(self.price_paid * SCRAP_FLOOR);
        }
    }
}

impl GameState {
    /// Equity in all assets (value minus any mortgage), for net worth.
    pub fn assets_equity(&self) -> f64 {
        self.assets.iter().map(|a| a.equity()).sum()
    }

    /// Value of the depreciating lifestyle assets: wealth you can't live off,
    /// so it's left out of the financial-independence test.
    pub fn lifestyle_assets_value(&self) -> f64 {
        self.assets.iter().filter(|a| !a.spec().is_investment()).map(|a| a.equity()).sum()
    }

    /// Wealth that doesn't count toward financial independence: the home you
    /// live in plus lifestyle assets.
    pub fn non_fi_wealth(&self) -> f64 {
        self.home_equity() + self.lifestyle_assets_value()
    }

    /// Monthly cost of lifestyle assets (car running costs, solar upkeep) net
    /// of bill savings. Part of what financial independence must cover.
    pub fn lifestyle_net_cost(&self) -> f64 {
        let cpi = self.economy.cpi;
        self.assets
            .iter()
            .filter(|a| !a.spec().is_investment())
            .map(|a| a.monthly_running_cost(cpi) - a.monthly_saving(cpi))
            .sum::<f64>()
            .max(0.0)
    }

    /// Mortgage payments (principal and interest) on assets this month.
    pub fn asset_debt_service(&self) -> f64 {
        self.assets.iter().filter_map(|a| a.mortgage.as_ref()).map(|m| m.payment).sum()
    }

    /// Called every payday, including while away: values move with the market
    /// and the clock.
    pub(crate) fn revalue_assets(&mut self) {
        let hpi = self.economy.home_price_index;
        for a in self.assets.iter_mut() {
            a.revalue(hpi);
        }
    }

    /// Buys an asset from the agent or showroom at the listed price. No
    /// haggling: it's a fixed-price purchase.
    pub fn buy_asset(&mut self, player: &mut Player, free_cash: f64, kind: usize, financed: bool) -> Result<String, String> {
        if self.status.is_lost() {
            return Err("The game is over.".to_string());
        }
        if self.has_prompt() {
            return Err("Finish what's pending first.".to_string());
        }
        if kind >= ASSET_SPECS.len() {
            return Err("No such asset.".to_string());
        }
        if self.assets.len() >= MAX_ASSETS {
            return Err(format!("You can't hold more than {MAX_ASSETS} assets. Sell one first."));
        }
        let spec = &ASSET_SPECS[kind];
        let financed = financed && spec.financeable;
        if financed && !self.career.employed {
            return Err("No bank will give you a mortgage while you're out of work.".to_string());
        }
        let hpi = self.economy.home_price_index;
        let price = asset_price(kind, hpi, self.economy.cpi);
        let (cash_needed, loan) = purchase_terms(kind, price, financed);
        if cash_needed > free_cash {
            return Err(format!("You need {cash_needed:.0} in free cash for the {} (you have {free_cash:.0}).", spec.name));
        }
        let mortgage = if loan > 0.0 {
            let rate = (player.interest_rate + BTL_SPREAD).max(0.0);
            Some(Mortgage { principal: loan, annual_rate: rate, payment: mortgage_payment(loan, rate) })
        } else {
            None
        };
        let now = player.current_tick;
        let id = self.alloc_id();
        self.assets.push(OwnedAsset { id, kind, bought_tick: now, price_paid: price, hpi_at_purchase: hpi, value: price, mortgage });
        player.cash -= cash_needed;
        self.record(now, "asset", format!("Bought {}", spec.name), -cash_needed);
        let how = if loan > 0.0 { format!(", with a {loan:.0} mortgage") } else { String::new() };
        let msg = format!("You buy {} for {price:.0}{how} ({cash_needed:.0} cash up front).", spec.name);
        self.push_news(now, NewsKind::Deal, msg.clone());
        Ok(msg)
    }

    /// Sells asset number `index` at market value less selling costs, clears
    /// any mortgage on it. A taxable gain or loss on investment property goes
    /// into the capital gains bucket.
    pub fn sell_asset(&mut self, player: &mut Player, index: usize) -> Result<String, String> {
        if self.status.is_lost() {
            return Err("The game is over.".to_string());
        }
        let Some(a) = self.assets.get(index) else {
            return Err("No such asset.".to_string());
        };
        let spec = a.spec();
        let net_price = a.value * (1.0 - spec.sell_cost);
        let proceeds = net_price - a.debt();
        if player.cash + proceeds < 0.0 {
            return Err(format!("The sale wouldn't cover the mortgage: you'd need another {:.0} in cash.", -(player.cash + proceeds)));
        }
        let gain = net_price - a.cost_basis();
        let value = a.value;
        self.assets.remove(index);
        player.cash += proceeds;
        let now = player.current_tick;
        self.record(now, "asset", format!("Sold {}", spec.name), proceeds);
        let mut msg = format!("You sell the {} for {value:.0} and net {proceeds:.0} after costs and any mortgage.", spec.name);
        if spec.is_investment() {
            self.pending_gains += gain;
            if gain > 0.0 {
                msg.push_str(&format!(" Taxable gain {gain:.0} ({:.0}% tax at the next payday).", CAPITAL_GAINS_RATE * 100.0));
            } else {
                msg.push_str(&format!(" A loss of {:.0} carries forward against future gains.", -gain));
            }
        }
        self.push_news(now, NewsKind::Deal, msg.clone());
        Ok(msg)
    }

    /// Repays extra mortgage principal on asset `index` from cash.
    pub fn prepay_asset_mortgage(&mut self, player: &mut Player, free_cash: f64, index: usize, amount: f64) -> Result<String, String> {
        if self.status.is_lost() {
            return Err("The game is over.".to_string());
        }
        if !(amount.is_finite() && amount > 0.0) {
            return Err("Enter a positive amount.".to_string());
        }
        let Some(a) = self.assets.get_mut(index) else {
            return Err("No such asset.".to_string());
        };
        let Some(m) = a.mortgage.as_mut() else {
            return Err("That asset has no mortgage.".to_string());
        };
        let pay = amount.min(m.principal);
        if pay > free_cash {
            return Err(format!("You only have {free_cash:.0} in free cash."));
        }
        m.principal -= pay;
        let cleared = m.principal < 1.0;
        if cleared {
            a.mortgage = None;
        }
        let name = a.name();
        player.cash -= pay;
        let now = player.current_tick;
        self.record(now, "mortgage_principal", format!("Extra repayment on the {name} mortgage"), -pay);
        Ok(if cleared { format!("The {name} is now mortgage-free.") } else { format!("You repay {pay:.0} of the {name} mortgage.") })
    }

    /// The monthly run of your assets: rent, upkeep, mortgage payments, bill
    /// savings and the tax on rental profit. Called inside the "personal
    /// life" part of payday, so it's paused while you're away.
    pub(crate) fn assets_payday<R: Rng>(&mut self, player: &mut Player, rng: &mut R) {
        if self.assets.is_empty() {
            return;
        }
        let cpi = self.economy.cpi;
        let now = player.current_tick;
        let (mut rent, mut costs, mut interest, mut principal, mut saving) = (0.0, 0.0, 0.0, 0.0, 0.0);
        // Costs that reduce taxable rental profit: upkeep and mortgage interest.
        let mut deductible = 0.0;
        let mut vacant: Vec<&'static str> = vec![];
        let mut cleared: Vec<&'static str> = vec![];
        for a in self.assets.iter_mut() {
            let spec = a.spec();
            let upkeep = a.monthly_running_cost(cpi);
            costs += upkeep;
            saving += a.monthly_saving(cpi);
            if spec.is_investment() {
                deductible += upkeep;
                if rng.gen::<f64>() < spec.vacancy {
                    vacant.push(spec.name);
                } else {
                    rent += a.monthly_rent();
                }
            }
            let mut done = false;
            if let Some(m) = a.mortgage.as_mut() {
                let i = m.principal * m.annual_rate / 12.0;
                let p = (m.payment - i).clamp(0.0, m.principal);
                interest += i;
                principal += p;
                deductible += i;
                m.principal -= p;
                done = m.principal < 1.0;
            }
            if done {
                a.mortgage = None;
                cleared.push(spec.name);
            }
        }
        let tax = (rent - deductible).max(0.0) * INVESTMENT_INCOME_RATE;
        player.cash += rent + saving - costs - interest - principal - tax;
        self.stats.rental_income += rent;
        self.stats.asset_costs += costs + interest;
        self.stats.interest_paid += interest;
        self.stats.taxes_paid += tax;
        self.record(now, "rent_income", "Rental income", rent);
        self.record(now, "other_income", "Solar savings on your electricity bill", saving);
        self.record(now, "asset_costs", "Upkeep and running costs of your assets", -costs);
        self.record(now, "interest", "Mortgage interest on your assets", -interest);
        self.record(now, "mortgage_principal", "Mortgage principal on your assets", -principal);
        self.record(now, "tax", "Tax on rental profit", -tax);
        if !vacant.is_empty() {
            self.push_news(now, NewsKind::Personal, format!("No tenants this month, so no rent from: {}.", vacant.join(", ")));
        }
        for name in cleared {
            self.push_news(now, NewsKind::Personal, format!("You've paid off the mortgage on the {name}."));
        }
    }

    /// Last-resort sale of assets when bills can't be paid. Returns a note
    /// per asset sold.
    pub(crate) fn forced_asset_sales(&mut self, player: &mut Player) -> Vec<String> {
        let now = player.current_tick;
        let mut sold = vec![];
        while player.cash < 0.0 && !self.assets.is_empty() {
            let a = self.assets.remove(0);
            let proceeds = a.value * FORCED_SALE_FACTOR - a.debt();
            player.cash += proceeds;
            self.record(now, "asset", format!("Forced sale: {}", a.name()), proceeds);
            sold.push(format!("{} (forced sale, {proceeds:+.0} after any mortgage)", a.name()));
        }
        sold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(value: f64, mortgage: Option<Mortgage>) -> OwnedAsset {
        OwnedAsset { id: 1, kind: 0, bought_tick: 0, price_paid: value, hpi_at_purchase: 100.0, value, mortgage }
    }

    #[test]
    fn prices_scale_with_their_index() {
        assert_eq!(asset_price(0, 100.0, 100.0), 150_000.0);
        assert_eq!(asset_price(0, 120.0, 100.0), 180_000.0, "property follows the home price index");
        assert!((asset_price(3, 120.0, 110.0) - 28_600.0).abs() < 1e-6, "a car follows the price level");
    }

    #[test]
    fn financing_a_flat_needs_a_quarter_down_plus_fees() {
        let (cash, loan) = purchase_terms(0, 200_000.0, true);
        assert!((loan - 150_000.0).abs() < 1e-9);
        assert!((cash - (50_000.0 + 5_000.0)).abs() < 1e-9);
        let (cash, loan) = purchase_terms(1, 40_000.0, true);
        assert_eq!(loan, 0.0, "storage units are cash only");
        assert!((cash - 40_800.0).abs() < 1e-9);
    }

    #[test]
    fn property_follows_the_index_and_cars_depreciate() {
        let mut f = flat(150_000.0, None);
        f.revalue(110.0);
        assert!((f.value - 165_000.0).abs() < 1e-6);
        let mut car = OwnedAsset { id: 2, kind: 3, bought_tick: 0, price_paid: 26_000.0, hpi_at_purchase: 100.0, value: 26_000.0, mortgage: None };
        for _ in 0..12 {
            car.revalue(100.0);
        }
        assert!((car.value - 26_000.0 * 0.86).abs() < 1.0, "{}", car.value);
        for _ in 0..600 {
            car.revalue(100.0);
        }
        assert!((car.value - 26_000.0 * SCRAP_FLOOR).abs() < 1e-6, "value stops at the scrap floor");
    }

    #[test]
    fn equity_is_value_minus_mortgage() {
        let f = flat(150_000.0, Some(Mortgage { principal: 112_500.0, annual_rate: 0.06, payment: 674.5 }));
        assert_eq!(f.equity(), 37_500.0);
        assert_eq!(f.debt(), 112_500.0);
    }
}
