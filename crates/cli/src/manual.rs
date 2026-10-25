//! The in-game finance manual: a glossary for students. Every entry has a
//! plain definition, a worked example with numbers from this game, what
//! happens to you when the thing changes, and where to see it in the game.
//! The examples are checked against the real game code in the tests below,
//! so the manual can't quietly drift out of date.

use sim_core::AssetClass;

pub struct Entry {
    pub category: &'static str,
    pub term: &'static str,
    /// What it is, in plain words.
    pub what: &'static str,
    /// A worked example with numbers.
    pub example: &'static str,
    /// How a change in it affects you.
    pub matters: &'static str,
    /// Where to see it in the game.
    pub where_in_game: &'static str,
}

pub const CATEGORIES: &[&str] = &[
    "Basics",
    "Trading",
    "Investments",
    "Economy",
    "Government & taxes",
    "Banking & housing",
    "Career & business",
];

pub static ENTRIES: &[Entry] = &[
    // ------------------------------------------------------------ Basics
    Entry {
        category: "Basics",
        term: "Net worth",
        what: "Everything you own minus everything you owe: cash, savings, investments, business value and home equity, less loans.",
        example: "Cash 5,000 + investments 20,000 + savings 3,000 + home equity 40,000 - a 6,000 loan = 62,000.",
        matters: "It is the scoreboard. A falling stock price lowers it, paying down debt raises it without changing what you own. Spending money you earned does not lower it until you spend more than you earn.",
        where_in_game: "Header (Net worth), Portfolio tab, Run report.",
    },
    Entry {
        category: "Basics",
        term: "Cash flow",
        what: "Money coming in each month minus money going out. A positive cash flow (a surplus) is what you can invest.",
        example: "Salary 4,200 - tax 220 - living costs 3,200 = about 780 surplus a month.",
        matters: "A bigger surplus means faster saving. Rent increases, inflation and loan payments shrink it; raises and business profit grow it. A negative cash flow drains your cash and eventually forces you to sell things.",
        where_in_game: "Logs tab (Transactions), Run report (monthly cash flow).",
    },
    Entry {
        category: "Basics",
        term: "Compound growth",
        what: "Earning returns on your earlier returns, not just on what you put in. Time is the main ingredient.",
        example: "10,000 growing 5% a year becomes 26,533 after 20 years (1.05 multiplied by itself 20 times is 2.65), versus 20,000 if growth were not reinvested.",
        matters: "A higher return rate matters more every year. A few percentage points of extra cost (fees, taxes) quietly cost you a large share of the final amount, and starting early beats investing more later.",
        where_in_game: "Net worth chart (Portfolio tab, press w). Reinvest your surplus and compare after a few game years.",
    },
    Entry {
        category: "Basics",
        term: "Return",
        what: "How much an investment gained or lost, as a percentage of what you paid. Total return includes income (dividends, coupons, rent) as well as price changes.",
        example: "You buy at 100, it pays 2 in dividends and the price rises to 105. Total return = (5 + 2) / 100 = 7%.",
        matters: "Compare investments by return and risk, not by price. A cheap-looking share is not better than an expensive one. A 50% loss needs a 100% gain to recover.",
        where_in_game: "Portfolio tab (Profit column) and the sell prompt.",
    },
    Entry {
        category: "Basics",
        term: "Risk and volatility",
        what: "Volatility measures how much a price swings. Risk is the chance of ending up with less than you hoped. Higher expected returns usually come with higher volatility.",
        example: "An index fund with 15% yearly volatility often moves within 15% either side of its average return. Crypto at 70-85% can lose half its value in a year, or double.",
        matters: "If volatility rises (events like a crash raise it for a while), your outcomes spread out in both directions. You need to hold enough safe money to avoid selling at the worst moment.",
        where_in_game: "Instrument info panel on the Market tab, and the volatility changes after market events.",
    },
    Entry {
        category: "Basics",
        term: "Diversification",
        what: "Spreading money across different kinds of investments so one bad event does not sink you.",
        example: "A crash event cuts stocks about 13% and crypto 18%. If 100% of 20,000 is in stocks you lose about 2,600. With half in savings and bonds, the same event costs about 1,300.",
        matters: "Different assets react differently to the same news: rate cuts help stocks and property, a supply shock helps oil, a recession hurts businesses but may lower rates. Holding several smooths the ride.",
        where_in_game: "Allocation view (Portfolio tab). Mutual funds are ready-made diversification.",
    },
    Entry {
        category: "Basics",
        term: "Liquidity",
        what: "How quickly and cheaply you can turn something into cash.",
        example: "Savings can be withdrawn instantly at full value. A home costs 6% to sell, and a forced sale to cover bills gets only 85% of its value. A business forced sale gets 70%.",
        matters: "Illiquid assets can trap you when you need cash. The more of your wealth is locked in a home or business, the more a job loss or bill shock hurts.",
        where_in_game: "Market spreads, home selling costs, forced sales in the news.",
    },
    Entry {
        category: "Basics",
        term: "Emergency fund",
        what: "Cash set aside to cover bills if your income stops, so you never have to sell investments in a bad market.",
        example: "Living costs of 3,200 a month: three months is 9,600. Layoffs last 45-120 days here, and unemployment pays only 50% of your salary.",
        matters: "Without one, a layoff plus a market dip can make you sell at a loss. With bills uncovered for three paydays in a row you go bankrupt. Savings are used first to cover a shortfall.",
        where_in_game: "Money menu (m), Deals tab. Header chips show UNEMPLOYED and BILLS warnings.",
    },
    Entry {
        category: "Basics",
        term: "Financial independence",
        what: "Having enough savings and investments that withdrawing a small yearly share covers your living costs forever, so work becomes optional. This game uses a 5% withdrawal rate: about 20 times your annual costs.",
        example: "Costs of 3,000 a month are 36,000 a year. You need 36,000 / 5% = 720,000 in savings and investments. Your home does not count, since you live in it.",
        matters: "Higher living costs (inflation, lifestyle creep, a mortgage) raise the target. Better returns and a higher savings rate shorten the road. It is how you win the game.",
        where_in_game: "FI% in the header, Life tab.",
    },
    // ----------------------------------------------------------- Trading
    Entry {
        category: "Trading",
        term: "Bid, ask and spread",
        what: "The bid is the highest price a buyer will pay; the ask is the lowest a seller will take. The gap is the spread, the cost of trading right now.",
        example: "A stock is worth 180. Market makers quote an ask of about 180.27 and a bid of about 179.73. Buying then immediately selling loses about 0.54 per share (0.3%). Crypto's spread is wider: a round trip costs about 1.2%.",
        matters: "A wide spread means frequent trading is expensive. Real estate trusts (about 2% round trip) and crypto punish short-term trading; index funds and currencies are cheap to trade.",
        where_in_game: "Order book view (Market tab, press w).",
    },
    Entry {
        category: "Trading",
        term: "Market order",
        what: "An order to buy or sell immediately at whatever price is available.",
        example: "Buying 20 shares at an ask of 180.27 costs about 3,605. A big order eats several price levels and gets a worse average price (slippage).",
        matters: "It is fast and certain but you pay the spread, and large orders pay more. In a volatile market the price you see may move before you act.",
        where_in_game: "b (buy) and s (sell) on the Market tab.",
    },
    Entry {
        category: "Trading",
        term: "Limit order",
        what: "An order to buy no higher than, or sell no lower than, a price you choose. It waits in the book until the price comes to you.",
        example: "A stock trades at 100. You place a limit buy at 95 for 10 shares and 950 of your cash is reserved. If the price falls to 95 on some later day, you buy at 95 or better.",
        matters: "You control the price but may never get filled. While it waits, the reserved cash cannot be used for anything else, so tying up money in far-off limits costs flexibility.",
        where_in_game: "l on the Market tab. Open orders show in the header and order book.",
    },
    Entry {
        category: "Trading",
        term: "Slippage and market depth",
        what: "Depth is how much can be traded at each price level. Slippage is the worse average price you get when your order is bigger than the best level.",
        example: "A stock has about 8,000 on offer per level, each level 0.1% higher. A 20,000 buy fills 8,000 + 8,000 + 4,000 across three levels and averages about 0.08% above the best ask.",
        matters: "Thin markets (crypto, property) move against big orders. Split large trades over several days, or use limit orders. Savings has unlimited depth and no spread.",
        where_in_game: "The sell and buy status messages show the slippage versus fair price.",
    },
    Entry {
        category: "Trading",
        term: "Average cost, profit and loss",
        what: "Your average cost is the average price you paid per unit. Unrealized profit is the gain on paper; realized profit is locked in when you sell.",
        example: "Buy 10 at 100, then 10 at 120: average cost 110. Sell all 20 at 130: profit = 20 x (130 - 110) = 400. Capital gains tax of 15% takes 60, leaving 340.",
        matters: "Only realized gains are taxed, and losses offset gains. Selling a winner triggers tax; selling a loser can reduce it. Do not hold a loser just to avoid admitting the loss.",
        where_in_game: "Portfolio tab (Bought at, Profit), sell prompts, Trades log.",
    },
    Entry {
        category: "Trading",
        term: "Forward contract",
        what: "An agreement to buy or sell something at a fixed price on a future date. Here it settles in cash for the difference.",
        example: "You go long 10 barrels of oil at a strike of 100 for 30 days. If oil is 120 at settlement you receive 10 x 20 = 200; if it is 90 you pay 100. You must hold 10% of the value as margin.",
        matters: "It lets you bet on a price or lock one in (a hedge) with little cash, but losses can be large. A higher interest rate raises the forward price, since waiting costs money.",
        where_in_game: "f on the Market tab; contracts on the Deals tab.",
    },
    // -------------------------------------------------------- Investments
    Entry {
        category: "Investments",
        term: "Stocks (equities)",
        what: "A share is a small ownership stake in a company. You profit from price rises and from dividends.",
        example: "100 shares of a 65 stock cost 6,500. If earnings disappoint and it falls 10%, you lose 650; a takeover bid might add 16%.",
        matters: "Stocks rise with growth and low interest rates and fall in recessions and rate hikes. A single company can also fail on its own news (earnings misses, scandals), which is why people diversify.",
        where_in_game: "AAPL, NRGX, GROV on the Market tab.",
    },
    Entry {
        category: "Investments",
        term: "Dividends and yield",
        what: "A dividend is a share of profit a company pays out. The yield is the yearly dividend as a percentage of the price.",
        example: "NRGX yields 4%. 100 shares worth 6,500 pay about 260 a year, roughly 21 a month before the 15% tax on investment income.",
        matters: "A high yield gives steady income but usually slower price growth. Income is taxed when received, so a high-yield investment is less tax-efficient than a growth one.",
        where_in_game: "Instrument info panel; dividends appear in the transactions log on paydays.",
    },
    Entry {
        category: "Investments",
        term: "Index fund",
        what: "A fund that holds a broad slice of the market, so you own a bit of everything instead of picking winners.",
        example: "IDX1 has about 15% volatility versus 28% for AAPL and 35% for NRGX. It tracks the market, with a 1.8% dividend yield.",
        matters: "It rises and falls with the whole economy. Good for beginners because no single company can sink it, though a market-wide crash still hurts.",
        where_in_game: "IDX1 on the Market tab. Your rivals' wealth tracks it too.",
    },
    Entry {
        category: "Investments",
        term: "Mutual fund, NAV and expense ratio",
        what: "A fund that holds a basket of other investments. Its price (net asset value, NAV) is the value of the basket per share. The expense ratio is a yearly fee taken out of the fund.",
        example: "BALF holds 45% index fund, 30% government bonds, 15% corporate bonds and 10% property. With a 0.75% expense ratio, 10,000 in the fund pays about 75 a year in fees.",
        matters: "The fund moves with its holdings, minus fees. A higher fee quietly reduces your compound growth every year, so compare funds on fees as well as holdings.",
        where_in_game: "BALF, GRWF, REAL; basket shown in the info panel.",
    },
    Entry {
        category: "Investments",
        term: "Bonds, coupons and duration",
        what: "A bond is a loan you make to a government or company. It pays a fixed coupon (interest) and returns the face value at the end. Duration measures how sensitive its price is to interest rates.",
        example: "GOVB has a face value of 1,000 and a 4% coupon: about 40 a year (about 3.3 a month before tax). Its duration is 7 years.",
        matters: "When rates rise, existing bonds with lower coupons become less attractive and their prices fall, about duration x the rate change. A one point rate rise cuts GOVB roughly 7%, to about 930. Rate cuts raise bond prices.",
        where_in_game: "GOVB and CORP; the policy rate is in the header and on the World tab.",
    },
    Entry {
        category: "Investments",
        term: "Real estate trusts",
        what: "Listed companies that own property and pay out rent. You get rental income plus any change in property values.",
        example: "PROP yields 3.5%: 10,000 invested pays about 350 a year in rent. Its growth drops about 1.5 percentage points a year for every point the policy rate rises above 4%.",
        matters: "Property is sensitive to interest rates (mortgages get costlier) and is thinly traded, so the spread is wide: about 2% to buy and sell again.",
        where_in_game: "PROP, HOME on the Market tab.",
    },
    Entry {
        category: "Investments",
        term: "Commodities",
        what: "Raw materials like oil, wheat and steel. Their price is driven by supply and demand rather than company profits.",
        example: "An oil supply disruption cuts supply by 20% of normal. That jumps the price about 18% at once and keeps pushing it up while the shortage lasts. High prices coax out more supply, which slowly ends the shortage.",
        matters: "Commodities rise with strong growth and shortages, and often rise with inflation. They pay no income, so their return is only price change.",
        where_in_game: "WHET, OIL, STEE; the supply and demand gauges on the World tab.",
    },
    Entry {
        category: "Investments",
        term: "Currencies (forex)",
        what: "The price of one currency in another. A pair like EURUSD is how many dollars one euro costs.",
        example: "EURUSD at 1.10: 1,000 euros cost 1,100 dollars. If it rises to 1.20, your euros are worth 9% more in dollars.",
        matters: "Currencies tend to wander around a long-run level rather than trend, so they are poor long-term growth investments but cheap to trade. Events like a dollar slump move them quickly.",
        where_in_game: "EURUSD, GBPUSD, USDJPY.",
    },
    Entry {
        category: "Investments",
        term: "Crypto",
        what: "Digital assets with no earnings or cash flows. Prices depend on sentiment and adoption.",
        example: "BTC at 60,000 with 70% yearly volatility can plausibly be 30,000 or 100,000 a year later. A crackdown event cuts crypto about 30% overnight; a mania adds 25%.",
        matters: "High potential and high risk. Position size is the key control: money you cannot afford to lose should not be here. The spread (about 1.2% round trip) also makes frequent trading costly.",
        where_in_game: "BTC, ETH; fractional quantities allowed.",
    },
    Entry {
        category: "Investments",
        term: "Savings account",
        what: "Money held at a bank that earns interest, safe from market swings. It is an account with a balance, not something you buy and sell: no price, no spread, and it compounds every month.",
        example: "10,000 at a 4% policy rate earns 3.5% (policy rate minus 0.5%): 350 a year, or about 297 after the 15% tax on interest.",
        matters: "When the central bank raises rates, savings pay more; when it cuts, less. If inflation is higher than your interest, your savings lose purchasing power even though the balance grows.",
        where_in_game: "Money menu (m, then 3 or 4); Money & home view on the Deals tab, which also shows how it compounds.",
    },
    // ----------------------------------------------------------- Economy
    Entry {
        category: "Economy",
        term: "Inflation and CPI",
        what: "Inflation is the yearly rise in prices. The CPI (consumer price index) tracks the overall price level; it starts at 100.",
        example: "CPI 103 means prices are 3% higher than at the start. At 3% inflation your 3,200 living costs become about 3,296 after a year, while 10,000 in cash buys about 3% less.",
        matters: "Higher inflation raises your costs faster than your salary (raises only come at reviews) and pushes the central bank to raise rates, which lowers bond and property prices. Real assets and commodities tend to hold up better.",
        where_in_game: "World tab; Year in review in the News.",
    },
    Entry {
        category: "Economy",
        term: "GDP growth and the business cycle",
        what: "GDP growth is how fast the whole economy is expanding. Economies cycle through booms and recessions.",
        example: "Trend growth here is 2.5%. A recession event pulls growth down about 2 points; a boom lifts it 1.5 points. Business profits scale with the cycle (a recession can cut them by a third).",
        matters: "Weak growth raises unemployment, cuts company and business profits, and usually lowers inflation and interest rates. Stronger growth lifts stocks, commodity demand and your business income.",
        where_in_game: "World tab; market events in the News.",
    },
    Entry {
        category: "Economy",
        term: "Unemployment",
        what: "The share of people looking for work who cannot find it.",
        example: "At the normal 5% layoffs are rare. In a recession it might rise to 8%, making a layoff about 60% more likely. Unemployment pays 50% of your salary and you cannot borrow while out of work.",
        matters: "High unemployment means job risk for you and weak demand for everything else. It is a signal to build your emergency fund and avoid taking on new debt.",
        where_in_game: "World tab; the UNEMPLOYED chip in the header.",
    },
    Entry {
        category: "Economy",
        term: "Central bank and policy rate",
        what: "The central bank sets the base interest rate. It raises rates to cool inflation and cuts them to support growth.",
        example: "If inflation jumps from 2.5% to 5%, the rule pushes the policy rate up by roughly 3.75 points over time. A mortgage at policy + 2.5% and a loan at policy + 4% both get more expensive.",
        matters: "Rates are the most important price in finance. Higher rates: lower bond prices, lower property prices, costlier loans, better savings returns. Lower rates do the reverse and boost stocks.",
        where_in_game: "Header (Rate), World tab, the Rate hike and Rate cut events.",
    },
    Entry {
        category: "Economy",
        term: "Supply and demand",
        what: "Prices rise when buyers want more than sellers offer, and fall when there is more on offer than wanted.",
        example: "Oil demand 1.05 against supply 0.90 is a shortage of 0.15, which pushes the oil price up at roughly 15 percentage points a year faster than normal. A glut works in reverse.",
        matters: "A shortage rewards producers and holders; high prices then bring new supply and the price eases. Watch for events that shock supply (disruptions, harvest failures) and growth, which drives demand.",
        where_in_game: "Commodity gauges on the World tab (SHORTAGE, GLUT, balanced).",
    },
    Entry {
        category: "Economy",
        term: "Home price index",
        what: "A measure of how house prices move, starting at 100.",
        example: "At an index of 120 a home that cost 100,000 at the start would cost 120,000. Each extra point of interest rate takes about one percentage point off the yearly growth of house prices.",
        matters: "Prices rise with inflation and growth and fall when rates are high. Buying when mortgage rates are low and prices are soft is the best case; buying at a peak can leave you underwater.",
        where_in_game: "World tab, listing prices on the Money & home screen.",
    },
    Entry {
        category: "Economy",
        term: "Market events",
        what: "Shocks like crashes, booms, rate decisions and company news that move prices and the economy.",
        example: "A crash event: stocks -13% at once, then higher volatility and weaker drift for about 60 days. Brokers who like you may warn you 4 to 14 days ahead.",
        matters: "Events cannot be predicted, so the defense is not to be wiped out by any single one: diversify, keep cash and avoid debt you cannot carry.",
        where_in_game: "Logs tab (News), status line.",
    },
    // -------------------------------------------------- Government & taxes
    Entry {
        category: "Government & taxes",
        term: "Income tax (progressive)",
        what: "A tax on your salary where higher slices of income are taxed at higher rates. Only the part inside each band is taxed at that band's rate.",
        example: "A yearly salary of 50,400: nothing on the first 24,000, then 10% on the next 26,400 = 2,640 a year, or 220 a month withheld. Earning more only raises the rate on the extra slice.",
        matters: "A raise is never fully yours: about a fifth of the next slice goes to tax once you pass 60,000. Governments can raise or cut the whole schedule, which changes your take-home pay.",
        where_in_game: "Transactions log (Income tax withheld); World tab shows the current tax stance.",
    },
    Entry {
        category: "Government & taxes",
        term: "Capital gains and investment income tax",
        what: "Profit from selling an investment is taxed at 15%, and dividends, coupons, rent and savings interest are also taxed at 15%. Business profit is taxed at 20%.",
        example: "You sell shares for a 1,000 profit and owe 150. If you later realize a 400 loss, it is carried forward and cuts the next gain's tax.",
        matters: "Tax drag reduces compound growth. Holding investments rather than trading often keeps more of your return, and harvesting losses can offset gains.",
        where_in_game: "Sell prompt (after-tax estimate), Transactions log.",
    },
    Entry {
        category: "Government & taxes",
        term: "Government budget and tax stance",
        what: "The government collects taxes and spends; if it runs deficits its debt grows. It adjusts taxes accordingly.",
        example: "If public debt stays above 100% of GDP, each January the government raises tax rates by 5%. If it falls below 70% it cuts them by 5%.",
        matters: "Fiscal policy changes your take-home pay and the business environment. A recession both lowers tax receipts and raises spending, widening deficits.",
        where_in_game: "World tab (Government section); tax events in the News.",
    },
    // -------------------------------------------------- Banking & housing
    Entry {
        category: "Banking & housing",
        term: "Interest and loans",
        what: "Interest is the price of using someone else's money. You pay it when you borrow and earn it when you save.",
        example: "Borrow 6,000 at 8% (policy 4% + 4%): interest 40 a month, plus 2% of the balance (120) repaid: 160 a month. You can borrow at most six months of salary in total, and not while unemployed.",
        matters: "Borrowing only helps if your investments beat the interest after tax. A rate rise makes new loans costlier. Too much debt plus a layoff is the fastest road to bankruptcy.",
        where_in_game: "Money menu (m, then 1 or 2); Debt chip in the header.",
    },
    Entry {
        category: "Banking & housing",
        term: "Mortgage",
        what: "A long loan used to buy a home, repaid in monthly instalments over 30 years. Early payments are mostly interest.",
        example: "A 250,000 home: 20% down (50,000) plus 3% closing costs (7,500) = 57,500 in cash. The 200,000 loan at 6% costs about 1,199 a month, plus roughly 417 a month for upkeep and tax (2% of value a year).",
        matters: "A higher mortgage rate raises your payment sharply, and the bank only lends if the payment is under 40% of your gross salary. The loan rate is fixed once you buy, so buying when rates are low locks in the saving.",
        where_in_game: "h on any tab; listings on the Deals tab (w for Money & home).",
    },
    Entry {
        category: "Banking & housing",
        term: "Home equity and being underwater",
        what: "Equity is what your home is worth minus the mortgage you still owe. If it is negative, you are underwater.",
        example: "You buy for 250,000 with a 200,000 loan: equity 50,000. If prices fall 20% the home is worth 200,000 and your equity is 0. Fall further and you owe more than it is worth.",
        matters: "You build equity by repaying the loan and if prices rise. A price fall wipes out equity first. Selling costs 6%, and you cannot sell if you cannot cover the shortfall.",
        where_in_game: "Money & home screen; counted in net worth but not in FI.",
    },
    Entry {
        category: "Banking & housing",
        term: "Renting versus buying",
        what: "Renting costs rent and no ownership. Owning costs mortgage interest, upkeep and tax, requires cash up front, and builds equity.",
        example: "Rent is 1,200 a month. Owning a 185,000 home with a 148,000 mortgage at 6% costs about 887 for the payment plus 308 for upkeep and tax = 1,195 a month, and about 147 of the first payment repays principal, which you keep.",
        matters: "Buying wins when prices rise, you stay long enough to recover the 9% round-trip costs (closing plus selling), and mortgage rates are reasonable. Renting wins when you need flexibility or prices are falling.",
        where_in_game: "Money & home screen compares the costs.",
    },
    Entry {
        category: "Banking & housing",
        term: "Rental property and landlord tax",
        what: "Property you own and let to tenants. You receive rent, pay upkeep, tax and any mortgage interest, and are taxed on the profit. Months with no tenant pay no rent but the costs go on.",
        example: "A 150,000 flat bought for cash at a 5.5% yield earns 687 a month in rent. Upkeep at 1.5% of its value is 187 a month, so the taxable profit is 500 and the 15% tax takes 75, leaving 425 a month (about 3.4% a year on the price). A mortgage adds interest to the costs, which can turn the profit negative in the early years.",
        matters: "Higher rates raise mortgage interest and push prices down; tenants leaving cost a month of rent; a fall in the home price index lowers your equity. Selling at a profit adds to your capital gains tax bill, and a loss carries forward.",
        where_in_game: "Money & home screen (Assets section, press a); Logs tab shows rent and tax lines; run report shows rent received.",
    },
    Entry {
        category: "Banking & housing",
        term: "Depreciation, cars and solar panels",
        what: "Depreciation is the value an asset loses with age and use. Unlike property, a car or a set of solar panels is worth less every year, and you cannot deduct the loss.",
        example: "A 26,000 car loses 14% of its value a year, so after 3 years it is worth about 16,537, and it costs about 170 a month to insure, fuel and service. Solar panels lose about 5% a year but cut your electricity bill by about 85 a month.",
        matters: "A depreciating asset is a purchase, not an investment: it lowers your net worth over time, so buy it for the use you get, not for the resale. That is why the game leaves cars and solar out of financial independence.",
        where_in_game: "Money & home screen (Assets section, press a); the car and solar rows show their value falling every month.",
    },
    // ------------------------------------------------ Career & business
    Entry {
        category: "Career & business",
        term: "Gross pay, net pay and lifestyle creep",
        what: "Gross pay is your salary before tax; net pay is what you take home. Lifestyle creep is spending that rises as your income rises.",
        example: "A promotion raises your salary by 700. Tax takes some, and the game adds 30% of the raise (210) to your monthly expenses, so your saving rises by much less than 700.",
        matters: "If every raise is spent, your surplus (and your progress to independence) never grows. Keeping expenses flat while income rises is the quickest way to build wealth.",
        where_in_game: "Review pop-ups; Transactions log.",
    },
    Entry {
        category: "Career & business",
        term: "Salary negotiation",
        what: "Employers have a maximum they will pay (their reservation price) that you cannot see. Asking too much wastes their patience; asking too little leaves money on the table.",
        example: "Your boss offers 4,350. Their real limit might be 4,520. A counter at 4,500 works, a counter at 6,000 costs patience, and a second insulting one ends the talks and the relationship.",
        matters: "Higher skill and a better relationship raise their limit and patience. A raise compounds: it lifts every future paycheck.",
        where_in_game: "Annual review pop-up; train in the career menu (c).",
    },
    Entry {
        category: "Career & business",
        term: "Business valuation and profit",
        what: "A small business is worth a multiple of its yearly or monthly profit. Profit is noisy and follows the economy.",
        example: "A cafe earning about 900 a month sells for about 22 months of profit, around 19,800. After the 20% profit tax you keep about 720 a month if it earns 900.",
        matters: "Recessions cut profit, and boom years raise it. Upgrading raises efficiency (cost: three months of profit for +8%). Over-paying kills the return: check payback time (price divided by monthly profit).",
        where_in_game: "Deals tab (listings and your businesses); v on any tab.",
    },
];

/// Index of the entry with this exact term.
pub fn find(term: &str) -> Option<usize> {
    ENTRIES.iter().position(|e| e.term == term)
}

/// The manual entry that explains an asset class.
pub fn term_for_class(class: AssetClass) -> &'static str {
    match class {
        AssetClass::Equity => "Stocks (equities)",
        AssetClass::IndexFund => "Index fund",
        AssetClass::MutualFund => "Mutual fund, NAV and expense ratio",
        AssetClass::Savings => "Savings account",
        AssetClass::Commodity => "Commodities",
        AssetClass::Bond => "Bonds, coupons and duration",
        AssetClass::RealEstate => "Real estate trusts",
        AssetClass::Forex => "Currencies (forex)",
        AssetClass::Crypto => "Crypto",
    }
}

/// First entry of the next (or previous) category, wrapping around.
pub fn jump_category(index: usize, forward: bool) -> usize {
    let n = CATEGORIES.len();
    let c = CATEGORIES.iter().position(|c| *c == ENTRIES[index].category).unwrap_or(0);
    let target = if forward { (c + 1) % n } else { (c + n - 1) % n };
    ENTRIES.iter().position(|e| e.category == CATEGORIES[target]).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::*;

    #[test]
    fn entries_are_complete_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for e in ENTRIES {
            assert!(seen.insert(e.term), "duplicate term {}", e.term);
            for (name, text) in [("what", e.what), ("example", e.example), ("matters", e.matters), ("where", e.where_in_game)] {
                assert!(text.len() > 20, "{} has a thin {name}", e.term);
                assert!(!text.contains('{') && !text.contains('"'), "{} {name} has odd characters", e.term);
            }
            assert!(CATEGORIES.contains(&e.category), "{} has an unknown category", e.term);
        }
        assert!(ENTRIES.len() >= 38);
    }

    #[test]
    fn categories_are_contiguous_and_in_order() {
        let mut order: Vec<&str> = vec![];
        for e in ENTRIES {
            if order.last() != Some(&e.category) {
                assert!(!order.contains(&e.category), "{} is split into two groups", e.category);
                order.push(e.category);
            }
        }
        assert_eq!(order, CATEGORIES, "every category used, in listing order");
    }

    #[test]
    fn every_asset_class_has_an_entry() {
        for c in AssetClass::ALL {
            assert!(find(term_for_class(c)).is_some(), "{c:?}");
        }
    }

    #[test]
    fn jumping_between_categories_works_both_ways() {
        let start = find("Net worth").unwrap();
        let next = jump_category(start, true);
        assert_eq!(ENTRIES[next].category, "Trading");
        assert_eq!(jump_category(next, false), start);
        // wraps around
        assert_eq!(ENTRIES[jump_category(start, false)].category, "Career & business");
        let last_cat_first = find("Gross pay, net pay and lifestyle creep").unwrap();
        assert_eq!(jump_category(last_cat_first, true), 0);
    }

    fn entry(term: &str) -> &'static Entry {
        &ENTRIES[find(term).unwrap_or_else(|| panic!("{term}"))]
    }

    // ---- the worked examples must match what the game really does ----

    #[test]
    fn asset_examples_match_the_game() {
        let flat = &ASSET_SPECS[0];
        let rent = flat.base_price * flat.gross_yield / 12.0;
        let upkeep = flat.base_price * flat.upkeep_rate / 12.0;
        assert!((rent - 687.5).abs() < 1e-9);
        assert!((upkeep - 187.5).abs() < 1e-9);
        assert!(((rent - upkeep) * INVESTMENT_INCOME_RATE - 75.0).abs() < 1e-9);
        let e = entry("Rental property and landlord tax");
        assert!(e.example.contains("687") && e.example.contains("187") && e.example.contains("75"));
        let mut car = OwnedAsset { id: 1, kind: 3, bought_tick: 0, price_paid: 26_000.0, hpi_at_purchase: 100.0, value: 26_000.0, mortgage: None };
        for _ in 0..36 {
            car.revalue(100.0);
        }
        assert!((car.value - 16_537.456).abs() < 1.0, "{}", car.value);
        assert_eq!(ASSET_SPECS[3].fixed_monthly_cost, 170.0);
        assert_eq!(ASSET_SPECS[2].monthly_saving, 85.0);
        assert!(entry("Depreciation, cars and solar panels").example.contains("16,537"));
    }

    #[test]
    fn tax_example_matches_the_tax_code() {
        assert!((annual_income_tax(50_400.0, 1.0) - 2_640.0).abs() < 1e-9);
        assert!((monthly_withholding(4_200.0, 1.0) - 220.0).abs() < 1e-9);
        assert!(entry("Income tax (progressive)").example.contains("2,640"));
        assert!(entry("Income tax (progressive)").example.contains("220 a month"));
        assert_eq!(CAPITAL_GAINS_RATE, 0.15);
        assert_eq!(INVESTMENT_INCOME_RATE, 0.15);
        assert_eq!(BUSINESS_PROFIT_RATE, 0.20);
    }

    #[test]
    fn compounding_and_independence_examples_are_right() {
        assert_eq!((10_000.0_f64 * 1.05_f64.powi(20)).round() as i64, 26_533);
        assert!(entry("Compound growth").example.contains("26,533"));
        assert_eq!(FI_WITHDRAWAL_RATE, 0.05);
        assert!((36_000.0 / FI_WITHDRAWAL_RATE - 720_000.0_f64).abs() < 1e-6);
        assert!(entry("Financial independence").example.contains("720,000"));
    }

    #[test]
    fn mortgage_and_housing_examples_are_right() {
        let pmt = mortgage_payment(200_000.0, 0.06);
        assert_eq!(pmt.round() as i64, 1_199);
        assert!(entry("Mortgage").example.contains("1,199"));
        assert!((250_000.0 * (DOWN_PAYMENT + CLOSING_COST) - 57_500.0_f64).abs() < 1e-6);
        assert!(entry("Mortgage").example.contains("57,500"));
        assert_eq!((250_000.0 * UPKEEP_RATE / 12.0_f64).round() as i64, 417);
        assert_eq!(MAX_PAYMENT_TO_INCOME, 0.40);
        // Rent vs buy: a 185k home, 80% mortgage at 6%.
        let loan = 185_000.0 * 0.8;
        let payment = mortgage_payment(loan, 0.06);
        assert_eq!(payment.round() as i64, 887);
        assert_eq!((185_000.0 * UPKEEP_RATE / 12.0_f64).round() as i64, 308);
        assert_eq!((payment - loan * 0.06 / 12.0).round() as i64, 147);
        let text = entry("Renting versus buying").example;
        assert!(text.contains("887") && text.contains("308") && text.contains("1,195") && text.contains("147"));
        // The "9% round trip" in the text: closing costs plus selling costs.
        assert!((SELLING_COST + CLOSING_COST - 0.09_f64).abs() < 1e-12);
        // Equity example: 250k home, 200k loan, prices -20%.
        assert_eq!(250_000.0 * 0.8 - 200_000.0, 0.0);
    }

    #[test]
    fn loan_savings_and_tax_on_interest_examples_are_right() {
        // 6,000 at 8%: 40 interest + 2% of principal = 160.
        assert_eq!(6_000.0 * 0.08 / 12.0, 40.0);
        assert_eq!(6_000.0 * LOAN_MIN_PAYMENT_RATE, 120.0);
        assert_eq!(LOAN_SPREAD, 0.04);
        assert_eq!(MAX_LOAN_MULTIPLE, 6.0);
        // 10,000 at 4% policy - 0.5% = 3.5% = 350, 297.5 after 15% tax.
        assert_eq!(SAVINGS_SPREAD, 0.005);
        assert!((10_000.0 * (0.04 - SAVINGS_SPREAD) * (1.0 - INVESTMENT_INCOME_RATE) - 297.5).abs() < 1e-9);
        assert!(entry("Savings account").example.contains("297"));
    }

    #[test]
    fn bond_example_is_in_the_right_ballpark() {
        let at_4 = bond_price(1000.0, 0.04, 7.0, 0.04);
        let at_5 = bond_price(1000.0, 0.04, 7.0, 0.05);
        assert!((at_4 - 1000.0).abs() < 1.0);
        assert!((at_5 - 930.0).abs() < 15.0, "GOVB at 5% rates: {at_5}");
        assert!(at_5 < at_4);
        assert!(entry("Bonds, coupons and duration").example.contains("3.3"));
        assert!((1000.0 * 0.04 * 30.0 / 365.0 - 3.29_f64).abs() < 0.01);
    }

    #[test]
    fn market_structure_examples_are_right() {
        use sim_core::{maker_params, OrderBook};
        let p = maker_params(AssetClass::Equity);
        let book = OrderBook::quote(180.0, &p);
        let (ask, bid) = (book.best_ask().unwrap(), book.best_bid().unwrap());
        assert!((ask - 180.27).abs() < 0.01 && (bid - 179.73).abs() < 0.01, "{ask} {bid}");
        assert!(((ask - bid) / 180.0 - 0.003).abs() < 0.0005);
        let crypto = maker_params(AssetClass::Crypto);
        assert!((crypto.half_spread * 2.0 - 0.012).abs() < 1e-9);
        let property = maker_params(AssetClass::RealEstate);
        assert!((property.half_spread * 2.0 - 0.02).abs() < 1e-9);
        assert_eq!(p.level_notional, 8_000.0);
        assert!((p.level_step - 0.001).abs() < 1e-12);
    }

    #[test]
    fn slippage_example_is_right() {
        use sim_core::{maker_params, OrderBook};
        let mut book = OrderBook::quote(180.0, &maker_params(AssetClass::Equity));
        let best = book.best_ask().unwrap();
        let (filled, cost) = book.take_buy(20_000.0 / 180.0, None);
        assert!((filled - 20_000.0 / 180.0).abs() < 1e-9);
        let premium = cost / filled / best - 1.0;
        assert!((premium - 0.0008).abs() < 0.0002, "{premium}");
        assert!(entry("Slippage and market depth").example.contains("0.08%"));
    }

    #[test]
    fn average_cost_example_is_right() {
        let mut pf = Portfolio::default();
        pf.buy(1, 10.0, 100.0);
        pf.buy(1, 10.0, 120.0);
        assert_eq!(pf.positions[&1].avg_cost, 110.0);
        let proceeds = pf.sell(1, 20.0, 130.0).unwrap();
        assert_eq!(proceeds, 2_600.0);
        assert_eq!(pf.realized_gains, 400.0);
        assert!((pf.realized_gains * (1.0 - CAPITAL_GAINS_RATE) - 340.0).abs() < 1e-9);
    }

    #[test]
    fn fund_fee_and_property_yield_examples_are_right() {
        assert!((10_000.0 * 0.0075 - 75.0_f64).abs() < 1e-9);
        assert!((10_000.0 * 0.035 - 350.0_f64).abs() < 1e-9);
        assert_eq!(UPKEEP_RATE, 0.02);
        assert_eq!(FORCED_SALE_FACTOR, 0.85);
        assert_eq!(FIRE_SALE_FACTOR, 0.70);
        assert!(entry("Mutual fund, NAV and expense ratio").what.contains("expense ratio"));
    }

    #[test]
    fn lifestyle_creep_and_inflation_examples_are_right() {
        assert_eq!(LIFESTYLE_CREEP, 0.30);
        assert!((700.0 * LIFESTYLE_CREEP - 210.0_f64).abs() < 1e-9);
        assert!((3_200.0 * 1.03_f64 - 3_296.0).abs() < 1e-9);
        assert_eq!(BENEFIT_RATE, 0.50);
        assert_eq!(INFLATION_TARGET, 0.025);
    }
}
