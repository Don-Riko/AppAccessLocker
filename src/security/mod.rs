pub mod pin;
pub mod argon2;
pub mod keyring;
pub mod face;

// Reexportaciones para permitir el acceso directo desde `crate::security::*`
pub use keyring::has_pin_setup;
pub use pin::{setup_new_pin, validate_pin};
pub use face::{delete_face_template, get_face_template, has_face_template, save_face_template};
