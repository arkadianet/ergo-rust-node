//! Advanced peer scoring with decay, response time tracking, and delivery metrics.
//!
//! This module provides a comprehensive peer scoring system that:
//! - Tracks penalties that decay over time
//! - Records response times for prioritizing fast peers
//! - Tracks modifier delivery success/failure rates
//! - Calculates peer priority for sync operations

use crate::penalties::{Penalties, PenaltyReason, Rewards};
use std::time::{Duration, Instant};

/// Peer score with decay and detailed tracking.
#[derive(Debug, Clone)]
pub struct PeerScore {
    /// Accumulated penalty points (0 to MAX_PENALTY).
    penalty: u32,
    /// Last time penalty was updated (for decay calculation).
    last_penalty_time: Instant,
    /// Average response time in milliseconds (exponential moving average).
    avg_response_time_ms: u64,
    /// Number of responses used to calculate average.
    response_count: u64,
    /// Number of modifiers successfully delivered.
    delivered_count: u64,
    /// Number of modifiers requested but not delivered.
    failed_count: u64,
    /// Number of invalid messages received.
    invalid_message_count: u32,
    /// Accumulated rewards (positive score).
    reward_score: i32,
    /// Time of last successful interaction.
    last_success: Instant,
}

impl Default for PeerScore {
    fn default() -> Self {
        Self::new()
    }
}

impl PeerScore {
    /// Create a new peer score with default values.
    pub fn new() -> Self {
        Self {
            penalty: 0,
            last_penalty_time: Instant::now(),
            avg_response_time_ms: 0,
            response_count: 0,
            delivered_count: 0,
            failed_count: 0,
            invalid_message_count: 0,
            reward_score: 0,
            last_success: Instant::now(),
        }
    }

    /// Calculate the current penalty after decay.
    ///
    /// Penalties decay at DECAY_PER_MINUTE rate.
    pub fn current_penalty(&self) -> u32 {
        let minutes_elapsed = self.last_penalty_time.elapsed().as_secs() / 60;
        let decay = (minutes_elapsed as u32).saturating_mul(Penalties::DECAY_PER_MINUTE);
        self.penalty.saturating_sub(decay)
    }

    /// Apply a penalty to this peer.
    ///
    /// # Arguments
    /// * `reason` - The reason for the penalty
    ///
    /// # Returns
    /// `true` if the peer should now be banned
    pub fn apply_penalty(&mut self, reason: PenaltyReason) -> bool {
        let amount = reason.penalty();
        self.apply_penalty_amount(amount)
    }

    /// Apply a specific penalty amount.
    ///
    /// # Arguments
    /// * `amount` - The penalty amount to apply
    ///
    /// # Returns
    /// `true` if the peer should now be banned
    pub fn apply_penalty_amount(&mut self, amount: u32) -> bool {
        // Calculate current penalty with decay
        let current = self.current_penalty();

        // Add new penalty
        self.penalty = current.saturating_add(amount).min(Penalties::MAX_PENALTY);
        self.last_penalty_time = Instant::now();

        // Check if should ban
        self.should_ban()
    }

    /// Apply a reward to this peer.
    pub fn apply_reward(&mut self, amount: i32) {
        self.reward_score = (self.reward_score + amount).min(Rewards::MAX_SCORE);
        self.last_success = Instant::now();
    }

    /// Record a successful delivery.
    pub fn record_delivery(&mut self) {
        self.delivered_count += 1;
        self.apply_reward(Rewards::SUCCESSFUL_DELIVERY);
    }

    /// Record a failed delivery (requested but not received).
    pub fn record_failure(&mut self) {
        self.failed_count += 1;
    }

    /// Record a response time.
    ///
    /// Uses exponential moving average with alpha = 0.125 (1/8).
    pub fn record_response_time(&mut self, duration: Duration) {
        let new_time = duration.as_millis() as u64;

        if self.response_count == 0 {
            self.avg_response_time_ms = new_time;
        } else {
            // EMA: new_avg = old_avg * 0.875 + new_value * 0.125
            self.avg_response_time_ms = (self.avg_response_time_ms * 7 + new_time) / 8;
        }

        self.response_count += 1;

        // Reward fast responses
        if new_time < 500 {
            self.apply_reward(Rewards::FAST_RESPONSE);
        }
    }

    /// Record an invalid message.
    pub fn record_invalid_message(&mut self) {
        self.invalid_message_count += 1;
    }

    /// Check if this peer should be banned.
    pub fn should_ban(&self) -> bool {
        self.current_penalty() >= Penalties::BAN_THRESHOLD
    }

    /// Calculate peer priority for sync operations.
    ///
    /// Higher values indicate better peers. Considers:
    /// - Response time (faster is better)
    /// - Delivery reliability (higher success rate is better)
    /// - Current penalty (lower is better)
    /// - Reward score (higher is better)
    ///
    /// Returns a value between 0.0 and 1.0.
    pub fn priority(&self) -> f64 {
        // Response time factor (0.0 to 1.0, faster is higher)
        // 0ms -> 1.0, 1000ms -> 0.5, 10000ms -> 0.1
        let response_factor = if self.avg_response_time_ms == 0 {
            0.5 // Unknown, neutral
        } else {
            1.0 / (1.0 + (self.avg_response_time_ms as f64 / 1000.0))
        };

        // Reliability factor (0.0 to 1.0)
        let total_requests = self.delivered_count + self.failed_count;
        let reliability_factor = if total_requests == 0 {
            0.5 // Unknown, neutral
        } else {
            self.delivered_count as f64 / total_requests as f64
        };

        // Penalty factor (0.0 to 1.0, lower penalty is higher)
        let penalty_factor = 1.0 - (self.current_penalty() as f64 / Penalties::MAX_PENALTY as f64);

        // Reward factor (0.0 to 1.0)
        let reward_factor = if self.reward_score <= 0 {
            0.5
        } else {
            0.5 + (self.reward_score as f64 / Rewards::MAX_SCORE as f64) * 0.5
        };

        // Weighted combination
        // Reliability is most important for sync, then response time, then scores
        (reliability_factor * 0.35
            + response_factor * 0.30
            + penalty_factor * 0.20
            + reward_factor * 0.15)
            .clamp(0.0, 1.0)
    }

    /// Get the average response time in milliseconds.
    pub fn avg_response_time(&self) -> u64 {
        self.avg_response_time_ms
    }

    /// Get the number of successful deliveries.
    pub fn delivered_count(&self) -> u64 {
        self.delivered_count
    }

    /// Get the number of failed deliveries.
    pub fn failed_count(&self) -> u64 {
        self.failed_count
    }

    /// Get the delivery success rate (0.0 to 1.0).
    pub fn delivery_rate(&self) -> f64 {
        let total = self.delivered_count + self.failed_count;
        if total == 0 {
            0.0
        } else {
            self.delivered_count as f64 / total as f64
        }
    }

    /// Get the raw penalty (without decay).
    pub fn raw_penalty(&self) -> u32 {
        self.penalty
    }

    /// Get the reward score.
    pub fn reward_score(&self) -> i32 {
        self.reward_score
    }

    /// Get time since last successful interaction.
    pub fn time_since_success(&self) -> Duration {
        self.last_success.elapsed()
    }

    /// Reset the score to default values.
    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_score() {
        let score = PeerScore::new();
        assert_eq!(score.current_penalty(), 0);
        assert!(!score.should_ban());
        assert_eq!(score.delivered_count(), 0);
        assert_eq!(score.failed_count(), 0);
    }

    #[test]
    fn test_penalty_application() {
        let mut score = PeerScore::new();

        // Apply small penalty
        let should_ban = score.apply_penalty(PenaltyReason::SlowResponse);
        assert!(!should_ban);
        assert_eq!(score.current_penalty(), Penalties::SLOW_RESPONSE);

        // Apply more penalties
        for _ in 0..50 {
            score.apply_penalty(PenaltyReason::MissingResponse);
        }
        assert!(score.should_ban());
    }

    #[test]
    fn test_penalty_decay() {
        let mut score = PeerScore::new();
        score.apply_penalty_amount(100);
        assert_eq!(score.current_penalty(), 100);

        // Simulate time passing (can't actually wait in test)
        // Instead, directly manipulate last_penalty_time
        score.last_penalty_time = Instant::now() - Duration::from_secs(60 * 5); // 5 minutes ago

        // Should have decayed by 50 points (5 min * 10 decay/min)
        assert_eq!(score.current_penalty(), 50);
    }

    #[test]
    fn test_penalty_full_decay() {
        let mut score = PeerScore::new();
        score.apply_penalty_amount(100);

        // After 15 minutes, should be fully decayed
        score.last_penalty_time = Instant::now() - Duration::from_secs(60 * 15);
        assert_eq!(score.current_penalty(), 0);
    }

    #[test]
    fn test_response_time_tracking() {
        let mut score = PeerScore::new();

        // First response sets the average
        score.record_response_time(Duration::from_millis(100));
        assert_eq!(score.avg_response_time(), 100);

        // Second response uses EMA
        score.record_response_time(Duration::from_millis(500));
        // EMA: 100 * 0.875 + 500 * 0.125 = 87.5 + 62.5 = 150
        assert_eq!(score.avg_response_time(), 150);
    }

    #[test]
    fn test_delivery_tracking() {
        let mut score = PeerScore::new();

        score.record_delivery();
        score.record_delivery();
        score.record_failure();

        assert_eq!(score.delivered_count(), 2);
        assert_eq!(score.failed_count(), 1);
        assert!((score.delivery_rate() - 0.666).abs() < 0.01);
    }

    #[test]
    fn test_priority_calculation() {
        let mut good_peer = PeerScore::new();
        good_peer.record_response_time(Duration::from_millis(50));
        for _ in 0..10 {
            good_peer.record_delivery();
        }

        let mut bad_peer = PeerScore::new();
        bad_peer.record_response_time(Duration::from_millis(5000));
        for _ in 0..10 {
            bad_peer.record_failure();
        }
        bad_peer.apply_penalty_amount(200);

        assert!(good_peer.priority() > bad_peer.priority());
    }

    #[test]
    fn test_critical_penalty_bans() {
        let mut score = PeerScore::new();

        // A single critical penalty should ban
        let should_ban = score.apply_penalty(PenaltyReason::MaliciousBehavior);
        assert!(should_ban);
        assert!(score.should_ban());
    }

    #[test]
    fn test_rewards() {
        let mut score = PeerScore::new();

        score.apply_reward(10);
        assert_eq!(score.reward_score(), 10);

        // Should cap at MAX_SCORE
        score.apply_reward(200);
        assert_eq!(score.reward_score(), Rewards::MAX_SCORE);
    }

    #[test]
    fn test_reset() {
        let mut score = PeerScore::new();
        score.apply_penalty_amount(100);
        score.record_delivery();

        score.reset();

        assert_eq!(score.current_penalty(), 0);
        assert_eq!(score.delivered_count(), 0);
    }
}
