//! Pure simulation logic: no database, no I/O. Everything here operates
//! on plain structs so it can be unit-tested in isolation and reused by
//! any front end (CLI today, GUI later).

pub mod assets;
pub mod career;
pub mod economy;
pub mod engine;
pub mod events;
pub mod game;
pub mod housing;
pub mod instrument;
pub mod market;
pub mod negotiation;
pub mod player;
pub mod portfolio;
pub mod social;

pub use assets::*;
pub use career::*;
pub use economy::*;
pub use engine::*;
pub use events::*;
pub use game::*;
pub use housing::*;
pub use instrument::*;
pub use market::*;
pub use negotiation::*;
pub use player::*;
pub use portfolio::*;
pub use social::*;
