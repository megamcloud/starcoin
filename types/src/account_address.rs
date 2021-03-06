// Copyright (c) The Starcoin Core Contributors
// SPDX-License-Identifier: Apache-2.0

// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::transaction::authenticator::AuthenticationKey;
use anyhow::{ensure, Error, Result};
use bytes::Bytes;
use rand::{rngs::OsRng, Rng};
use serde::{de::Error as _, Deserialize, Deserializer, Serialize, Serializer};
use starcoin_crypto::{ed25519::Ed25519PublicKey, hash::CryptoHash, HashValue};
use std::borrow::Cow;
use std::{convert::TryFrom, fmt, str::FromStr};

pub const ADDRESS_LENGTH: usize = 16;
pub const AUTHENTICATION_KEY_LENGTH: usize = ADDRESS_LENGTH * 2;

const SHORT_STRING_LENGTH: usize = 4;

/// A struct that represents an account address.
#[derive(Ord, PartialOrd, Eq, PartialEq, Hash, Clone, Copy)]
pub struct AccountAddress([u8; ADDRESS_LENGTH]);

impl AccountAddress {
    pub const fn new(address: [u8; ADDRESS_LENGTH]) -> Self {
        AccountAddress(address)
    }

    pub const DEFAULT: Self = Self([0u8; ADDRESS_LENGTH]);

    pub fn random() -> Self {
        let mut rng = OsRng::new().expect("can't access OsRng");
        let buf: [u8; ADDRESS_LENGTH] = rng.gen();
        AccountAddress::new(buf)
    }

    // Helpful in log messages
    pub fn short_str(&self) -> String {
        hex::encode(&self.0[..SHORT_STRING_LENGTH])
    }

    pub fn to_vec(&self) -> Vec<u8> {
        self.0.to_vec()
    }

    pub fn authentication_key(public_key: &Ed25519PublicKey) -> AuthenticationKey {
        AuthenticationKey::ed25519(public_key)
    }

    pub fn from_public_key(public_key: &Ed25519PublicKey) -> Self {
        AccountAddress::authentication_key(public_key)
            .derived_address()
            .into()
    }

    pub fn from_hex_literal(literal: &str) -> Result<Self> {
        ensure!(literal.starts_with("0x"), "literal must start with 0x.");

        let hex_len = literal.len() - 2;
        let mut result = if hex_len % 2 != 0 {
            let mut hex_str = String::with_capacity(hex_len + 1);
            hex_str.push('0');
            hex_str.push_str(&literal[2..]);
            hex::decode(&hex_str)?
        } else {
            hex::decode(&literal[2..])?
        };

        let len = result.len();
        let padded_result = if len < ADDRESS_LENGTH {
            let mut padded = Vec::with_capacity(ADDRESS_LENGTH);
            padded.resize(ADDRESS_LENGTH - len, 0u8);
            padded.append(&mut result);
            padded
        } else {
            result
        };

        AccountAddress::try_from(padded_result)
    }

    pub fn into_inner(self) -> [u8; ADDRESS_LENGTH] {
        self.0
    }
}

impl Default for AccountAddress {
    fn default() -> AccountAddress {
        AccountAddress::DEFAULT
    }
}

impl AsRef<[u8]> for AccountAddress {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Display for AccountAddress {
    fn fmt(&self, f: &mut fmt::Formatter) -> std::fmt::Result {
        // Forward to the LowerHex impl with a "0x" prepended (the # flag).
        write!(f, "{:#x}", self)
    }
}

impl fmt::Debug for AccountAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Forward to the LowerHex impl with a "0x" prepended (the # flag).
        write!(f, "{:#x}", self)
    }
}

impl fmt::LowerHex for AccountAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(&self.0))
    }
}

impl TryFrom<&[u8]> for AccountAddress {
    type Error = Error;

    /// Tries to convert the provided byte array into Address.
    fn try_from(bytes: &[u8]) -> Result<AccountAddress> {
        ensure!(
            bytes.len() == ADDRESS_LENGTH,
            "The Address {:?} is of invalid length",
            bytes
        );
        let mut addr = [0u8; ADDRESS_LENGTH];
        addr.copy_from_slice(bytes);
        Ok(AccountAddress(addr))
    }
}

impl TryFrom<&[u8; ADDRESS_LENGTH]> for AccountAddress {
    type Error = Error;

    /// Tries to convert the provided byte array into Address.
    fn try_from(bytes: &[u8; ADDRESS_LENGTH]) -> Result<AccountAddress> {
        AccountAddress::try_from(&bytes[..])
    }
}

impl TryFrom<Vec<u8>> for AccountAddress {
    type Error = Error;

    /// Tries to convert the provided byte buffer into Address.
    fn try_from(bytes: Vec<u8>) -> Result<AccountAddress> {
        AccountAddress::try_from(&bytes[..])
    }
}

impl From<AccountAddress> for Vec<u8> {
    fn from(addr: AccountAddress) -> Vec<u8> {
        addr.0.to_vec()
    }
}

impl From<&AccountAddress> for Vec<u8> {
    fn from(addr: &AccountAddress) -> Vec<u8> {
        addr.0.to_vec()
    }
}

impl TryFrom<Bytes> for AccountAddress {
    type Error = Error;

    fn try_from(bytes: Bytes) -> Result<AccountAddress> {
        AccountAddress::try_from(bytes.as_ref())
    }
}

impl From<AccountAddress> for Bytes {
    fn from(addr: AccountAddress) -> Bytes {
        Bytes::copy_from_slice(addr.0.as_ref())
    }
}

impl From<&AccountAddress> for String {
    fn from(addr: &AccountAddress) -> String {
        ::hex::encode(addr.as_ref())
    }
}

impl TryFrom<String> for AccountAddress {
    type Error = Error;

    fn try_from(s: String) -> Result<AccountAddress> {
        assert!(!s.is_empty());
        let bytes_out = ::hex::decode(s)?;
        AccountAddress::try_from(bytes_out.as_slice())
    }
}

impl FromStr for AccountAddress {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        //assert!(!s.is_empty());
        let bytes_out = ::hex::decode(s)?;
        AccountAddress::try_from(bytes_out.as_slice())
    }
}

impl<'de> Deserialize<'de> for AccountAddress {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let s: Cow<str> = serde::private::de::borrow_cow_str(deserializer)?;
            // let s = <&str>::deserialize(deserializer)?;
            AccountAddress::from_str(s.as_ref()).map_err(D::Error::custom)
        } else {
            let b = <[u8; ADDRESS_LENGTH]>::deserialize(deserializer)?;
            Ok(AccountAddress::new(b))
        }
    }
}

impl Serialize for AccountAddress {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            self.to_string().serialize(serializer)
        } else {
            self.0.serialize(serializer)
        }
    }
}

//======================= after libra ============================

impl CryptoHash for AccountAddress {
    fn crypto_hash(&self) -> HashValue {
        HashValue::from_sha3_256(self.as_ref())
    }
}

impl Into<libra_types::account_address::AccountAddress> for AccountAddress {
    fn into(self) -> libra_types::account_address::AccountAddress {
        libra_types::account_address::AccountAddress::new(self.0)
    }
}

impl From<libra_types::account_address::AccountAddress> for AccountAddress {
    fn from(libra_address: libra_types::account_address::AccountAddress) -> Self {
        Self::try_from(libra_address.to_vec()).expect("libra address to address must success.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert() {
        let address0 = AccountAddress::random();
        let address1: libra_types::account_address::AccountAddress = address0.into();
        let address2: AccountAddress = address1.into();
        assert_eq!(address0, address2);
    }
}
