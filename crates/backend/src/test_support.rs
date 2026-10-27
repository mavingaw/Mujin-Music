//! Helpers shared by the unit tests.

use rand::RngExt;
use rand::distr::Alphanumeric;

/// A password for a test account, made up when the test runs, so no test
/// depends on a fixed credential.
pub(crate) fn generated_password() -> String {
    rand::rng()
        .sample_iter(Alphanumeric)
        .take(24)
        .map(char::from)
        .collect()
}

/// A near miss for `password`: the same text without its last character.
pub(crate) fn wrong_password(password: &str) -> String {
    let mut wrong = password.to_owned();
    wrong.pop();
    assert_ne!(wrong, password, "a wrong password must differ");
    wrong
}

mod tests {
    use super::{generated_password, wrong_password};

    #[test]
    fn generated_passwords_are_long_and_never_repeat() {
        let first = generated_password();
        let second = generated_password();
        assert_eq!(first.len(), 24);
        assert!(first.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(first, second);
        let wrong = wrong_password(&first);
        assert_ne!(wrong, first);
        assert!(first.starts_with(&wrong));
    }
}
