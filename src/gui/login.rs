use gtk4::prelude::*;
use libadwaita::Application;
use gtk4::{ApplicationWindow, Box, Button, Orientation, PasswordEntry, Label};
use crate::config::settings::{LockMode, Settings};

use std::rc::Rc;
use std::cell::RefCell;

pub fn show_login_window(app: &Application, target_app: String, app_args: Vec<String>) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("App Protegida")
        .default_width(350)
        .default_height(200)
        .build();

    let vbox = Box::new(Orientation::Vertical, 12);
    vbox.set_margin_top(24);
    vbox.set_margin_bottom(24);
    vbox.set_margin_start(24);
    vbox.set_margin_end(24);

    let label = Label::new(Some(&format!("Desbloquear el acceso a: {}", target_app)));
    
    let pin_entry = PasswordEntry::new();
    pin_entry.set_activates_default(true);

    let unlock_btn = Button::with_label("Desbloquear");
    window.set_default_widget(Some(&unlock_btn));

    // --- SOLUCIÓN: Clonar TODO lo necesario ANTES del closure ---
    let window_clone = window.clone();
    let value = pin_entry.clone();
    let app_clone = app.clone(); // <- ¡AQUÍ ESTÁ LA CLAVE! 
    
    unlock_btn.connect_clicked(move |_| {
        let pin = value.text().to_string();
        
        if crate::security::validate_pin(&pin) {
            match std::process::Command::new(&target_app).args(&app_args).spawn() {
                Ok(child) => {
                    let child_rc = Rc::new(RefCell::new(child));
                    
                    // Elegimos el monitor según el modo configurado.
                    let settings = Settings::load();
                    let face_template = match settings.lock_mode {
                        LockMode::Authenticator => crate::security::get_face_template(),
                        LockMode::Classic => None,
                    };
                    match (settings.lock_mode, face_template) {
                        (LockMode::Authenticator, Some(template)) => {
                            crate::monitor::presence::start_presence_monitor(
                                app_clone.clone(), target_app.clone(), child_rc, settings, template,
                            );
                        }
                        (LockMode::Authenticator, None) => {
                            eprintln!("Modo Autenticador sin rostro registrado: se usa el modo Clásico.");
                            crate::monitor::activity::start_idle_monitor(app_clone.clone(), target_app.clone(), child_rc);
                        }
                        (LockMode::Classic, _) => {
                            crate::monitor::activity::start_idle_monitor(app_clone.clone(), target_app.clone(), child_rc);
                        }
                    }

                    window_clone.close();
                },
                Err(e) => {
                    eprintln!("Error al lanzar {}: {}", target_app, e);
                }
            }
        } else {
            value.set_text("");
            println!("PIN Incorrecto");
        }
    });

    vbox.append(&label);
    vbox.append(&pin_entry);
    vbox.append(&unlock_btn);
    window.set_child(Some(&vbox));
    window.present();
}