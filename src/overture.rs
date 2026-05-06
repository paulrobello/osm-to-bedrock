//! Compatibility re-exports for Overture Maps support.
//!
//! The implementation lives in `par-osm-rust` so `osm-to-bedrock` and
//! `osm-world` share one source of truth for Overture fetching, parsing,
//! caching, and source policy.

pub use par_osm_rust::overture::*;
