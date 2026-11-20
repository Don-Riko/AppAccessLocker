//! Ventana de gestión: conmutador entre modo Clásico (temporizador) y modo
//! Autenticador (cámara), y registro/eliminación del rostro del usuario.

use crate::config::settings::{LockMode, Settings};
use gtk4::prelude::*;
use gtk4::{ApplicationWindow, Box, Button, CheckButton, Label, Orientation};
use libadwaita::Application;
use std::cell::RefCell;
use std::rc::Rc;

pub fn show_manage_window(app: &Application) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("AppAccessLocker — Configuración")
        .default_width(460)
        .default_height(360)
        .build();

    let vbox = Box::new(Orientation::Vertical, 12);
    vbox.set_margin_top(24);
    vbox.set_margin_bottom(24);
    vbox.set_margin_start(24);
    vbox.set_margin_end(24);

    let settings = Rc::new(RefCell::new(Settings::load()));

    // --- Modo de bloqueo
    let mode_title = Label::new(Some("Modo de bloqueo"));
    mode_title.add_css_class("title-4");
    mode_title.set_halign(gtk4::Align::Start);

    let classic_btn = CheckButton::with_label("Clásico: bloquear por inactividad (temporizador)");
    let auth_btn = CheckButton::with_label("Autenticador: bloquear cuando la cámara no vea al usuario registrado");
    auth_btn.set_group(Some(&classic_btn));
    match settings.borrow().lock_mode {
        LockMode::Classic => classic_btn.set_active(true),
        LockMode::Authenticator => auth_btn.set_active(true),
    }

    let s = settings.borrow();
    let params = Label::new(Some(&format!(
        "Autenticador: comprueba cada {} s y bloquea tras {} s sin reconocerte.\nAl reconocerte de nuevo, la app se reanuda automáticamente (con verificación de vida).\nLa cámara solo se usa en este modo; nunca se guardan imágenes.",
        s.presence_check_interval_secs, s.presence_lock_after_secs
    )));
    drop(s);
    params.add_css_class("dim-label");
    params.set_wrap(true);
    params.set_halign(gtk4::Align::Start);

    // --- Rostro registrado
    let face_title = Label::new(Some("Rostro registrado"));
    face_title.add_css_class("title-4");
    face_title.set_halign(gtk4::Align::Start);

    let face_status = Label::new(None);
    face_status.set_halign(gtk4::Align::Start);
    let enroll_btn = Button::with_label("Registrar rostro…");
    let delete_btn = Button::with_label("Eliminar rostro");
    delete_btn.add_css_class("destructive-action");

    let refresh_face = {
        let face_status = face_status.clone();
        let delete_btn = delete_btn.clone();
        let enroll_btn = enroll_btn.clone();
        Rc::new(move || {
            let has = crate::security::has_face_template();
            face_status.set_text(if has { "Estado: rostro registrado ✔" } else { "Estado: ningún rostro registrado" });
            delete_btn.set_sensitive(has);
            enroll_btn.set_label(if has { "Volver a registrar rostro…" } else { "Registrar rostro…" });
        })
    };
    refresh_face();

    let face_row = Box::new(Orientation::Horizontal, 8);
    face_row.append(&enroll_btn);
    face_row.append(&delete_btn);

    // --- Mensajes y guardar
    let message = Label::new(None);
    message.set_wrap(true);
    message.set_halign(gtk4::Align::Start);
    let save_btn = Button::with_label("Guardar");
    save_btn.add_css_class("suggested-action");

    {
        let app = app.clone();
        let refresh_face = refresh_face.clone();
        let message = message.clone();
        enroll_btn.connect_clicked(move |_| {
            if !Settings::models_dir().join("det_500m.onnx").exists() {
                message.set_text(&format!(
                    "Faltan los modelos de visión en {}.\nDescarga buffalo_sc (det_500m.onnx y w600k_mbf.onnx) en esa carpeta.",
                    Settings::models_dir().display()
                ));
                return;
            }
            let refresh_face = refresh_face.clone();
            let message = message.clone();
            crate::gui::enroll::show_enroll_window(
                &app,
                Rc::new(move |ok| {
                    refresh_face();
                    message.set_text(if ok { "Rostro registrado." } else { "Registro cancelado." });
                }),
            );
        });
    }

    {
        let refresh_face = refresh_face.clone();
        let message = message.clone();
        delete_btn.connect_clicked(move |_| {
            match crate::security::delete_face_template() {
                Ok(()) => message.set_text("Rostro eliminado."),
                Err(e) => message.set_text(&format!("No se pudo eliminar: {e}")),
            }
            refresh_face();
        });
    }

    {
        let settings = settings.clone();
        let auth_btn = auth_btn.clone();
        let message = message.clone();
        save_btn.connect_clicked(move |_| {
            let mode = if auth_btn.is_active() { LockMode::Authenticator } else { LockMode::Classic };
            if mode == LockMode::Authenticator {
                if !crate::security::has_face_template() {
                    message.set_text("Registra tu rostro antes de activar el modo Autenticador.");
                    return;
                }
                if !Settings::models_dir().join("w600k_mbf.onnx").exists() {
                    message.set_text(&format!("Faltan los modelos de visión en {}.", Settings::models_dir().display()));
                    return;
                }
            }
            let mut s = settings.borrow_mut();
            s.lock_mode = mode;
            match s.save() {
                Ok(()) => message.set_text(&format!(
                    "Configuración guardada. Modo activo: {}.",
                    match mode {
                        LockMode::Classic => "Clásico",
                        LockMode::Authenticator => "Autenticador",
                    }
                )),
                Err(e) => message.set_text(&format!("No se pudo guardar: {e}")),
            }
        });
    }

    vbox.append(&mode_title);
    vbox.append(&classic_btn);
    vbox.append(&auth_btn);
    vbox.append(&params);
    vbox.append(&face_title);
    vbox.append(&face_status);
    vbox.append(&face_row);
    vbox.append(&message);
    vbox.append(&save_btn);

    window.set_child(Some(&vbox));
    window.present();
}
