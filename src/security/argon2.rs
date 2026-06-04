use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};

pub fn hash_pin(pin: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let password_hash = argon2.hash_password(pin.as_bytes(), &salt).unwrap();
    password_hash.to_string()
}

pub fn verify_pin(pin: &str, hashed_pin: &str) -> bool {
    let parsed_hash = PasswordHash::new(hashed_pin).unwrap();
    Argon2::default()
        .verify_password(pin.as_bytes(), &parsed_hash)
        .is_ok()
}