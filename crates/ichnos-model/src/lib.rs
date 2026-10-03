//! The viewer's side of a session: the live feed that tails its database, the events and model
//! steps received so far, and the session model built from them, with the labels, filters and
//! "Ask agent" prompt the views show.
//!
//! Pure apart from [`feed::Feed`]'s reads: no clock, no async runtime, no window. The model and
//! labels read only Ichnos events and their tool fields, never which agent produced them.

pub mod ask;
pub mod call;
pub mod feed;
pub mod filter;
pub mod labels;
pub mod model;
pub mod session_data;
pub mod step;
