use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct Settings {
    pub locked_apps: Vec<String>, // Nombres de ejecutables, ej: ["firefox", "telegram-desktop"]
    pub timeout_seconds: u32,
}

impl Settings {
    fn config_path() -> PathBuf {
        let proj_dirs = ProjectDirs::from("com", "tudominio", "AppAccessLocker")
            .expect("No se encontró el directorio de configuración");
        let dir = proj_dirs.config_dir();
        fs::create_dir_all(dir).ok();
        dir.join("settings.json")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if path.exists() {
            let data = fs::read_to_string(path).unwrap_or_default();
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            Self::default()
        }
    }

    pub fn save(&self) -> Result<(), std::io::Error> {
        let data = serde_json::to_string_pretty(self)?;
        fs::write(Self::config_path(), data)
    }
}