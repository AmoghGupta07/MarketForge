# MarketForge

A CLI economy sim with a Postgres backend, built around one shared
tick/pricing engine. Currently one game mode: **Salaried Investor**. You
have a career, bills and a rival field. Invest across stocks, funds,
savings, bonds, property, currencies and crypto, survive what life and the
market throw at you, and reach **financial independence**.

## Requirements

- Rust (stable, recent; a `Cargo.lock` with pinned versions is included)
- A local Postgres server, running

## Setup

1. Create a database: `createdb econsim`
2. Tell the app where it is. Either export it:
   ```bash
   export DATABASE_URL="postgres://localhost/econsim"
   ```
   or copy `.env.example` to `.env` and edit it. The app reads `.env`
   from the working directory or any parent. Real environment variables
   override the file, and with neither set it defaults to
   `postgres://localhost/econsim`.
3. `cargo run --bin econsim` - migrations and instrument seeding run
   automatically. Existing databases upgrade in place: new instruments
   are added by symbol, and existing saves start trading them from
   their current tick at the base price.

## How it plays

- Saves: pick one by number, `n` for new, `d` to delete. Each save has
  its own cash, tick count, interest rate, holdings, open orders and
  catch-up timing.
- Catch-up: loading a save fast-forwards it by the real time since it was
  last played (one game day = 60 real seconds, the same pace as live play), capped
  at 1,440 days (one real day away, about four years of game time) so a week away
  doesn't fast-forward decades; set `MARKETFORGE_MAX_AWAY_TICKS` to change the cap). **While you're away your personal
  life is paused**: no salary or unemployment benefit, no bills or loan
  payments, no reviews, personal events or decisions, and rivals don't save
  either, so you can't come back to a pile of cash you didn't earn by
  playing. The world doesn't pause: prices, the economy, market events, resting
  limit orders, forward settlement and your investment income (dividends,
  coupons, rent, savings interest, businesses) all keep running. Pending
  decisions wait for you. Skipping time with `n` while playing still pays you,
  since you chose to advance the clock. Set `MARKETFORGE_OFFLINE_PAY=1` (in
  `.env` or the environment) to restore the old "live your whole life offline"
  behavior.
- Live: **one tick (one simulated day) is 60 seconds**, both while you play and
  when catching up. A 30-day month is half an hour and a year is six hours of play.
  Set `MARKETFORGE_TICK_SECONDS` in `.env` to change it. `t` advances one tick, `n`
  skips N.
- **Everything time-limited is measured in game days**, not real time: you have
  20 days to answer a decision or a salary review, business listings last 120 days
  and homes 180, new listings appear about every 60 days, forwards run 30/60/90
  days, and broker tips and market events come 4-14 days apart. How long a day
  takes on your clock is just the one pace setting above.
- 1 tick = 1 simulated day. Salary/expenses, bond coupons and rent are
  paid every 30 days.

## Trading: order book and market-makers

Every instrument has an NPC market-maker quoting a ladder of orders
around the instrument's fair price: asks above, bids below, with a
class-specific spread and finite depth per level.

- **Market order** (`b` / `s`): walks the ladder now. You pay the
  spread, big orders slip into worse levels, and an order larger than the
  visible depth fills only partially.
- **Limit order** (`l`, then `b`/`s`, quantity, price): fills immediately
  against any levels at or better than your price; the rest rests in the
  book.
- **Batch clearing**: each tick the market-makers re-quote around the new
  price and your resting orders are matched oldest-first against the fresh
  ladders. That is how a resting order fills as time passes, including
  during skips and catch-up.
- Resting buys reserve cash and resting sells reserve units, so you can't
  commit the same money or shares twice (the header shows free cash). A
  resting buy that can no longer be funded when it would fill (say,
  expenses drained your cash) is cancelled.
- `x` cancels all your open orders on the selected instrument. The
  order-book view (`w`) shows the ladder and your orders; the trades view
  shows recent fills.

Spreads and depth by class: forex and index/mutual funds are tight and
deep; commodities, bonds and equities are in between; real estate and
crypto are wide and thin. Savings has no spread and no depth limit.

## Instruments (19 tradable)

| Class | Symbols | Pricing |
|---|---|---|
| Equity | AAPL, NRGX, GROV | GBM random walk from per-instrument drift/volatility |
| Index Fund | IDX1 | standalone GBM |
| Commodity | WHET, OIL, STEE | GBM |
| Bond | GOVB, CORP | deterministic: moves against the gap between the save's prevailing rate and the bond's coupon, scaled by duration; coupons paid monthly |
| Real Estate | PROP, HOME | GBM whose drift falls when the prevailing rate is above its 4% long-run level; pays monthly rent (yield on price) |
| Forex | EURUSD, GBPUSD, USDJPY | mean-reverting around a long-run level (Ornstein-Uhlenbeck on log price), no drift |
| Crypto | BTC, ETH | GBM with high drift and very high volatility; fractional quantities |
| Mutual Fund | BALF, GRWF, REAL | derived: NAV moves by the weighted total return (price change plus income) of its basket each tick, minus the annual expense ratio |

The market list shows what each type pays you to hold it (dividend yield, rent,
current bond yield, fund fee, or nothing for commodities, currencies and crypto).

**Bank savings is not on the market.** It used to be an instrument called SAVE with a
price chart, which made a bank account look like a stock. It is now only the savings
account (`m`, then 3/4; Deals tab, Money & home view), shown as a balance with its
rate, interest earned and a compound-interest projection. Older saves that held SAVE
have its value moved into the account automatically.

Mutual fund baskets: BALF = IDX1 45% / GOVB 30% / CORP 15% / PROP 10%;
GRWF = AAPL 35% / NRGX 25% / GROV 15% / IDX1 25%; REAL = PROP 30% /
HOME 30% / OIL 20% / STEE 10% / WHET 10%. Weights are effectively
rebalanced every tick.

## The game: goals, events, career, deals

**Win / lose.** You win when your net worth, at a 5% withdrawal rate, covers
your monthly expenses (net worth of 20x annual expenses). You can keep playing
after. You lose by bankruptcy: if you can't cover your bills at a payday, the
game cancels your resting buy orders and force-sells holdings (largest first,
shown as FORCED SALE), then businesses at a 30% discount. If you're still short,
that's a strike; three paydays in a row and the save is frozen as GAME OVER.
Net worth includes businesses (at paper value) and forward contracts (marked
to market). Milestones ($10k ... $1M) are announced in the News.

**Random events and news.** Market events are scheduled a few days ahead and
then fire: crashes, recessions, booms, rate cuts and hikes, oil spikes and
gluts, a tech product launch, a harvest failure, crypto manias and crackdowns,
a dollar slump, housing booms. Each applies an instant price jump (funds and
bonds react through their holdings and the interest rate) and often a
lingering drift/volatility shift. Recessions and booms also scale business
profits. **Your broker (Marcus) tips you off ahead of time** if your
relationship is good, and sometimes passes on rumors that turn out to be
nothing. Personal events: tax refunds, bonuses, an inheritance, surprise raises,
appliance failures, and (rarely) layoffs.

**Decisions.** Now and then a pop-up asks you to choose: car trouble, a rent
increase, a friend's startup, a medical bill, a family loan, a competing job
offer, weekend work, a training course. Options have different costs and some
are gambles. Press a number to choose. Ignore it and the default is picked
after 20 days. Tab hides the pop-up (Tab or `p` brings it back). A manual skip
(`n`) stops as soon as one appears. Catch-up never blocks: pending prompts
resolve themselves.

**Career.** A six-rung ladder (Junior Analyst to Vice President). Every 360
days you get an annual review: a cost-of-living raise, or a promotion if the
dice (and your skill) allow. **You negotiate your salary with your boss**; your
skill and relationship raise how far she'll go. The default offer is a
cost-of-living raise tied to current inflation. A promotion raises expenses by
30% of the raise (lifestyle creep), and your expenses track the price level
every month. `c` opens the career menu: take a course (+1 skill, cooldown), or
job hunt when unemployed (layoffs cut pay to a 50% benefit until you're rehired).

**Negotiation.** One haggling engine runs every deal. The NPC has a hidden
reservation price and a standing offer. Make a counter-offer: if it meets the
reservation they accept; otherwise they concede half the remaining gap but
lose patience, and insulting lowballs cost double. Run them out of patience and
they walk away (relationship drops). Successful deals build the relationship,
which improves their terms and patience next time. The pop-up shows the fair value
but never the reservation. Press `a` to take their number, Esc to walk away.

**Forward contracts** (`f` on an instrument): buy or sell a quantity for
settlement in 30, 60 or 90 days at a strike you negotiate with the broker.
Cash-settled at maturity; needs 10% of the notional in free cash as margin. A
long forward profits if the price ends above your strike.

**Businesses** (`v`): five kinds (food cart ... delivery route) are listed for
sale by two NPC owners and expire after 120 days. Negotiate the price (fair value
is about 18-24 months of profit). Each payday they pay out a noisy profit
that follows the economy. Upgrade one (3 months of profit) for +8% efficiency, up
to 160%, or sell one to an NPC buyer.

**Borrowing** (`m`): borrow up to six months of salary in total (not while
unemployed) at the prevailing interest rate plus 4%. Every payday you pay the
interest plus 2% of the balance; repay early any time. Debt is subtracted from
net worth, so leverage helps only if your investments beat the interest. Missing
payments follows the same shortfall rules as any other bills, so too much debt
can bankrupt you.

**Dividends.** Equities and the index fund now pay a yield on payday, like rent
and bond coupons (AAPL 0.5%, NRGX 4%, GROV 2.5%, IDX1 1.8%). Price drift was
lowered by the same amount, so total expected returns are unchanged, but income
investing now behaves differently from growth investing. Mutual funds include
their holdings' dividends in their NAV.

**Company-specific events.** Earnings misses, a food-safety scandal and a
takeover bid hit individual stocks alongside the macro events (with tips from
the broker as usual).

**Run report and help.** The second view of the Life tab (`w`) is a run report: net worth now
and at its peak, your rank, money in and out (salary, business profit, living
expenses, loan interest), and counts of decisions, deals and forced sales. `?`
opens an in-game help overlay.

## The finance manual (tab 7)

A glossary built into the game for students, with 44 terms in seven groups: Basics,
Trading, Investments, Economy, Government & taxes, Banking & housing, Career &
business. Every entry has four parts: **what it is**, a **worked example** with
numbers from this game, **why it matters and what happens when it changes**, and
**where to see it in the game**.

- Up/Down choose a term, Left/Right (or `[` `]`) jump between groups, PgUp/PgDn move
  eight at a time, Esc returns to where you were.
- **Press `i` on any screen** to open the entry for what you're looking at: on the
  Market tab it explains the selected instrument's asset class (or bid/ask/spread when
  you're viewing the order book), on Portfolio it explains net worth or diversification,
  on World inflation and CPI, on Deals mortgages or business valuation, and so on.
- The examples are not hand-waved: the manual's unit tests recompute the key numbers
  (tax bands, mortgage payments, compounding, bond price moves, spreads, slippage,
  average cost) against the game's own code, so the text can't drift from the rules.

## Seeing your profit when you sell

- The sell prompts (market and limit) show what you paid, an estimate of what this sale
  would earn at the current order book (or your limit price), the percentage, and the
  estimate after the 15% capital gains tax.
- After a sale the status line and the Trades log (Logs tab) record the profit or
  loss against your average cost, before tax. The Portfolio tab lists each holding's
  cost ("Bought at"), value and profit if sold. Each sale's realized profit is stored in
  the database with the trade.

## The economy

A macro model updates once a month (every payday) in `sim-core/src/economy.rs`.
The **World** tab (5) shows it live, with history charts.

- **Inflation and prices.** A CPI (100 at the start) compounds monthly. Inflation
  mean-reverts to 2.5%, rises with growth and energy shortages, and jumps on
  "inflation spike" events. Your living expenses grow with it; salaries only
  catch up at reviews.
- **Growth, unemployment, cycles.** GDP growth follows boom, stimulus and
  recession events; unemployment follows growth (Okun's law). Layoff odds
  scale with unemployment, and businesses' profits scale with the cycle.
- **Central bank.** A Taylor-style rule moves the policy rate a quarter of the way
  to its target each month, from inflation and growth. That rate drives bond
  prices, property, loan costs and savings interest.
- **Supply and demand.** Wheat, oil and steel each have a supply and demand
  index. Demand follows growth; supply recovers on its own and expands when
  prices are high. The gap pushes the commodity's price (shortage up, glut down).
  Oil spikes, gluts and harvest failures move supply.
- **Government.** Progressive income tax withheld from salary (0% to 24k/yr, then
  10/20/30/38%), 15% on dividends, coupons, rent and savings interest, 15% on
  realized capital gains (losses carry forward), 20% on business profit. A
  budget model tracks debt/GDP; each January the government raises or cuts
  its tax stance if debt is high or low, and tax-cut/hike events shift it too.
  Unemployment benefit is 50% of salary.
- **Banking.** Personal loans (policy rate + 4%) and a safe **savings account**
  (`m`, then 3/4) paying the policy rate minus 0.5%. Savings are the first
  thing used to cover a bill shortfall, before any forced sale.

## Housing: rent vs buy

You start out renting; about a third of your expenses is rent. Buying a home is
the big capital goal of the game.

- **Listings** (Deals tab, `w` for Money & home, or `h`): homes from a studio
  (~110k) to a villa (~520k) at today's home price index, with the up-front
  cash, monthly payment and an affordability verdict for each.
- **To buy:** 20% down plus 3% closing costs in cash, a mortgage the bank will
  approve (payment at most 40% of your gross salary, and you must be employed),
  and a price you negotiate with the agent. Unaffordable homes are refused
  before you sit down. The mortgage is a 30-year fixed-rate loan at the policy
  rate plus 2.5%.
- **What changes:** your rent leaves your expenses and you pay the mortgage plus
  2% of the home's value a year in tax and upkeep instead. The home is an asset
  that follows the **home price index**, which rises with inflation and growth and
  falls when interest rates rise. Net worth counts your equity (value minus
  mortgage); you can end up underwater.
- **Selling** (`h`, then 2) costs 6%. A forced sale (85% of value) is the last thing
  tried when you can't cover your bills. Home equity does **not** count toward
  financial independence, since you can't live off the house you live in.
- Renters get landlord decisions (rent increases); owners get repair decisions
  (a leaking roof).

## Assets: rental property, solar and a car

Press `a` (Deals tab, Money & home view) to buy, sell or pay down the mortgage on
extra assets beyond the home you live in. Prices are fixed (no haggling), scaled
by the home price index for property and by the price level for everything else.
You can hold up to eight at once.

| Asset | Price (index 100) | Pays / costs | Notes |
|---|---|---|---|
| Rental Flat | 150k | 5.5% gross rent yield, 1.5% upkeep, 8% of months empty | Buy-to-let mortgage: 25% down, policy rate + 3%, 30 years |
| Storage Units | 45k | 8.5% yield, 2% upkeep, 14% of months empty | Cash only |
| Solar Panels | 14k | saves 85/mo on bills, 0.5% upkeep | Loses ~5% of its value a year |
| Car | 26k | 170/mo to run | Loses ~14% a year |

- **Rent and bills** run every payday: rent (less vacancies), upkeep, mortgage
  interest and principal, and solar's bill savings. Like the rest of your
  personal life, this is paused while you're away.
- **Taxes.** Income, dividend, capital gains and business taxes already existed.
  New here: rental profit (rent minus upkeep and mortgage interest) is taxed at
  15%, and selling a flat or storage units at a profit adds to your realized
  capital gains (a loss carries forward). Losses on a car or solar aren't
  deductible.
- **Net worth** counts your equity (value minus mortgage). Cars and solar are
  left out of financial independence like your home; their running costs (less
  solar's savings) count toward the expenses you need to cover. Rental equity
  does count.
- **Forced sales.** If bills can't be paid, assets are sold (at 85% of value, after
  any mortgage) after businesses and before your home.
- **SQL.** `asset` is a transfer category (like `trade` and `home`): the purchase
  isn't spending. `rent_income`, `asset_costs` and `interest` are ordinary income
  and expenses (migration `0012_assets.sql`).
- **Not done yet** (see roadmap): vacancy streaks, tenant events, property
  renovations, selling to the market at a negotiated price.

## The SQL layer

The database does real work, not just storage (`migrations/`, tests in
`crates/db/tests/integration.rs`).

- **Transactions.** Every cash movement (salary, tax, bills, interest, dividends,
  deals, trades) is a row in `transactions` (append-only, with a generated `month`).
- **Triggers.** `transactions_totals` keeps running totals per category;
  `transactions_immutable` rejects edits to the ledger; `net_worth_record` tracks
  each save's peak net worth; `players_tick_forward` stops time running backwards.
  A CHECK constraint stops negative positions.
- **Stored procedures and functions.** `close_month(player, month)` rolls the
  ledger into `monthly_summary` (called on every save; idempotent).
  `compact_history(player, tick)` thins old price and net worth history (full
  resolution for the last 2,000 days, one point per 30 before; run on load).
  `portfolio_value(player)` and `is_transfer(category)` are SQL functions.
- **Views.** `v_latest_prices`, `v_portfolio` (value, unrealized P&L),
  `v_monthly_cashflow`, `v_net_worth_monthly`, `v_economy_latest`,
  `v_balance_sheet`.
- **Logs.** The Logs tab reads the `transactions` ledger directly: every salary, tax,
  bill, interest payment, dividend, deal, trade and home purchase, newest first.
- **Historical data.** `price_history`, `net_worth_history`, `economy_history`
  (monthly CPI, inflation, GDP, unemployment, policy rate, debt) and
  `monthly_summary`. The Report view's monthly cash flow comes from the SQL
  ledger, not from memory.

Quick queries to try: `SELECT * FROM v_portfolio;`, `SELECT * FROM monthly_summary
ORDER BY month DESC LIMIT 12;`, `SELECT * FROM category_totals;`.

**Rivals.** Elena (steady), Victor (aggressive, leveraged to the market) and Priya
(conservative) are simulated investors whose wealth follows the broad market
plus their own edge and savings. The Life tab shows the leaderboard, and the
News tells you when you overtake or are overtaken.

Everything in this layer lives in `sim-core` (`game.rs`, `events.rs`,
`career.rs`, `negotiation.rs`, `social.rs`) and is stored as one JSON document
per save (`game_state` table). Saves from earlier versions get a fresh game
layer on their next load, with rivals scaled to your net worth.

### Balance check

`cargo run -p sim-core --example balance --release` plays 60 headless games per
strategy (set `MONTHS=240` for twenty years). Last run, ten years, a passive
player who only invests spare cash: about 1 bankruptcy in 60, median net worth
about 210k vs about 180k for the average rival, and few wins before year 20
(taxes and inflation cost roughly 15% of end net worth versus a tax-free world).
All-in crypto has a wide spread (p10 40k, p90 240k). Tune the constants at the
top of `game.rs`, `events.rs` and `career.rs`.

## Screens and controls

The UI is organized into seven tabs. Press **1-7** to jump straight to one; `w`
cycles the views inside a tab. The footer always shows only the keys that matter
on the current tab, and `?` opens the full list.

| Tab | Views (`w`) | What it's for |
|---|---|---|
| **1 Market** | price chart, order book | Browse instruments and trade. The list shows what you hold in green. |
| **2 Portfolio** | allocation, net worth over time | Your holdings and where the money is. |
| **3 Life** | career & goals, run report | Job, financial-independence progress, rivals, monthly cash flow. |
| **4 Deals** | businesses & contracts, money & home | Businesses, listings, forwards, the people you deal with; savings, loan and housing (rent vs buy). |
| **5 World** | (one screen) | Inflation, growth, unemployment, rates, home prices, government, commodity supply and demand. |
| **6 Logs** | transactions, news, trades | Every cash movement (from the SQL ledger), the news feed, and your trades with the profit on each sale. Arrows / PgUp / PgDn scroll. |
| **7 Manual** | (one screen) | The in-game finance manual (see below). |

The header shows the tabs, the day, cash, net worth, the rate and a countdown to the next day, plus a chip only
when something needs attention (debt, open orders, UNEMPLOYED, BILLS 1/3, a
prompt waiting). Narrow terminals (under 100 columns) stack the list above the
chart.

Keys: `Up`/`Down` or `j`/`k` select an instrument - `b` market buy - `s` market
sell - `l` limit order - `x` cancel your orders on it - `f` forward contract -
`c` career menu - `m` money (borrow/repay/savings) - `h` home (buy/sell) - `a` assets (rental, solar, car) - `v` business menu - `t`
advance one day - `n` skip N days - `Tab`/`p` show or hide a pending prompt -
`i` explain what you're looking at (opens the manual) - `?` help - `q`/`Esc` quit. Trading keys pressed on another tab jump you to Market
first. When you're choosing a home, business or asset from a list, move the highlight with `Up`/`Down` (or `j`/`k`) and press `Enter`; pressing the row's number still works as a shortcut. Type digits (and `.`) for quantities and prices, `Enter` confirms, `Esc`
cancels.

## Project layout

```
crates/
  sim-core/  - pure simulation (tick engine, pricing, order book/market,
               portfolio math, and the game layer: events, career,
               negotiation, businesses, rivals). No DB, no I/O; unit tested.
  db/        - sqlx + Postgres: connection, migrations, queries,
               multi-save management. Integration tests in tests/.
  cli/       - the econsim binary: ratatui dashboard.
migrations/  - schema, applied automatically on startup.
```

## Tests

```bash
cargo test -p sim-core                      # no database needed
DATABASE_URL=postgres://user:pass@localhost/postgres cargo test -p db
```

The DB integration tests create and drop their own scratch databases
(`econsim_it_fresh`, `econsim_it_upgrade`), so the role needs
`CREATEDB`; your real data is never touched. They are skipped if
`DATABASE_URL` is unset.

## Roadmap (still deferred)

Other career paths (Trader mode was cut until it has real mechanics of its own),
margin, options and short selling,
NPC-initiated interactions, difficulty settings and scenarios, per-order cancel,
packaging (bundled Postgres, Linux + Windows builds).
