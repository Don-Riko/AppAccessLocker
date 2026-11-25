//! Detecta otras aplicaciones que intentan usar la cámara mientras AAL la tiene.
//!
//! V4L2 permite abrir el dispositivo varias veces aunque solo un proceso pueda
//! hacer streaming; por eso, quien intenta usarla queda con un descriptor abierto
//! a `/dev/videoN`. Se localiza recorriendo `/proc/*/fd` (solo procesos del mismo
//! usuario son legibles). Los eventos `IN_OPEN` de inotify avisan de cada intento.
//!
//! Las apps que usan el portal de cámara (GNOME Cámara/Snapshot, navegadores en
//! Wayland…) no abren el dispositivo: crean un flujo `Stream/Input/Video` en
//! PipeWire y es el demonio `pipewire` quien lo abre. Esas se detectan con
//! `pw-dump`.

use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;

/// Procesos que no cuentan como "otra app": servidores multimedia que solo sondean
/// el dispositivo y otras instancias de AAL.
const IGNORED: &[&str] = &[
    "pipewire", "wireplumber", "pipewire-pulse", "app-access-loc", "app-access-locker",
];

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CameraUser {
    pub pid: u32,
    pub name: String,
    /// Está transmitiendo ahora (flujo `running` o dispositivo abierto), no solo conectado.
    pub active: bool,
}

/// Lista las apps (distintas de AAL) que usan o intentan usar la cámara, ya sea
/// abriendo `device` directamente o mediante un flujo de cámara en PipeWire.
pub fn other_users(device: &str) -> Vec<CameraUser> {
    let mut users = direct_users(device);
    for u in pipewire_users() {
        match users.iter_mut().find(|d| d.pid == u.pid) {
            Some(d) => d.active |= u.active,
            None => users.push(u),
        }
    }
    users
}

/// Procesos con el dispositivo V4L2 abierto.
fn direct_users(device: &str) -> Vec<CameraUser> {
    let own = std::process::id();
    let mut users = Vec::new();
    let Ok(proc_dir) = fs::read_dir("/proc") else { return users };
    for entry in proc_dir.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        if pid == own {
            continue;
        }
        let Ok(fds) = fs::read_dir(entry.path().join("fd")) else { continue };
        let holds = fds
            .flatten()
            .any(|fd| fs::read_link(fd.path()).map_or(false, |t| t == Path::new(device)));
        if !holds {
            continue;
        }
        let name = process_name(pid);
        if IGNORED.contains(&name.as_str()) {
            continue;
        }
        users.push(CameraUser { pid, name, active: true });
    }
    users
}

fn process_name(pid: u32) -> String {
    // El nombre del ejecutable es más fiable que `comm` (truncado a 15 caracteres).
    if let Ok(exe) = fs::read_link(format!("/proc/{pid}/exe")) {
        if let Some(n) = exe.file_name().and_then(|n| n.to_str()) {
            return n.to_string();
        }
    }
    fs::read_to_string(format!("/proc/{pid}/comm"))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| format!("pid {pid}"))
}

fn prop<'a>(obj: &'a Value, key: &str) -> Option<&'a Value> {
    obj.get("info")?.get("props")?.get(key)
}

fn prop_str<'a>(obj: &'a Value, key: &str) -> Option<&'a str> {
    prop(obj, key)?.as_str()
}

fn prop_u64(obj: &Value, key: &str) -> Option<u64> {
    let v = prop(obj, key)?;
    v.as_u64().or_else(|| v.as_str()?.parse().ok())
}

/// Clientes de PipeWire con un flujo de vídeo de entrada dirigido a la cámara.
fn pipewire_users() -> Vec<CameraUser> {
    let Ok(out) = Command::new("pw-dump").arg("--no-colors").output() else { return Vec::new() };
    let Ok(Value::Array(objects)) = serde_json::from_slice::<Value>(&out.stdout) else { return Vec::new() };
    let kind = |o: &Value| o.get("type").and_then(Value::as_str).unwrap_or("").rsplit(':').next().unwrap_or("").to_string();
    let id = |o: &Value| o.get("id").and_then(Value::as_u64);

    let clients: HashMap<u64, &Value> =
        objects.iter().filter(|o| kind(o) == "Client").filter_map(|o| Some((id(o)?, o))).collect();
    // Nodos que son una cámara física (fuente V4L2 / libcamera).
    let camera_nodes: Vec<u64> = objects
        .iter()
        .filter(|o| kind(o) == "Node")
        .filter(|o| prop_str(o, "media.class") == Some("Video/Source"))
        .filter(|o| {
            prop_str(o, "media.role") == Some("Camera")
                || prop_str(o, "node.name").map_or(false, |n| n.starts_with("v4l2_") || n.starts_with("libcamera"))
        })
        .filter_map(id)
        .collect();
    let linked_to_camera = |node: u64| {
        objects.iter().filter(|o| kind(o) == "Link").any(|l| {
            let src = l.get("info").and_then(|i| i.get("output-node-id")).and_then(Value::as_u64);
            let dst = l.get("info").and_then(|i| i.get("input-node-id")).and_then(Value::as_u64);
            dst == Some(node) && src.map_or(false, |s| camera_nodes.contains(&s))
        })
    };

    let own = std::process::id() as u64;
    let mut users = Vec::new();
    for node in objects.iter().filter(|o| kind(o) == "Node") {
        if prop_str(node, "media.class") != Some("Stream/Input/Video") {
            continue;
        }
        let Some(node_id) = id(node) else { continue };
        let Some(client) = prop_u64(node, "client.id").and_then(|c| clients.get(&c)) else { continue };
        // Solo flujos de cámara: concedidos por el portal de cámara o enlazados a una.
        let camera_role = prop_str(client, "pipewire.access.portal.media_roles").map_or(false, |r| r.contains("Camera"));
        if !camera_role && !linked_to_camera(node_id) {
            continue;
        }
        let Some(pid) = prop_u64(client, "application.process.id") else { continue };
        if pid == own {
            continue;
        }
        let binary = prop_str(client, "application.process.binary").unwrap_or("");
        if IGNORED.contains(&binary) {
            continue;
        }
        let name = prop_str(client, "application.name")
            .filter(|n| !n.is_empty())
            .unwrap_or(if binary.is_empty() { "otra aplicación" } else { binary })
            .to_string();
        let active = node.get("info").and_then(|i| i.get("state")).and_then(Value::as_str) == Some("running");
        match users.iter_mut().find(|u: &&mut CameraUser| u.pid == pid as u32) {
            Some(u) => u.active |= active,
            None => users.push(CameraUser { pid: pid as u32, name, active }),
        }
    }
    users
}
