//! Haggling engine, shared by every deal in the game: salary reviews,
//! forward contracts with the broker, buying and selling businesses.
//!
//! The NPC has a hidden *reservation price* (the worst deal it will
//! accept) and an opening *counter*. Each time you make an offer it
//! can't accept, it concedes half the remaining distance toward its
//! reservation, but loses patience; run it out of patience and it walks
//! away. A good relationship shifts the reservation in your favor and
//! buys extra patience. Insulting lowballs burn patience twice as fast.
//!
//! "Price" means whatever is being negotiated: a unit price, a strike,
//! a monthly salary.

use rand::Rng;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    /// You are buying: the NPC wants a high price, you want a low one.
    PlayerBuys,
    /// You are selling (a business, your labor): you want a high price.
    PlayerSells,
}

/// Starting spread an NPC has over fair value, before relationship.
pub const BASE_MARGIN: f64 = 0.10;
/// How far past its reservation the NPC opens.
pub const OPENING_GAP: f64 = 0.10;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Negotiation {
    pub side: Side,
    pub fair: f64,
    /// Hidden from the player in the UI.
    pub reservation: f64,
    /// The NPC's current standing offer/ask.
    pub counter: f64,
    pub patience: u32,
    pub round: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Response {
    Accepted { price: f64 },
    Counter { price: f64, patience: u32, insulted: bool },
    WalkedAway,
}

impl Negotiation {
    /// Standard opening for an NPC with the given relationship (0-100).
    pub fn open<R: Rng>(side: Side, fair: f64, relationship: f64, rng: &mut R) -> Self {
        let rel_adj = (relationship - 50.0) / 50.0 * 0.06;
        let mood = rng.gen_range(-0.02..0.02);
        let m = (BASE_MARGIN - rel_adj + mood).clamp(0.01, 0.20);
        let (reservation, counter) = match side {
            Side::PlayerBuys => (fair * (1.0 + m), fair * (1.0 + m + OPENING_GAP)),
            Side::PlayerSells => (fair * (1.0 - m), fair * (1.0 - m - OPENING_GAP)),
        };
        let patience = if relationship >= 70.0 { 4 } else { 3 };
        Negotiation { side, fair, reservation, counter, patience, round: 0 }
    }

    /// Fully specified opening (used for salary reviews, where the
    /// employer's first offer is a proposal, not a lowball).
    pub fn custom(side: Side, fair: f64, reservation: f64, counter: f64, patience: u32) -> Self {
        Negotiation { side, fair, reservation, counter, patience, round: 0 }
    }

    /// Would the NPC take this price?
    pub fn acceptable(&self, price: f64) -> bool {
        match self.side {
            Side::PlayerBuys => price >= self.reservation - 1e-9,
            Side::PlayerSells => price <= self.reservation + 1e-9,
        }
    }

    pub fn offer(&mut self, price: f64) -> Response {
        if self.acceptable(price) {
            return Response::Accepted { price };
        }
        self.round += 1;
        let insulted = match self.side {
            Side::PlayerBuys => price < self.fair * 0.75,
            Side::PlayerSells => price > self.fair * 1.25,
        };
        self.patience = self.patience.saturating_sub(if insulted { 2 } else { 1 });
        if self.patience == 0 {
            return Response::WalkedAway;
        }
        // Concede half the remaining gap to the reservation price.
        self.counter = self.reservation + (self.counter - self.reservation) * 0.5;
        Response::Counter { price: self.counter, patience: self.patience, insulted }
    }

    /// The price if you take the NPC's current standing offer.
    pub fn accept_counter(&self) -> f64 {
        self.counter
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    fn rng() -> StdRng {
        StdRng::seed_from_u64(5)
    }

    #[test]
    fn opens_on_the_npcs_side_of_fair_value() {
        let b = Negotiation::open(Side::PlayerBuys, 100.0, 50.0, &mut rng());
        assert!(b.counter > b.reservation && b.reservation > 100.0);
        let s = Negotiation::open(Side::PlayerSells, 100.0, 50.0, &mut rng());
        assert!(s.counter < s.reservation && s.reservation < 100.0);
    }

    #[test]
    fn good_relationships_give_better_terms() {
        let friend = Negotiation::open(Side::PlayerBuys, 100.0, 95.0, &mut StdRng::seed_from_u64(1));
        let stranger = Negotiation::open(Side::PlayerBuys, 100.0, 10.0, &mut StdRng::seed_from_u64(1));
        assert!(friend.reservation < stranger.reservation);
        assert!(friend.patience > stranger.patience);
    }

    #[test]
    fn accepts_at_or_beyond_reservation_only() {
        let mut n = Negotiation::open(Side::PlayerBuys, 100.0, 50.0, &mut rng());
        let r = n.reservation;
        assert!(matches!(n.offer(r + 0.01), Response::Accepted { .. }));
        let mut n = Negotiation::open(Side::PlayerBuys, 100.0, 50.0, &mut rng());
        assert!(matches!(n.offer(n.reservation - 0.5), Response::Counter { .. }));
    }

    #[test]
    fn counter_converges_but_never_crosses_reservation() {
        let mut n = Negotiation::open(Side::PlayerBuys, 100.0, 90.0, &mut rng());
        n.patience = 50;
        let low = n.reservation * 0.9;
        let mut last = n.counter;
        for _ in 0..30 {
            if let Response::Counter { price, .. } = n.offer(low) {
                assert!(price <= last + 1e-12);
                assert!(price >= n.reservation - 1e-12);
                last = price;
            }
        }
        assert!((last - n.reservation).abs() < 1e-3);
    }

    #[test]
    fn walks_away_when_patience_runs_out() {
        let mut n = Negotiation::open(Side::PlayerBuys, 100.0, 50.0, &mut rng());
        let low = n.reservation - 1.0;
        let mut results = vec![];
        for _ in 0..5 {
            results.push(n.offer(low));
        }
        assert!(results.contains(&Response::WalkedAway));
    }

    #[test]
    fn insults_cost_double_patience() {
        let mut n = Negotiation::open(Side::PlayerBuys, 100.0, 50.0, &mut rng());
        assert_eq!(n.patience, 3);
        match n.offer(40.0) {
            Response::Counter { patience, insulted, .. } => {
                assert!(insulted);
                assert_eq!(patience, 1);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn seller_side_mirrors_buyer_side() {
        let mut n = Negotiation::open(Side::PlayerSells, 100.0, 50.0, &mut rng());
        assert!(matches!(n.offer(n.reservation - 0.01), Response::Accepted { .. }));
        let mut n = Negotiation::open(Side::PlayerSells, 100.0, 50.0, &mut rng());
        if let Response::Counter { price, .. } = n.offer(n.reservation + 1.0) {
            assert!(price <= n.reservation);
        } else {
            panic!("expected counter");
        }
    }
}
