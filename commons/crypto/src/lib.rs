// Copyright (c) The Starcoin Core Contributors
// SPDX-License-Identifier: Apache-2.0

#![forbid(unsafe_code)]

//! A library supplying various cryptographic primitives
// just wrap libra-crypto.

pub mod ed25519;
pub mod hash;
pub mod keygen;
pub mod test_utils;
pub mod traits;

pub use crate::hash::HashValue;
pub use crate::traits::*;
