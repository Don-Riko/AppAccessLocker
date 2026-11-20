//! Plantilla facial (embedding 512-d) guardada en el keyring del usuario.
//! Nunca se almacenan imágenes: solo el vector numérico, en la misma bóveda que el PIN.

use keyring::Entry;

const SERVICE: &str = "app_access_locker_service";
const USER: &str = "face_template";

fn get_entry() -> Entry {
    Entry::new(SERVICE, USER).unwrap()
}

pub fn has_face_template() -> bool {
    get_entry().get_password().is_ok()
}

pub fn save_face_template(template: &[f32]) -> Result<(), String> {
    let data = serde_json::to_string(template).map_err(|e| e.to_string())?;
    get_entry().set_password(&data).map_err(|e| e.to_string())
}

pub fn get_face_template() -> Option<Vec<f32>> {
    let data = get_entry().get_password().ok()?;
    serde_json::from_str(&data).ok()
}

pub fn delete_face_template() -> Result<(), String> {
    match get_entry().delete_password() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}
