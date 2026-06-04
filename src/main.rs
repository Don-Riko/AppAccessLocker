mod gui;
mod security;
mod launcher;
mod monitor;
mod config;

use std::env;
use gtk4::prelude::*;
use gtk4::gio; // Importamos gio para acceder a las banderas de la app
use libadwaita::Application;

const APP_ID: &str = "com.tudominio.AppAccessLocker";

fn main() {
    let args: Vec<String> = env::args().collect();
    let target_app = if args.len() > 1 { args[1].clone() } else { String::new() };
    let app_args: Vec<String> = if args.len() > 2 { args[2..].to_vec() } else { vec![] };

    // 1. Añadimos la bandera NON_UNIQUE para permitir múltiples instancias
    let app = Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE) 
        .build();

    // 2. Configuramos el icono de forma segura durante el arranque (startup)
    app.connect_startup(|_| {
        gtk4::Window::set_default_icon_name(APP_ID);
    });

    app.connect_activate(move |app| {
        if !security::keyring::has_pin_setup() {
            gui::setup::show_setup_window(app);
        } else if target_app.is_empty() {
            println!("Modo de gestión de AppAccessLocker (Añadir UI aquí)");
        } else {
            gui::login::show_login_window(app, target_app.clone(), app_args.clone());
        }
    });

    app.run_with_args(&[""]);
}