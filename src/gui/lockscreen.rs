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

    let label = Label::new(Some(&format!("Inactividad de 30s alcanzada.\nDesbloquear {}:", target_app)));
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
        
        if crate::security::pin::validate_pin(&pin) {
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