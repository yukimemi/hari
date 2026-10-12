//! Native Git hook execution for Jujutsu.
//!
//! Hari configures jj's `git.executable-path` to a Git proxy. The proxy
//! removes only Git push's `--no-verify`, leaving hook discovery, remote
//! advertisement, hook input, and transport safety checks to Git itself.

pub mod proxy;
