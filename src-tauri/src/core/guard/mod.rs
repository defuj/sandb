//! Layer 1 of the AI safeguards: classifies AI-written SQL before it reaches a database.
//! Policies only; the facts come from `core::sql::facts`.

mod classify;
#[cfg(test)]
mod tests;

pub use classify::{
    GuardCheck, GuardRejection, ReadOk, WarningCode, WriteInfo, WriteStatement, WriteWarning,
    check_ai_read, classify_write, guard_check,
};
