use keyring::Entry;

const SERVICE: &str = "app_access_locker_service";
const USER: &str = "local_user"; // Podría ser dinámico según el SO

fn get_entry() -> Entry {
    Entry::new(SERVICE, USER).unwrap()
}

pub fn has_pin_setup() -> bool {
    get_entry().get_password().is_ok()
}

pub fn save_hashed_pin(hashed_pin: &str) -> Result<(), keyring::Error> {
    let entry = get_entry();
    entry.set_password(hashed_pin)
}

pub fn get_hashed_pin() -> Option<String> {
    get_entry().get_password().ok()
}
