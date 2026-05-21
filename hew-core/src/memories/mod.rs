//! Memory-side data structures shared across hew.
//!
//! Today this module is a thin namespace; the live surface is
//! [`links`] which fixes the cross-memory [`LinkRow`] grammar that
//! later tasks in the Memory Links epic (hew-dko) hang off of.
//!
//! [`LinkRow`]: links::LinkRow

pub mod links;
pub mod suggest;
