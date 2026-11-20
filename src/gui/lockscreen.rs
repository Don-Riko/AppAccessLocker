use gtk4::prelude::*;
use libadwaita::Application;
use gtk4::{ApplicationWindow, Box, Button, Orientation, PasswordEntry, Label};
use std::rc::Rc;
use std::cell::RefCell;
use std::process::{Command, Child};

pub fn show_lockscreen(app: &Application, target_app: String, child_rc: Rc<RefCell<Child>>) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Bloqueo por Inactividad")
        .build();

    let vbox = Box::new(Orientation::Vertical, 12);
    vbox.set_valign(gtk4::Align::Center);
    vbox.set_halign(gtk4::Align::Center);

    let idle_secs = crate::monitor::activity::IDLE_LIMIT_MS / 1000;
    let label = Label::new(Some(&format!("Inactividad de {}s alcanzada.\nDesbloquear {}:", idle_secs, target_app)));
    let pin_entry = PasswordEntry::new();
    pin_entry.set_activates_default(true);

    let unlock_btn = Button::with_label("Desbloquear");
    window.set_default_widget(Some(&unlock_btn));

    let window_clone = window.clone();
    let value = pin_entry.clone();
    let app_clone = app.clone();
    let target_clone = target_app.clone();
    
    unlock_btn.connect_clicked(move |_| {
        let pin = value.text().to_string();
        
        if crate::security::validate_pin(&pin) {
            // 1. DESCONGELAMOS LA APP OBJETIVO
            let pid = child_rc.borrow().id();
            Command::new("kill").args(["-CONT", &pid.to_string()]).spawn().ok();
            
            // 2. Reiniciamos el monitor de inactividad
            crate::monitor::activity::start_idle_monitor(app_clone.clone(), target_clone.clone(), child_rc.clone());
            
            // 3. Cerramos el lockscreen
            window_clone.close();
        } else {
            value.set_text("");
            println!("PIN Incorrecto");
        }
    });

    vbox.append(&label);
    vbox.append(&pin_entry);
    vbox.append(&unlock_btn);
    
    window.set_child(Some(&vbox));
    window.set_fullscreened(true);
    // Quitamos los controles de la ventana para que no puedan cerrarla con la X
    window.set_decorated(false); 
    window.present();
}

/// Lockscreen del modo Autenticador: muestra el estado de la cámara/verificación
/// y ofrece el PIN como respaldo. El desbloqueo automático lo decide el monitor.
pub struct PresenceLockscreen {
    window: ApplicationWindow,
    status: Label,
}

impl PresenceLockscreen {
    pub fn show(app: &Application, target_app: &str, on_pin_unlock: Rc<dyn Fn()>) -> Self {
        let window = ApplicationWindow::builder()
            .application(app)
            .title("Usuario no detectado")
            .build();

        let vbox = Box::new(Orientation::Vertical, 12);
        vbox.set_valign(gtk4::Align::Center);
        vbox.set_halign(gtk4::Align::Center);

        let title = Label::new(Some(&format!(
            "No se detectó al usuario registrado.\n{} está en pausa.",
            target_app
        )));
        title.set_justify(gtk4::Justification::Center);

        let status = Label::new(Some("Buscando al usuario registrado…"));
        status.set_justify(gtk4::Justification::Center);
        status.add_css_class("dim-label");

        let camera_note = Label::new(Some("📷 Cámara activa para reconocimiento facial"));
        camera_note.add_css_class("dim-label");

        let pin_entry = PasswordEntry::new();
        pin_entry.set_placeholder_text(Some("PIN de respaldo"));
        pin_entry.set_activates_default(true);

        let unlock_btn = Button::with_label("Desbloquear con PIN");
        window.set_default_widget(Some(&unlock_btn));

        let value = pin_entry.clone();
        unlock_btn.connect_clicked(move |_| {
            let pin = value.text().to_string();
            if crate::security::validate_pin(&pin) {
                on_pin_unlock();
            } else {
                value.set_text("");
                println!("PIN Incorrecto");
            }
        });

        vbox.append(&title);
        vbox.append(&status);
        vbox.append(&pin_entry);
        vbox.append(&unlock_btn);
        vbox.append(&camera_note);

        window.set_child(Some(&vbox));
        window.set_fullscreened(true);
        window.set_decorated(false);
        window.present();

        Self { window, status }
    }

    pub fn set_status(&self, text: &str) {
        self.status.set_text(text);
    }

    pub fn close(&self) {
        self.window.close();
    }
}
