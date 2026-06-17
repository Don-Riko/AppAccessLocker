use gtk4::prelude::*;
use libadwaita::Application;
use gtk4::{ApplicationWindow, Box, Button, Orientation, PasswordEntry};

pub fn show_setup_window(app: &Application) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Configurar PIN")
        .default_width(350)
        .default_height(200)
        .build();

    let vbox = Box::new(Orientation::Vertical, 12);
    vbox.set_margin_top(24);
    vbox.set_margin_bottom(24);
    vbox.set_margin_start(24);
    vbox.set_margin_end(24);

    let pin_entry = PasswordEntry::new();
    pin_entry.set_placeholder_text(Some("Ingresa un nuevo PIN"));

    let save_btn = Button::with_label("Guardar PIN");
    
    let window_clone = window.clone();
    let value = pin_entry.clone();
    save_btn.connect_clicked(move |_| {
        let pin = value.text().to_string();
        if !pin.is_empty() {
            crate::security::setup_new_pin(&pin).unwrap();
            println!("PIN guardado. Reinicia la aplicación.");
            window_clone.close();
        }
    });

    vbox.append(&pin_entry);
    vbox.append(&save_btn);
    window.set_child(Some(&vbox));
    window.present();
}