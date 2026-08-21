//! Evidence and provenance model (directive sections 4.1 and 10).
//!
//! The rule this crate exists to enforce: nothing downstream may assert a
//! claim about the ecosystem without an `Evidence` record backing it, and
//! that record must say plainly which of the five confidence classes the
//! claim belongs to. Nothing here silently upgrades a guess into a fact.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The five confidence classes from directive section 4.1. Every claim
/// Orbweaver stores must be tagged with exactly one of these — mixing them
/// up (e.g. treating an LLM hypothesis as an observed fact) is the failure
/// mode this type exists to prevent.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Confidence {
    /// Directly read from a source (a file exists, a commit has this
    /// timestamp). Not an inference at all.
    Observed,
    /// Follows from observed facts by a fixed rule with no uncertainty
    /// (e.g. "Cargo.toml lists a path dependency on ../foo").
    DeterministicInference,
    /// Follows from observed facts by a rule that can be wrong; carries an
    /// explicit probability estimate.
    ProbabilisticInference(f32),
    /// Produced by an LLM reasoning pass; carries the model's own
    /// confidence if it gave one.
    LlmHypothesis(f32),
    /// Not a claim about current state at all — a proposed future
    /// intervention or opportunity.
    ProposedOpportunity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub id: Uuid,
    pub source: String,
    pub source_type: SourceType,
    pub repository: String,
    pub commit: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub extractor: String,
    pub confidence: Confidence,
    pub raw_reference: String,
    pub derived_claim: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceType {
    Manifest,
    GitHistory,
    Readme,
    Filesystem,
}

impl Evidence {
    pub fn new(
        source: impl Into<String>,
        source_type: SourceType,
        repository: impl Into<String>,
        extractor: impl Into<String>,
        confidence: Confidence,
        raw_reference: impl Into<String>,
        derived_claim: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            source: source.into(),
            source_type,
            repository: repository.into(),
            commit: None,
            timestamp: Utc::now(),
            extractor: extractor.into(),
            confidence,
            raw_reference: raw_reference.into(),
            derived_claim: derived_claim.into(),
        }
    }
}

/// A value that may be unknown for a reason that itself matters (directive
/// section 33: "unavailable" and "unknown" must never collapse into
/// `false`/`None`/zero).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Availability<T> {
    Known(T),
    Unavailable { reason: String },
    Unknown,
}
