//! HP Prime Desktop IDE End-to-End Integration Test Suite.
//!
//! Exclusively owns and runs requirement-driven opaque-box integration tests
//! covering all 32 inventoried features across Tiers 1–4 (362 test cases):
//!
//! - `harness`: Headless programmatic models and VM execution harnesses.
//! - `tier1_coverage`: Tier 1 Feature Coverage (160 test cases: 32 features × 5 tests).
//! - `tier2_boundary`: Tier 2 Boundary & Corner Cases (160 test cases: 32 features × 5 tests).
//! - `tier3_pairwise`: Tier 3 Cross-Feature Interactions (32 test cases).
//! - `tier4_scenarios`: Tier 4 Real-World Application Workflows (10 test cases).

pub mod harness;
pub mod tier1_coverage;
pub mod tier2_boundary;
pub mod tier3_pairwise;
pub mod tier4_scenarios;
