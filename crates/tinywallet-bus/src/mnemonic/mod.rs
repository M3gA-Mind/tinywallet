//! BIP-39 mnemonic word-count rule shared by hosts and the wallet module.
//!
//! Pure vocabulary: no I/O, no BIP-39 word list, no entropy handling. Hosts
//! use [`validate_mnemonic_word_count`] to reject a request before it crosses
//! the bus; the compiled module remains the authority on the phrase itself.

/// The word counts BIP-39 defines (128 to 256 bits of entropy).
pub const MNEMONIC_WORD_COUNTS: [u8; 5] = [12, 15, 18, 21, 24];

/// A mnemonic word count outside [`MNEMONIC_WORD_COUNTS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("unsupported mnemonic word count {count}")]
pub struct UnsupportedWordCount {
    /// The rejected word count.
    pub count: usize,
}

/// Accepts a word count that BIP-39 defines.
///
/// # Errors
///
/// Returns [`UnsupportedWordCount`] when `count` is not in
/// [`MNEMONIC_WORD_COUNTS`].
pub fn validate_mnemonic_word_count(count: u8) -> Result<u8, UnsupportedWordCount> {
    if MNEMONIC_WORD_COUNTS.contains(&count) {
        Ok(count)
    } else {
        Err(UnsupportedWordCount {
            count: usize::from(count),
        })
    }
}

/// Like [`validate_mnemonic_word_count`] for a count taken from a phrase's
/// whitespace-split length, which may exceed `u8`.
///
/// # Errors
///
/// Returns [`UnsupportedWordCount`] when `count` is not in
/// [`MNEMONIC_WORD_COUNTS`].
pub fn validate_mnemonic_word_count_usize(count: usize) -> Result<u8, UnsupportedWordCount> {
    u8::try_from(count)
        .ok()
        .and_then(|narrow| validate_mnemonic_word_count(narrow).ok())
        .ok_or(UnsupportedWordCount { count })
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
