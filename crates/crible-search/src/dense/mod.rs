// SPDX-License-Identifier: Apache-2.0

//! Dense branch of a search: the cosine between the query and the passages of
//! the pool.
//!
//! `docs/03-recherche.md` ("Branche dense") splits the branch by strategy: the
//! scan of the binary codes above the exact threshold, and the exact dot
//! product below it. Phase P1 only implements the exact strategy
//! ([`exact`]); the planner and the binary scan arrive in P4
//! (`PLANIFIER-STRATEGIE`). Nothing here chooses a strategy: the caller decides
//! and passes the pool to search.

pub mod exact;
