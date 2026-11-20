use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Modo de bloqueo de la aplicación protegida.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LockMode {
    /// Modo clásico: bloquea por inactividad (temporizador vía Mutter IdleMonitor).
    #[default]
    Classic,
    /// Modo autenticador: bloquea cuando la cámara deja de ver al usuario registrado.
    Authenticator,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(default)]
pub struct Settings {
    pub locked_apps: Vec<String>, // Nombres de ejecutables, ej: ["firefox", "telegram-desktop"]
    pub timeout_seconds: u32,

    /// Modo de bloqueo activo.
    pub lock_mode: LockMode,
    /// Dispositivo V4L2 de la cámara.
    pub camera_device: String,
    /// Cada cuántos segundos se comprueba la presencia del usuario (modo autenticador).
    pub presence_check_interval_secs: u32,
    /// Segundos sin reconocer al usuario antes de bloquear (modo autenticador).
    pub presence_lock_after_secs: u32,
    /// Umbral de similitud coseno para aceptar un rostro como el registrado (0..1).
    pub face_match_threshold: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            locked_apps: Vec::new(),
            timeout_seconds: 30,
            lock_mode: LockMode::Classic,
            camera_device: "/dev/video0".to_string(),
            presence_check_interval_secs: 5,
            presence_lock_after_secs: 10,
            face_match_threshold: 0.40,
        }
    }
}

impl Settings {
    fn project_dirs() -> ProjectDirs {
        ProjectDirs::from("com", "tudominio", "AppAccessLocker")
            .expect("No se encontró el directorio de configuración")
    }

    fn config_path() -> PathBuf {
        let proj_dirs = Self::project_dirs();
        let dir = proj_dirs.config_dir();
        fs::create_dir_all(dir).ok();
        dir.join("settings.json")
    }

    /// Directorio donde se buscan los modelos ONNX (det_500m.onnx, w600k_mbf.onnx).
    pub fn models_dir() -> PathBuf {
        Self::project_dirs().data_dir().join("models")
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
