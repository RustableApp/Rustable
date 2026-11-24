//! Bounded descriptions for generated DROP/TRUNCATE action reviews.
//! User SQL confirmation is controlled by the connection protection mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// Statement text as written, trimmed and shortened for display.
    pub statement: String,
    pub reason: &'static str,
}
