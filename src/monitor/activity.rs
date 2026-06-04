use std::process::{Command, Child};
use std::rc::Rc;
use std::cell::RefCell;
use libadwaita::Application;
use gtk4::prelude::*; // Necesario para app.hold()

/// Consulta a GNOME cuánto tiempo (en milisegundos) lleva el usuario sin tocar nada
fn get_idle_time_ms() -> u64 {
    let output = Command::new("busctl")
        .args([
            "--user", "call", 
            "org.gnome.Mutter.IdleMonitor", 
            "/org/gnome/Mutter/IdleMonitor/Core", 
            "org.gnome.Mutter.IdleMonitor", 
            "GetIdletime"
        ])
        .output();

    if let Ok(out) = output {
        let stdout = String::from_utf8_lossy(&out.stdout);
        // La respuesta suele ser "t 12345", capturamos el número final.
        if let Some(time_str) = stdout.split_whitespace().last() {
            return time_str.parse::<u64>().unwrap_or(0);
        }
    }
    0
}

/// Inicia un bucle en segundo plano que monitorea cada segundo
pub fn start_idle_monitor(app: Application, target_app: String, child_rc: Rc<RefCell<Child>>) {
    // `app.hold()` evita que la aplicación Rust se cierre cuando no hay ventanas abiertas
    let hold_guard = app.hold(); 

    gtk4::glib::timeout_add_seconds_local(1, move || {
        // Mantenemos la referencia viva
        let _guard = &hold_guard;

        // 1. Si el usuario cerró la app objetivo (ej. cerró Chrome), terminamos el monitor y Rust muere.
        if let Ok(Some(_)) = child_rc.borrow_mut().try_wait() {
            return gtk4::glib::ControlFlow::Break;
        }

        // 2. Revisamos si lleva 30 segundos de inactividad
        let idle_ms = get_idle_time_ms();
        if idle_ms >= 30_000 {
            let pid = child_rc.borrow().id();
            
            // ¡MAGIA LINUX! Congelamos la aplicación objetivo
            Command::new("kill").args(["-STOP", &pid.to_string()]).spawn().ok();
            
            // Invocamos el lockscreen para pedir el PIN
            crate::gui::lockscreen::show_lockscreen(&app, target_app.clone(), child_rc.clone());
            
            // Detenemos este temporizador temporalmente
            return gtk4::glib::ControlFlow::Break; 
        }

        gtk4::glib::ControlFlow::Continue
    });
}