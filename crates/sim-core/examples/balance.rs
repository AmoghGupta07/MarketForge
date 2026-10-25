//! Headless balance check: plays many 10-year games with simple
//! strategies and prints how they end. Run with:
//!   cargo run -p sim-core --example balance --release
use rand::rngs::StdRng;
use rand::SeedableRng;
use sim_core::*;

fn inst(id: i32, sym: &str, class: AssetClass, price: f64, drift: f64, vol: f64) -> InstrumentState {
    InstrumentState {
        instrument: Instrument {
            id, symbol: sym.into(), name: sym.into(), class, base_price: price, drift, volatility: vol,
            face_value: 0.0, coupon_rate: 0.0, duration: 0.0, income_yield: 0.0, rate_sensitivity: 0.0,
            expense_ratio: 0.0, constituents: vec![],
        },
        price,
    }
}

fn inst_y(id: i32, sym: &str, class: AssetClass, price: f64, drift: f64, vol: f64, y: f64) -> InstrumentState {
    let mut s = inst(id, sym, class, price, drift, vol);
    s.instrument.income_yield = y;
    s
}

fn play(seed: u64, strategy: &str) -> (f64, GameStatus, usize, usize, usize, f64) {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut p = Player {
        id: 1, game_mode: GameMode::SalariedInvestor, cash: 2_000.0, salary: 4_000.0, expenses: 3_200.0,
        current_tick: 0, interest_rate: LONG_RUN_INTEREST_RATE,
    };
    let mut inst_set = vec![
        inst_y(1, "IDX1", AssetClass::IndexFund, 100.0, 0.062, 0.15, 0.018),
        inst(2, "OIL", AssetClass::Commodity, 78.0, 0.02, 0.40),
        inst(3, "SAVE", AssetClass::Savings, 1.0, 0.04, 0.0),
        inst_y(4, "AAPL", AssetClass::Equity, 180.0, 0.115, 0.28, 0.005),
        inst_y(5, "NRGX", AssetClass::Equity, 65.0, 0.03, 0.35, 0.04),
        inst(6, "WHET", AssetClass::Commodity, 6.5, 0.03, 0.30),
        inst(7, "BTC", AssetClass::Crypto, 60000.0, 0.15, 0.70),
        inst(8, "PROP", AssetClass::RealEstate, 50.0, 0.05, 0.14),
        inst_y(9, "GROV", AssetClass::Equity, 42.0, 0.035, 0.22, 0.025),
        inst(10, "STEE", AssetClass::Commodity, 850.0, 0.03, 0.25),
        inst(11, "ETH", AssetClass::Crypto, 3000.0, 0.12, 0.85),
        inst(12, "HOME", AssetClass::RealEstate, 250.0, 0.045, 0.10),
    ];
    let mut pf = Portfolio::default();
    let mut m = Market::new(&inst_set, vec![]);
    let mut g = GameState::new(&p, 2_000.0, true);
    let months: i64 = std::env::var("MONTHS").ok().and_then(|m| m.parse().ok()).unwrap_or(120);
    for month in 0..months {
        fast_forward_with_rng(&mut p, &mut inst_set, &mut pf, &mut m, &mut g, 30, false, &mut rng);
        if g.status.is_lost() {
            break;
        }
        // Answer prompts with the default / accept the offer.
        if g.decision.is_some() {
            let d = decision(&g.decision.as_ref().unwrap().key).unwrap().default_option;
            g.choose_decision(&mut p, d, &mut rng).ok();
        }
        // Invest spare cash above a 3-month buffer.
        let buffer = 3.0 * p.expenses;
        if strategy == "index" && p.cash > buffer + 500.0 && month > 0 {
            let spend = p.cash - buffer;
            let qty = spend / inst_set[0].price;
            let _ = m.market_order(&mut p, &mut pf, 1, true, qty * 0.98);
        }
        if strategy == "crypto" && p.cash > buffer + 500.0 && month > 0 {
            let spend = p.cash - buffer;
            let qty = spend / inst_set[6].price;
            let _ = m.market_order(&mut p, &mut pf, 7, true, qty * 0.98);
        }
    }
    let prices: std::collections::HashMap<i32, f64> = inst_set.iter().map(|s| (s.instrument.id, s.price)).collect();
    let nw = p.cash + pf.market_value(&prices) + g.assets_value(&prices);
    let count = |k: NewsKind| g.news.iter().filter(|n| n.kind == k).count();
    let layoffs = g.news.iter().filter(|n| n.text.contains("LAYOFFS")).count();
    let rivals = g.rivals.iter().map(|r| r.wealth).sum::<f64>() / g.rivals.len() as f64;
    (nw, g.status.clone(), count(NewsKind::Market), layoffs, g.career.title_idx, rivals)
}

fn main() {
    for strategy in ["cash", "index", "crypto"] {
        let mut nws = vec![];
        let (mut lost, mut won, mut layoffs, mut promo, mut rival) = (0, 0, 0, 0usize, 0.0);
        for seed in 0..60 {
            let (nw, status, _mkt, lo, title, rv) = play(seed, strategy);
            rival += rv;
            nws.push(nw);
            lost += status.is_lost() as usize;
            won += status.is_won() as usize;
            layoffs += lo;
            promo += title;
        }
        nws.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!(
            "{strategy:>7}: net worth at end  p10 {:>9.0}  median {:>9.0}  p90 {:>9.0} | lost {lost}/60  won {won}/60 | avg promotions {:.1} | layoffs/game {:.1} | avg rival {:.0}",
            nws[6], nws[30], nws[54], promo as f64 / 60.0, layoffs as f64 / 60.0, rival / 60.0
        );
    }
}
