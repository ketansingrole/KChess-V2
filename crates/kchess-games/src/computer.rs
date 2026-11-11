//! A game against the engine: the domain's typed session (`kchess_domain::games::computer`),
//! which the frontend calls with what it knows (`Host`) and whose effects it carries out, in order.

pub use kchess_domain::games::Host;
pub use kchess_domain::games::computer::*;
