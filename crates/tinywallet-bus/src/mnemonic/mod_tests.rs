#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

#[test]
fn accepts_every_bip39_word_count() {
    for count in MNEMONIC_WORD_COUNTS {
        assert_eq!(validate_mnemonic_word_count(count), Ok(count));
        assert_eq!(
            validate_mnemonic_word_count_usize(usize::from(count)),
            Ok(count)
        );
    }
}

#[test]
fn rejects_counts_between_and_outside_the_valid_set() {
    for count in [0u8, 1, 11, 13, 14, 16, 20, 23, 25, 255] {
        assert_eq!(
            validate_mnemonic_word_count(count),
            Err(UnsupportedWordCount {
                count: usize::from(count)
            })
        );
    }
}

#[test]
fn usize_variant_rejects_counts_that_overflow_u8() {
    // 268 would truncate to 12 under a bare `as u8` cast.
    assert_eq!(
        validate_mnemonic_word_count_usize(268),
        Err(UnsupportedWordCount { count: 268 })
    );
}

#[test]
fn error_message_names_the_rejected_count() {
    assert_eq!(
        validate_mnemonic_word_count(13).unwrap_err().to_string(),
        "unsupported mnemonic word count 13"
    );
}
