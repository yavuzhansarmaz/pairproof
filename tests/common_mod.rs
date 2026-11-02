use pairproof::connection::{Endpoint, EphemeralUsername};

pub fn creds() -> (Endpoint, EphemeralUsername) {
    (
        Endpoint::new("203.0.113.10", 2222).expect("endpoint"),
        EphemeralUsername::parse("pp-abcd1234").expect("user"),
    )
}
