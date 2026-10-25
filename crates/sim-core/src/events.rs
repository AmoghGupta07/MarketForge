//! Random events and life decisions: the catalog (static data) and the
//! small state structs that live in `GameState`.

use rand::Rng;
use serde::{Deserialize, Serialize};

use crate::instrument::{AssetClass, Instrument};

// ---------------------------------------------------------------- targets

/// Which instruments an effect touches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Target {
    All,
    Class(AssetClass),
    Symbol(String),
}

impl Target {
    pub fn matches(&self, i: &Instrument) -> bool {
        match self {
            Target::All => true,
            Target::Class(c) => i.class == *c,
            Target::Symbol(s) => i.symbol == *s,
        }
    }
}

/// `Target` for static catalog data (no heap strings).
#[derive(Clone, Copy)]
pub enum T {
    All,
    Class(AssetClass),
    Sym(&'static str),
}

impl T {
    pub fn owned(self) -> Target {
        match self {
            T::All => Target::All,
            T::Class(c) => Target::Class(c),
            T::Sym(s) => Target::Symbol(s.to_string()),
        }
    }
}

// ----------------------------------------------------------- market events

/// A market-moving event. Fires some ticks after it is scheduled
/// (brokers with a good relationship may tip you off in between).
pub struct MarketEventDef {
    pub key: &'static str,
    pub headline: &'static str,
    /// The rumor text a tipster passes along beforehand.
    pub tip: &'static str,
    pub weight: f64,
    /// Instant change to the prevailing interest rate (absolute).
    pub rate_shock: f64,
    /// Instant price jumps (fraction, e.g. -0.12 = -12%).
    pub shocks: &'static [(T, f64)],
    /// How long the lingering effects below last.
    pub duration: i64,
    /// Annualized drift added while active.
    pub drift: &'static [(T, f64)],
    /// Volatility multiplier while active.
    pub vol: &'static [(T, f64)],
    /// Multiplier on business profits while active (1.0 = none).
    pub econ: f64,
}

use AssetClass as C;

pub static MARKET_EVENTS: &[MarketEventDef] = &[
    MarketEventDef {
        key: "crash",
        headline: "MARKET CRASH: stocks plunge on recession fears",
        tip: "a big sell-off is coming for stocks",
        weight: 0.8,
        rate_shock: 0.0,
        shocks: &[(T::Class(C::Equity), -0.13), (T::Class(C::IndexFund), -0.10), (T::Class(C::Crypto), -0.18)],
        duration: 60,
        drift: &[(T::Class(C::Equity), -0.30), (T::Class(C::IndexFund), -0.25), (T::Class(C::Crypto), -0.40)],
        vol: &[(T::Class(C::Equity), 1.8), (T::Class(C::IndexFund), 1.8), (T::Class(C::Crypto), 1.5)],
        econ: 0.85,
    },
    MarketEventDef {
        key: "recession",
        headline: "RECESSION: the economy contracts, businesses struggle",
        tip: "economists are about to call a recession",
        weight: 0.5,
        rate_shock: -0.008,
        shocks: &[(T::Class(C::Equity), -0.06), (T::Class(C::RealEstate), -0.04)],
        duration: 150,
        drift: &[(T::Class(C::Equity), -0.25), (T::Class(C::RealEstate), -0.15), (T::Class(C::Commodity), -0.15)],
        vol: &[(T::Class(C::Equity), 1.4)],
        econ: 0.65,
    },
    MarketEventDef {
        key: "boom",
        headline: "BOOM: strong growth lifts the whole economy",
        tip: "growth numbers are about to surprise on the upside",
        weight: 0.6,
        rate_shock: 0.004,
        shocks: &[(T::Class(C::Equity), 0.05), (T::Class(C::Commodity), 0.04)],
        duration: 120,
        drift: &[(T::Class(C::Equity), 0.20), (T::Class(C::IndexFund), 0.15), (T::Class(C::Commodity), 0.10)],
        vol: &[],
        econ: 1.25,
    },
    MarketEventDef {
        key: "rate_cut",
        headline: "Central bank cuts rates, markets rally",
        tip: "the central bank is about to cut rates",
        weight: 0.8,
        rate_shock: -0.010,
        shocks: &[(T::Class(C::Equity), 0.05), (T::Class(C::RealEstate), 0.05)],
        duration: 40,
        drift: &[(T::Class(C::Equity), 0.15), (T::Class(C::RealEstate), 0.15)],
        vol: &[],
        econ: 1.0,
    },
    MarketEventDef {
        key: "rate_hike",
        headline: "Central bank hikes rates, property and stocks fall",
        tip: "a surprise rate hike is on the way",
        weight: 0.8,
        rate_shock: 0.015,
        shocks: &[(T::Class(C::RealEstate), -0.05), (T::Class(C::Equity), -0.03)],
        duration: 60,
        drift: &[(T::Class(C::RealEstate), -0.10)],
        vol: &[],
        econ: 1.0,
    },
    MarketEventDef {
        key: "oil_spike",
        headline: "Supply disruption sends oil soaring",
        tip: "oil supply is about to be disrupted",
        weight: 0.8,
        rate_shock: 0.0,
        shocks: &[(T::Sym("OIL"), 0.18), (T::Sym("NRGX"), 0.07)],
        duration: 30,
        drift: &[],
        vol: &[(T::Sym("OIL"), 1.6)],
        econ: 1.0,
    },
    MarketEventDef {
        key: "oil_glut",
        headline: "Oil glut: prices slump as supply floods the market",
        tip: "there's a glut of oil about to hit the market",
        weight: 0.7,
        rate_shock: 0.0,
        shocks: &[(T::Sym("OIL"), -0.14), (T::Sym("NRGX"), -0.05)],
        duration: 30,
        drift: &[],
        vol: &[(T::Sym("OIL"), 1.4)],
        econ: 1.0,
    },
    MarketEventDef {
        key: "tech_boom",
        headline: "Applecore unveils a hit product, shares surge",
        tip: "Applecore has something big coming",
        weight: 0.6,
        rate_shock: 0.0,
        shocks: &[(T::Sym("AAPL"), 0.11)],
        duration: 30,
        drift: &[(T::Sym("AAPL"), 0.25)],
        vol: &[],
        econ: 1.0,
    },
    MarketEventDef {
        key: "harvest_failure",
        headline: "Harvest failure sends wheat prices higher",
        tip: "bad weather is wrecking the wheat harvest",
        weight: 0.6,
        rate_shock: 0.0,
        shocks: &[(T::Sym("WHET"), 0.15), (T::Sym("GROV"), -0.04)],
        duration: 40,
        drift: &[],
        vol: &[(T::Sym("WHET"), 1.5)],
        econ: 1.0,
    },
    MarketEventDef {
        key: "crypto_boom",
        headline: "Crypto frenzy: retail money floods in",
        tip: "crypto is about to run",
        weight: 0.7,
        rate_shock: 0.0,
        shocks: &[(T::Class(C::Crypto), 0.25)],
        duration: 40,
        drift: &[(T::Class(C::Crypto), 0.50)],
        vol: &[(T::Class(C::Crypto), 1.5)],
        econ: 1.0,
    },
    MarketEventDef {
        key: "crypto_crackdown",
        headline: "Regulators crack down on crypto exchanges",
        tip: "regulators are about to go after crypto",
        weight: 0.6,
        rate_shock: 0.0,
        shocks: &[(T::Class(C::Crypto), -0.30)],
        duration: 40,
        drift: &[(T::Class(C::Crypto), -0.30)],
        vol: &[(T::Class(C::Crypto), 1.5)],
        econ: 1.0,
    },
    MarketEventDef {
        key: "aapl_miss",
        headline: "Applecore misses earnings badly, shares tumble",
        tip: "Applecore's numbers are going to disappoint",
        weight: 0.5,
        rate_shock: 0.0,
        shocks: &[(T::Sym("AAPL"), -0.10)],
        duration: 20,
        drift: &[],
        vol: &[(T::Sym("AAPL"), 1.5)],
        econ: 1.0,
    },
    MarketEventDef {
        key: "grov_scandal",
        headline: "Food safety scandal hits Grover Foods",
        tip: "something ugly is about to come out at Grover Foods",
        weight: 0.4,
        rate_shock: 0.0,
        shocks: &[(T::Sym("GROV"), -0.14), (T::Sym("WHET"), -0.02)],
        duration: 30,
        drift: &[(T::Sym("GROV"), -0.20)],
        vol: &[(T::Sym("GROV"), 1.6)],
        econ: 1.0,
    },
    MarketEventDef {
        key: "nrgx_buyout",
        headline: "Nordic Energy receives a takeover bid, shares jump",
        tip: "someone is circling Nordic Energy",
        weight: 0.4,
        rate_shock: 0.0,
        shocks: &[(T::Sym("NRGX"), 0.16)],
        duration: 0,
        drift: &[],
        vol: &[],
        econ: 1.0,
    },
    MarketEventDef {
        key: "inflation_spike",
        headline: "Inflation surges: prices jump, the central bank turns hawkish",
        tip: "inflation numbers are going to come in hot",
        weight: 0.5,
        rate_shock: 0.008,
        shocks: &[(T::Class(C::Equity), -0.04), (T::Class(C::RealEstate), -0.03)],
        duration: 60,
        drift: &[(T::Class(C::Equity), -0.08), (T::Class(C::Commodity), 0.10)],
        vol: &[],
        econ: 0.95,
    },
    MarketEventDef {
        key: "stimulus",
        headline: "Government announces a stimulus package",
        tip: "the government is preparing a stimulus package",
        weight: 0.5,
        rate_shock: 0.0,
        shocks: &[(T::Class(C::Equity), 0.04)],
        duration: 90,
        drift: &[(T::Class(C::Equity), 0.10), (T::Class(C::Commodity), 0.05)],
        vol: &[],
        econ: 1.10,
    },
    MarketEventDef {
        key: "dollar_slump",
        headline: "The dollar slumps against major currencies",
        tip: "the dollar is about to weaken",
        weight: 0.5,
        rate_shock: 0.0,
        shocks: &[(T::Sym("EURUSD"), 0.05), (T::Sym("GBPUSD"), 0.04), (T::Sym("USDJPY"), -0.04)],
        duration: 0,
        drift: &[],
        vol: &[],
        econ: 1.0,
    },
    MarketEventDef {
        key: "housing_boom",
        headline: "Housing boom: property trusts rally",
        tip: "property prices are about to heat up",
        weight: 0.6,
        rate_shock: 0.0,
        shocks: &[(T::Class(C::RealEstate), 0.07)],
        duration: 90,
        drift: &[(T::Class(C::RealEstate), 0.12)],
        vol: &[],
        econ: 1.0,
    },
];

pub fn market_event(key: &str) -> Option<&'static MarketEventDef> {
    MARKET_EVENTS.iter().find(|e| e.key == key)
}

/// A lingering market effect currently in force.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveEffect {
    pub label: String,
    pub ends_tick: i64,
    pub drift: Vec<(Target, f64)>,
    pub vol: Vec<(Target, f64)>,
    pub econ: f64,
}

/// A market event that has been decided but hasn't happened yet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledEvent {
    pub key: String,
    pub fire_tick: i64,
}

// -------------------------------------------------------------- effects

/// Something that happens to *you*. Used by personal events and by the
/// outcomes of decisions.
#[derive(Clone, Copy)]
pub enum Effect {
    Cash(f64),
    /// Multiple of your monthly salary.
    CashSalaryFrac(f64),
    /// Permanent change to monthly expenses.
    ExpenseDelta(f64),
    /// Change in your rent (and, if you rent, your expenses).
    RentDelta(f64),
    /// Monthly expense (negative = income) for a limited time.
    TempExpense { amount: f64, ticks: i64, label: &'static str },
    CashLater { amount: f64, ticks: i64, label: &'static str },
    Skill(f64),
    /// Permanent salary change (fraction).
    SalaryPct(f64),
    LoseJob { min_ticks: i64, max_ticks: i64 },
    /// Government changes the tax stance (additive on the rate multiplier).
    TaxMult(f64),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Cond {
    Always,
    Employed,
    /// You rent your home.
    Renting,
    /// You own your home.
    Owning,
}

pub struct PersonalEventDef {
    pub key: &'static str,
    pub headline: &'static str,
    pub weight: f64,
    pub cond: Cond,
    pub effects: &'static [Effect],
}

pub static PERSONAL_EVENTS: &[PersonalEventDef] = &[
    PersonalEventDef { key: "tax_refund", headline: "A tax refund of $800 lands in your account", weight: 1.0, cond: Cond::Always, effects: &[Effect::Cash(800.0)] },
    PersonalEventDef { key: "appliance", headline: "Your washing machine dies: $450 for a replacement", weight: 1.0, cond: Cond::Always, effects: &[Effect::Cash(-450.0)] },
    PersonalEventDef { key: "inheritance", headline: "A distant relative leaves you $5,000", weight: 0.25, cond: Cond::Always, effects: &[Effect::Cash(5_000.0)] },
    PersonalEventDef { key: "bonus", headline: "Your company pays a bonus worth most of a month's salary", weight: 0.8, cond: Cond::Employed, effects: &[Effect::CashSalaryFrac(0.8)] },
    PersonalEventDef { key: "surprise_raise", headline: "Your manager gives you an unscheduled 4% raise", weight: 0.12, cond: Cond::Employed, effects: &[Effect::SalaryPct(0.04)] },
    PersonalEventDef { key: "tax_cut", headline: "Parliament passes a tax cut", weight: 0.25, cond: Cond::Always, effects: &[Effect::TaxMult(-0.08)] },
    PersonalEventDef { key: "tax_hike", headline: "Parliament raises taxes to fund new spending", weight: 0.25, cond: Cond::Always, effects: &[Effect::TaxMult(0.08)] },
    PersonalEventDef { key: "layoff", headline: "LAYOFFS: your company cuts your position. You're out of work", weight: 0.07, cond: Cond::Employed, effects: &[Effect::LoseJob { min_ticks: 45, max_ticks: 120 }] },
];

pub fn personal_event(key: &str) -> Option<&'static PersonalEventDef> {
    PERSONAL_EVENTS.iter().find(|e| e.key == key)
}

// ------------------------------------------------------------- decisions

pub struct OutcomeDef {
    pub text: &'static str,
    pub effects: &'static [Effect],
}

pub struct OptionDef {
    pub label: &'static str,
    /// (probability, outcome); probabilities sum to 1.
    pub outcomes: &'static [(f64, OutcomeDef)],
}

pub struct DecisionDef {
    pub key: &'static str,
    pub title: &'static str,
    pub body: &'static str,
    pub weight: f64,
    pub cond: Cond,
    /// Chosen automatically if you don't decide in time.
    pub default_option: usize,
    pub options: &'static [OptionDef],
}

/// Game days you have to answer a decision.
pub const DECISION_WINDOW_TICKS: i64 = 20;

pub static DECISIONS: &[DecisionDef] = &[
    DecisionDef {
        key: "car_trouble",
        title: "Car trouble",
        body: "Your car's transmission is failing. The mechanic quotes $1,800 to fix it.",
        weight: 1.0,
        cond: Cond::Always,
        default_option: 0,
        options: &[
            OptionDef { label: "Pay for the repair ($1,800)", outcomes: &[(1.0, OutcomeDef { text: "You pay the mechanic and the car runs fine.", effects: &[Effect::Cash(-1_800.0)] })] },
            OptionDef { label: "Sell it and take transit (+$2,500, -$60/mo)", outcomes: &[(1.0, OutcomeDef { text: "You sell the car. Transit costs less to run.", effects: &[Effect::Cash(2_500.0), Effect::ExpenseDelta(-60.0)] })] },
            OptionDef {
                label: "Gamble on a cheap fix ($400)",
                outcomes: &[
                    (0.5, OutcomeDef { text: "The cheap fix holds up!", effects: &[Effect::Cash(-400.0)] }),
                    (0.5, OutcomeDef { text: "It dies again within weeks. Total bill: $2,600.", effects: &[Effect::Cash(-2_600.0)] }),
                ],
            },
        ],
    },
    DecisionDef {
        key: "rent_increase",
        title: "Rent increase",
        body: "Your landlord is raising the rent by $100 a month.",
        weight: 0.6,
        cond: Cond::Renting,
        default_option: 0,
        options: &[
            OptionDef { label: "Accept (+$100/mo)", outcomes: &[(1.0, OutcomeDef { text: "You grumble and sign the new lease.", effects: &[Effect::RentDelta(100.0)] })] },
            OptionDef {
                label: "Negotiate",
                outcomes: &[
                    (0.5, OutcomeDef { text: "The landlord backs down. No increase.", effects: &[] }),
                    (0.5, OutcomeDef { text: "No luck, and you annoyed them. Rent goes up $150.", effects: &[Effect::RentDelta(150.0)] }),
                ],
            },
            OptionDef { label: "Move somewhere cheaper (-$600 now, -$40/mo)", outcomes: &[(1.0, OutcomeDef { text: "Moving is a hassle, but the new place is cheaper.", effects: &[Effect::Cash(-600.0), Effect::RentDelta(-40.0)] })] },
        ],
    },
    DecisionDef {
        key: "roof_leak",
        title: "A leaking roof",
        body: "Your roof is leaking. A full replacement costs $6,000.",
        weight: 0.7,
        cond: Cond::Owning,
        default_option: 0,
        options: &[
            OptionDef { label: "Replace the roof ($6,000)", outcomes: &[(1.0, OutcomeDef { text: "A new roof. No more worries for decades.", effects: &[Effect::Cash(-6_000.0)] })] },
            OptionDef {
                label: "Patch it ($1,200)",
                outcomes: &[
                    (0.5, OutcomeDef { text: "The patch holds.", effects: &[Effect::Cash(-1_200.0)] }),
                    (0.5, OutcomeDef { text: "The patch fails in the next storm. Repairs total $5,000.", effects: &[Effect::Cash(-5_000.0)] }),
                ],
            },
            OptionDef {
                label: "Ignore it",
                outcomes: &[
                    (0.7, OutcomeDef { text: "You got lucky this year.", effects: &[] }),
                    (0.3, OutcomeDef { text: "Water damage! The bill is $9,000.", effects: &[Effect::Cash(-9_000.0)] }),
                ],
            },
        ],
    },
    DecisionDef {
        key: "startup_friend",
        title: "A friend's startup",
        body: "A friend's startup is raising money and wants $2,000 from you.",
        weight: 0.8,
        cond: Cond::Always,
        default_option: 1,
        options: &[
            OptionDef {
                label: "Invest $2,000",
                outcomes: &[
                    (0.3, OutcomeDef { text: "The startup gets acquired! Your stake pays $9,000 in a few months.", effects: &[Effect::Cash(-2_000.0), Effect::CashLater { amount: 9_000.0, ticks: 120, label: "startup acquisition payout" }] }),
                    (0.7, OutcomeDef { text: "The startup folds. Your $2,000 is gone.", effects: &[Effect::Cash(-2_000.0)] }),
                ],
            },
            OptionDef { label: "Politely decline", outcomes: &[(1.0, OutcomeDef { text: "You pass. Your friend understands.", effects: &[] })] },
        ],
    },
    DecisionDef {
        key: "training_offer",
        title: "Training course",
        body: "Your employer will subsidize a professional course, but you pay $800 yourself.",
        weight: 0.9,
        cond: Cond::Employed,
        default_option: 1,
        options: &[
            OptionDef { label: "Take it (-$800, skill +1.5)", outcomes: &[(1.0, OutcomeDef { text: "You finish the course. Your boss notices.", effects: &[Effect::Cash(-800.0), Effect::Skill(1.5)] })] },
            OptionDef { label: "Decline", outcomes: &[(1.0, OutcomeDef { text: "You skip it.", effects: &[] })] },
        ],
    },
    DecisionDef {
        key: "job_offer",
        title: "A competing job offer",
        body: "A competitor is offering you a job at 12% higher pay.",
        weight: 0.7,
        cond: Cond::Employed,
        default_option: 2,
        options: &[
            OptionDef {
                label: "Take the new job",
                outcomes: &[
                    (0.8, OutcomeDef { text: "You switch jobs. Salary up 12%.", effects: &[Effect::SalaryPct(0.12)] }),
                    (0.2, OutcomeDef { text: "The job isn't what they promised. Salary up only 5%, plus $500 in moving costs.", effects: &[Effect::SalaryPct(0.05), Effect::Cash(-500.0)] }),
                ],
            },
            OptionDef {
                label: "Use it to ask for a raise",
                outcomes: &[
                    (0.5, OutcomeDef { text: "Your boss matches it. Salary up 8%.", effects: &[Effect::SalaryPct(0.08)] }),
                    (0.5, OutcomeDef { text: "Your boss isn't impressed. Nothing changes.", effects: &[] }),
                ],
            },
            OptionDef { label: "Decline", outcomes: &[(1.0, OutcomeDef { text: "You stay put.", effects: &[] })] },
        ],
    },
    DecisionDef {
        key: "medical_bill",
        title: "Medical bill",
        body: "An unexpected medical bill arrives: $2,400.",
        weight: 0.8,
        cond: Cond::Always,
        default_option: 1,
        options: &[
            OptionDef { label: "Pay in full ($2,400)", outcomes: &[(1.0, OutcomeDef { text: "You pay it off at once.", effects: &[Effect::Cash(-2_400.0)] })] },
            OptionDef { label: "Payment plan ($210/mo for a year)", outcomes: &[(1.0, OutcomeDef { text: "You set up the plan.", effects: &[Effect::TempExpense { amount: 210.0, ticks: 360, label: "medical payment plan" }] })] },
        ],
    },
    DecisionDef {
        key: "family_loan",
        title: "A relative asks for a loan",
        body: "A relative asks to borrow $3,000 and promises to pay you back.",
        weight: 0.8,
        cond: Cond::Always,
        default_option: 1,
        options: &[
            OptionDef {
                label: "Lend $3,000",
                outcomes: &[
                    (0.6, OutcomeDef { text: "They'll repay $3,300 in about six months.", effects: &[Effect::Cash(-3_000.0), Effect::CashLater { amount: 3_300.0, ticks: 180, label: "family loan repayment" }] }),
                    (0.4, OutcomeDef { text: "Months pass and the money never comes back.", effects: &[Effect::Cash(-3_000.0)] }),
                ],
            },
            OptionDef { label: "Say no", outcomes: &[(1.0, OutcomeDef { text: "Things are a bit awkward at family dinners.", effects: &[] })] },
        ],
    },
    DecisionDef {
        key: "side_gig",
        title: "Weekend work",
        body: "A friend offers you weekend work: $500 a month for the next year.",
        weight: 0.8,
        cond: Cond::Always,
        default_option: 1,
        options: &[
            OptionDef {
                label: "Take it (+$500/mo)",
                outcomes: &[
                    (0.7, OutcomeDef { text: "You power through the year. Easy money.", effects: &[Effect::TempExpense { amount: -500.0, ticks: 360, label: "weekend work income" }] }),
                    (0.3, OutcomeDef { text: "You burn out after three months. It hurts your day job (skill -1).", effects: &[Effect::TempExpense { amount: -500.0, ticks: 90, label: "weekend work income" }, Effect::Skill(-1.0)] }),
                ],
            },
            OptionDef { label: "Pass", outcomes: &[(1.0, OutcomeDef { text: "You enjoy your weekends.", effects: &[] })] },
        ],
    },
];

pub fn decision(key: &str) -> Option<&'static DecisionDef> {
    DECISIONS.iter().find(|d| d.key == key)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingDecision {
    pub key: String,
    pub deadline_tick: i64,
}

// ------------------------------------------------------------- bookkeeping

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledCash {
    pub due_tick: i64,
    pub amount: f64,
    pub label: String,
}

/// A recurring monthly expense (negative = income) that ends.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TempExpense {
    pub ends_tick: i64,
    pub amount: f64,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NewsKind {
    Market,
    Personal,
    Career,
    Deal,
    Social,
    Goal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewsItem {
    pub tick: i64,
    pub kind: NewsKind,
    pub text: String,
}

/// Picks one item with probability proportional to its weight.
pub fn pick_weighted<'a, X, R: Rng>(items: &[&'a X], weight: impl Fn(&X) -> f64, rng: &mut R) -> Option<&'a X> {
    let total: f64 = items.iter().map(|x| weight(x)).sum();
    if items.is_empty() || total <= 0.0 {
        return None;
    }
    let mut roll = rng.gen_range(0.0..total);
    for x in items {
        let w = weight(x);
        if roll < w {
            return Some(*x);
        }
        roll -= w;
    }
    items.last().copied()
}
