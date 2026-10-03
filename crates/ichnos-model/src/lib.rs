//! The viewer's side of a session, built from its logged events.
//!
//! Pure: no clock, no async runtime, no window. It reads only Ichnos events and their tool
//! fields, never which agent produced them.

pub mod call;
pub mod step;
