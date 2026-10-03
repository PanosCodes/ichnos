//! The viewer's side of a session: the live feed that tails its database, the events and model
//! steps received so far, and the session model built from them.
//!
//! Pure apart from [`feed::Feed`]'s reads: no clock, no async runtime, no window. The model reads
//! only Ichnos events and their tool fields, never which agent produced them.

pub mod call;
pub mod feed;
pub mod model;
pub mod session_data;
pub mod step;
