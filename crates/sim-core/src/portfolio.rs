use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct Position {
    pub instrument_id: i32,
    pub quantity: f64,
    pub avg_cost: f64,
}

#[derive(Debug, Clone, Default)]
pub struct Portfolio {
    pub positions: HashMap<i32, Position>,
    /// Profit (or loss) realized by sales since the game layer last
    /// collected it for capital gains tax.
    pub realized_gains: f64,
}

impl Portfolio {
    /// Records a buy at an already-determined execution price. Where
    /// that price comes from (order book walk, resting-order fill) is
    /// the `market` module's business, not the portfolio's.
    pub fn buy(&mut self, instrument_id: i32, quantity: f64, price: f64) -> f64 {
        let cost = quantity * price;
        let pos = self.positions.entry(instrument_id).or_insert(Position {
            instrument_id,
            quantity: 0.0,
            avg_cost: 0.0,
        });
        let total_cost = pos.avg_cost * pos.quantity + cost;
        pos.quantity += quantity;
        pos.avg_cost = if pos.quantity > 0.0 {
            total_cost / pos.quantity
        } else {
            0.0
        };
        cost
    }

    /// Returns proceeds, or `None` if the player doesn't hold enough
    /// to sell that quantity.
    pub fn sell(&mut self, instrument_id: i32, quantity: f64, price: f64) -> Option<f64> {
        let pos = self.positions.get_mut(&instrument_id)?;
        if pos.quantity < quantity {
            return None;
        }
        let gain = (price - pos.avg_cost) * quantity;
        pos.quantity -= quantity;
        if pos.quantity <= 0.0001 {
            self.positions.remove(&instrument_id);
        }
        self.realized_gains += gain;
        Some(quantity * price)
    }

    /// Units currently held of one instrument (0 if none).
    pub fn quantity_of(&self, instrument_id: i32) -> f64 {
        self.positions.get(&instrument_id).map_or(0.0, |p| p.quantity)
    }

    pub fn market_value(&self, prices: &HashMap<i32, f64>) -> f64 {
        self.positions
            .values()
            .map(|p| p.quantity * prices.get(&p.instrument_id).copied().unwrap_or(0.0))
            .sum()
    }
}
