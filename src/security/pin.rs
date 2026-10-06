use super::{argon2, keyring};

pub fn setup_new_pin(pin: &str) -> Result<(), String> {
    let hashed = argon2::hash_pin(pin);
    keyring::save_hashed_pin(&hashed).map_err(|e| e.to_string())
}

pub fn validate_pin(pin: &str) -> bool {
    if let Some(hashed_pin) = keyring::get_hashed_pin() {
        argon2::verify_pin(pin, &hashed_pin)
    } else {
        false
    }
}