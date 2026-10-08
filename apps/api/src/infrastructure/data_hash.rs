//! Composite hash of data files for `config_hash`. Each part is `(label, bytes)`, length-prefixed
//! so parts cannot be shifted into one another. Feed parts in a fixed order (sorted by label).

use sha2::{Digest, Sha256};

/// Incremental config hash.
#[derive(Default)]
pub struct DataHash(Sha256);

impl DataHash {
    /// An empty hash.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one labelled part.
    #[must_use]
    pub fn part(mut self, label: &str, bytes: &[u8]) -> Self {
        for field in [label.as_bytes(), bytes] {
            self.0
                .update(u64::try_from(field.len()).unwrap_or(u64::MAX).to_le_bytes());
            self.0.update(field);
        }
        self
    }

    /// `sha256:<64 hex>`.
    #[must_use]
    pub fn finish(self) -> String {
        use std::fmt::Write as _;
        let digest = self.0.finalize();
        digest.iter().fold(String::from("sha256:"), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_cannot_be_shifted_into_each_other() {
        let a = DataHash::new().part("ab", b"c").finish();
        let b = DataHash::new().part("a", b"bc").finish();
        assert_ne!(a, b);
        assert_eq!(a, DataHash::new().part("ab", b"c").finish());
    }
}
