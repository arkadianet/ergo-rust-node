//! Penalty definitions for peer misbehaviors.
//!
//! This module defines penalty amounts for various protocol violations
//! and misbehaviors. Penalties accumulate and can lead to temporary bans.
//! Penalties decay over time, allowing peers to be rehabilitated.

/// Penalty amounts for various peer misbehaviors.
///
/// The scoring system uses negative points (penalties) that accumulate.
/// When a peer's total penalty exceeds the ban threshold, they are banned.
/// Penalties decay over time at a rate of DECAY_PER_MINUTE.
pub struct Penalties;

impl Penalties {
    // ========== Minor violations (1-10 points) ==========

    /// Slow response to a request (response came but was slow).
    pub const SLOW_RESPONSE: u32 = 2;

    /// Missing response to a request (no response after timeout).
    pub const MISSING_RESPONSE: u32 = 10;

    /// Sent a duplicate message we already have.
    pub const DUPLICATE_MESSAGE: u32 = 5;

    /// Sent a message with invalid checksum.
    pub const INVALID_CHECKSUM: u32 = 5;

    // ========== Moderate violations (20-50 points) ==========

    /// Sent a message with invalid format.
    pub const INVALID_MESSAGE_FORMAT: u32 = 20;

    /// Sent an unexpected message for the current protocol state.
    pub const UNEXPECTED_MESSAGE: u32 = 30;

    /// Sent an invalid modifier (header, block, transaction).
    pub const INVALID_MODIFIER: u32 = 50;

    /// Sent a modifier we previously declined.
    pub const DECLINED_MODIFIER: u32 = 40;

    // ========== Severe violations (80-150 points) ==========

    /// Sent an invalid transaction.
    pub const INVALID_TRANSACTION: u32 = 80;

    /// Sent an invalid block.
    pub const INVALID_BLOCK: u32 = 100;

    /// Spam detected (too many requests, flooding).
    pub const SPAM_DETECTED: u32 = 150;

    /// Sent data that exceeds size limits.
    pub const SIZE_LIMIT_EXCEEDED: u32 = 100;

    // ========== Critical violations (instant ban) ==========

    /// Severe protocol violation (e.g., invalid handshake).
    pub const PROTOCOL_VIOLATION: u32 = 500;

    /// Malicious behavior detected.
    pub const MALICIOUS_BEHAVIOR: u32 = 1000;

    // ========== Thresholds and decay ==========

    /// Maximum penalty score before ban.
    pub const BAN_THRESHOLD: u32 = 500;

    /// Penalty decay per minute.
    pub const DECAY_PER_MINUTE: u32 = 10;

    /// Maximum penalty (saturates at this value).
    pub const MAX_PENALTY: u32 = 1000;
}

/// Reward amounts for good peer behavior.
pub struct Rewards;

impl Rewards {
    /// Successfully delivered a requested modifier.
    pub const SUCCESSFUL_DELIVERY: i32 = 1;

    /// Fast response to a request.
    pub const FAST_RESPONSE: i32 = 1;

    /// Successfully connected.
    pub const SUCCESSFUL_CONNECTION: i32 = 5;

    /// Valid block received.
    pub const VALID_BLOCK: i32 = 2;

    /// Maximum score a peer can have (cap for rewards).
    pub const MAX_SCORE: i32 = 100;
}

/// Penalty reason for logging and debugging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PenaltyReason {
    SlowResponse,
    MissingResponse,
    DuplicateMessage,
    InvalidChecksum,
    InvalidMessageFormat,
    UnexpectedMessage,
    InvalidModifier,
    DeclinedModifier,
    InvalidTransaction,
    InvalidBlock,
    SpamDetected,
    SizeLimitExceeded,
    ProtocolViolation,
    MaliciousBehavior,
}

impl PenaltyReason {
    /// Get the penalty amount for this reason.
    pub fn penalty(&self) -> u32 {
        match self {
            Self::SlowResponse => Penalties::SLOW_RESPONSE,
            Self::MissingResponse => Penalties::MISSING_RESPONSE,
            Self::DuplicateMessage => Penalties::DUPLICATE_MESSAGE,
            Self::InvalidChecksum => Penalties::INVALID_CHECKSUM,
            Self::InvalidMessageFormat => Penalties::INVALID_MESSAGE_FORMAT,
            Self::UnexpectedMessage => Penalties::UNEXPECTED_MESSAGE,
            Self::InvalidModifier => Penalties::INVALID_MODIFIER,
            Self::DeclinedModifier => Penalties::DECLINED_MODIFIER,
            Self::InvalidTransaction => Penalties::INVALID_TRANSACTION,
            Self::InvalidBlock => Penalties::INVALID_BLOCK,
            Self::SpamDetected => Penalties::SPAM_DETECTED,
            Self::SizeLimitExceeded => Penalties::SIZE_LIMIT_EXCEEDED,
            Self::ProtocolViolation => Penalties::PROTOCOL_VIOLATION,
            Self::MaliciousBehavior => Penalties::MALICIOUS_BEHAVIOR,
        }
    }

    /// Check if this is a critical violation (instant ban).
    pub fn is_critical(&self) -> bool {
        matches!(self, Self::ProtocolViolation | Self::MaliciousBehavior)
    }
}

impl std::fmt::Display for PenaltyReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SlowResponse => write!(f, "slow response"),
            Self::MissingResponse => write!(f, "missing response"),
            Self::DuplicateMessage => write!(f, "duplicate message"),
            Self::InvalidChecksum => write!(f, "invalid checksum"),
            Self::InvalidMessageFormat => write!(f, "invalid message format"),
            Self::UnexpectedMessage => write!(f, "unexpected message"),
            Self::InvalidModifier => write!(f, "invalid modifier"),
            Self::DeclinedModifier => write!(f, "declined modifier resent"),
            Self::InvalidTransaction => write!(f, "invalid transaction"),
            Self::InvalidBlock => write!(f, "invalid block"),
            Self::SpamDetected => write!(f, "spam detected"),
            Self::SizeLimitExceeded => write!(f, "size limit exceeded"),
            Self::ProtocolViolation => write!(f, "protocol violation"),
            Self::MaliciousBehavior => write!(f, "malicious behavior"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_penalty_values() {
        // Minor penalties should be low
        assert!(Penalties::SLOW_RESPONSE < 10);
        assert!(Penalties::MISSING_RESPONSE <= 10);

        // Moderate penalties
        assert!(Penalties::INVALID_MODIFIER >= 20);
        assert!(Penalties::INVALID_MODIFIER <= 50);

        // Severe penalties
        assert!(Penalties::INVALID_BLOCK >= 80);
        assert!(Penalties::SPAM_DETECTED >= 100);

        // Critical should exceed ban threshold
        assert!(Penalties::PROTOCOL_VIOLATION >= Penalties::BAN_THRESHOLD);
        assert!(Penalties::MALICIOUS_BEHAVIOR >= Penalties::BAN_THRESHOLD);
    }

    #[test]
    fn test_penalty_reason() {
        assert_eq!(
            PenaltyReason::InvalidBlock.penalty(),
            Penalties::INVALID_BLOCK
        );
        assert!(PenaltyReason::MaliciousBehavior.is_critical());
        assert!(!PenaltyReason::SlowResponse.is_critical());
    }

    #[test]
    fn test_decay_allows_recovery() {
        // After 50 minutes, a 500-point penalty should decay to 0
        let initial_penalty = 500u32;
        let minutes = 50u32;
        let decayed = initial_penalty.saturating_sub(minutes * Penalties::DECAY_PER_MINUTE);
        assert_eq!(decayed, 0);
    }
}
